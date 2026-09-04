//! A window presented through wgpu, when the `gpu` feature is on.
//!
//! The software path hands the application a `Frame` of words and copies the
//! damaged rows to the compositor. This path hands it a [`Pen`] over a
//! [`denise_wgpu::GpuPainter`] and presents a swapchain texture. Nothing about
//! the window, the input or the scheduling changes; only what draws.
//!
//! # Damage
//!
//! A swapchain image rotates and its age cannot be trusted, so nothing
//! incremental can be built on it directly. This surface therefore keeps a
//! texture of its own, draws the damage onto that with the rest left exactly as
//! the last frame left it, and copies the result to the swapchain. The copy is
//! whole-texture and costs a blit; the drawing — which is what actually costs —
//! is only the damage.
//!
//! That makes the age honest: [`BufferAge::Frames(1)`](BufferAge::Frames), the
//! same as a persistent shadow buffer, because the target holds exactly the
//! previous frame. It is [`BufferAge::Undefined`](BufferAge::Undefined) for one
//! frame after the target is made or resized, which is the one frame that must
//! repaint everything.

use std::sync::Arc;

use denise::{BufferAge, InputEvent, InputSource, Pen, Rect, Size};
use denise_wgpu::Gpu;
use winit::window::Window;

use crate::Error;

/// A winit window drawn into through `denise-wgpu`.
///
/// The counterpart of the platform's CPU surface for
/// [`Present::Gpu`](crate::Present::Gpu). It does not implement
/// [`denise::Surface`] — there is no frame of words to hand out — so an
/// application reaches it only through [`DeniseApp::paint`](crate::DeniseApp::paint).
pub struct GpuSurface {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    gpu: Gpu,
    size: Size,
    scale_factor: f32,
    events: Vec<InputEvent>,
    /// The target kept between frames, so a frame can draw only its damage.
    /// `None` until the first paint and after every resize.
    canvas: Option<Canvas>,
}

/// The persistent target and the view onto it.
struct Canvas {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    /// Set until something has been drawn into it. An undefined texture must be
    /// painted whole before any of it can be kept.
    fresh: bool,
}

impl GpuSurface {
    /// Opens a swapchain on `window` and builds the painter for it.
    ///
    /// Fails with [`Error::Gpu`] when wgpu finds no adapter that can present to
    /// this window, and with [`Error::Present`] when the window cannot be
    /// turned into a surface at all.
    pub fn new(window: Arc<Window>) -> Result<Self, Error> {
        let instance = wgpu::Instance::default();
        let surface = instance
            .create_surface(window.clone())
            .map_err(|e| Error::Present(e.to_string()))?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: Some(&surface),
            ..Default::default()
        }))
        .map_err(|_| Error::Gpu("no adapter can present to this window".into()))?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("denise window"),
            ..Default::default()
        }))
        .map_err(|e| Error::Gpu(e.to_string()))?;

        let inner = window.inner_size();
        let size = Size::new(inner.width, inner.height);
        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .ok_or_else(|| Error::Gpu("the adapter cannot present to this window".into()))?;
        // A non-sRGB format where one exists: Denise's colours are bytes meant
        // for the screen, and an sRGB target would convert them on the way in,
        // which the software path never does.
        let caps = surface.get_capabilities(&adapter);
        if let Some(format) = caps.formats.iter().copied().find(|f| !f.is_srgb()) {
            config.format = format;
        }
        config.present_mode = wgpu::PresentMode::AutoVsync;
        // The swapchain is copied into rather than drawn into, so it needs to
        // be a copy destination as well as an attachment.
        config.usage |= wgpu::TextureUsages::COPY_DST;
        surface.configure(&device, &config);

        let gpu = Gpu::new(device, queue, config.format);
        Ok(Self {
            scale_factor: window.scale_factor() as f32,
            window,
            surface,
            config,
            gpu,
            size,
            events: Vec::new(),
            canvas: None,
        })
    }

    /// The window this presents to.
    pub fn window(&self) -> &Arc<Window> {
        &self.window
    }

    /// Visible extent in physical pixels.
    pub fn size(&self) -> Size {
        self.size
    }

    /// Physical pixels per logical pixel.
    pub fn scale_factor(&self) -> f32 {
        self.scale_factor
    }

    /// Follows the window: reconfigures the swapchain to `size`.
    pub fn resize(&mut self, size: Size, scale_factor: f32) {
        if size == self.size && scale_factor == self.scale_factor {
            return;
        }
        self.size = size;
        self.scale_factor = scale_factor;
        if size.is_empty() {
            return;
        }
        self.config.width = size.width;
        self.config.height = size.height;
        self.surface.configure(self.gpu.device(), &self.config);
        // A target of the old size holds nothing useful for the new one.
        self.canvas = None;
    }

    pub(crate) fn push_event(&mut self, event: InputEvent) {
        self.events.push(event);
    }

    /// Draws one frame through `draw` and presents it.
    ///
    /// Returns `Ok(false)` when the swapchain had nothing to give — the window
    /// is occluded, or the surface was just rebuilt — which is a frame to skip,
    /// not an error. The next redraw gets a fresh texture.
    pub fn paint(
        &mut self,
        damage: &[Rect],
        draw: impl FnOnce(&mut Pen<'_>),
    ) -> Result<bool, Error> {
        use wgpu::CurrentSurfaceTexture as Current;

        if self.size.is_empty() {
            return Ok(false);
        }
        let frame = match self.surface.get_current_texture() {
            Current::Success(frame) | Current::Suboptimal(frame) => frame,
            Current::Lost | Current::Outdated => {
                self.surface.configure(self.gpu.device(), &self.config);
                return Ok(false);
            }
            Current::Timeout | Current::Occluded => return Ok(false),
            Current::Validation => {
                return Err(Error::Gpu("the swapchain failed validation".into()));
            }
        };

        let canvas = self.canvas.get_or_insert_with(|| {
            let texture = self.gpu.device().create_texture(&wgpu::TextureDescriptor {
                label: Some("denise canvas"),
                size: wgpu::Extent3d {
                    width: self.size.width,
                    height: self.size.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.config.format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            Canvas {
                texture,
                view,
                fresh: true,
            }
        });

        let mut painter = self.gpu.painter(self.size);
        draw(&mut Pen::new(&mut painter));
        if canvas.fresh {
            // Nothing to keep yet, and the caller was told `Undefined` so it
            // has drawn the lot: clear and take all of it.
            painter.finish(&canvas.view);
            canvas.fresh = false;
        } else {
            painter.finish_onto(&canvas.view, damage);
        }

        // Whole-texture, because the swapchain image rotates and only the
        // target we keep is known to be the previous frame. A blit of the
        // window is cheap next to rasterising it.
        let mut encoder =
            self.gpu
                .device()
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("denise present"),
                });
        encoder.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &canvas.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyTextureInfo {
                texture: &frame.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::Extent3d {
                width: self.size.width,
                height: self.size.height,
                depth_or_array_layers: 1,
            },
        );
        self.gpu.queue().submit([encoder.finish()]);

        self.window.pre_present_notify();
        self.gpu.queue().present(frame);
        Ok(true)
    }

    /// What an application is told about the target it is about to draw into.
    ///
    /// [`Frames(1)`](BufferAge::Frames) once there is a target holding the
    /// previous frame, which is what lets an application repaint only what
    /// changed; [`Undefined`](BufferAge::Undefined) on the first frame and
    /// after a resize, when everything must be drawn.
    pub fn age(&self) -> BufferAge {
        match &self.canvas {
            Some(canvas) if !canvas.fresh => BufferAge::Frames(1),
            _ => BufferAge::Undefined,
        }
    }
}

impl InputSource for GpuSurface {
    fn poll(&mut self, out: &mut Vec<InputEvent>) {
        out.append(&mut self.events);
    }
}
