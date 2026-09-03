//! A window presented through wgpu, when the `gpu` feature is on.
//!
//! The software path hands the application a `Frame` of words and copies the
//! damaged rows to the compositor. This path hands it a [`Pen`] over a
//! [`denise_wgpu::GpuPainter`] and presents a swapchain texture. Nothing about
//! the window, the input or the scheduling changes; only what draws.
//!
//! Every frame is a full repaint. A swapchain gives no reliable buffer age, and
//! on a desktop GPU redrawing a window is nothing — this is the one place the
//! damage tracker's work is not needed, and [`BufferAge::Undefined`] is what
//! says so to the application.

use std::sync::Arc;

use denise::{BufferAge, InputEvent, InputSource, Pen, Size};
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
    }

    pub(crate) fn push_event(&mut self, event: InputEvent) {
        self.events.push(event);
    }

    /// Draws one frame through `draw` and presents it.
    ///
    /// Returns `Ok(false)` when the swapchain had nothing to give — the window
    /// is occluded, or the surface was just rebuilt — which is a frame to skip,
    /// not an error. The next redraw gets a fresh texture.
    pub fn paint(&mut self, draw: impl FnOnce(&mut Pen<'_>)) -> Result<bool, Error> {
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
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut painter = self.gpu.painter(self.size);
        draw(&mut Pen::new(&mut painter));
        painter.finish(&view);

        self.window.pre_present_notify();
        self.gpu.queue().present(frame);
        Ok(true)
    }

    /// What an application is told about the buffer it is about to draw: every
    /// frame here starts from nothing.
    pub const fn age(&self) -> BufferAge {
        BufferAge::Undefined
    }
}

impl InputSource for GpuSurface {
    fn poll(&mut self, out: &mut Vec<InputEvent>) {
        out.append(&mut self.events);
    }
}
