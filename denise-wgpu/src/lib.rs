//! A GPU painter for Denise, on wgpu.
//!
//! The second implementation of [`Painter`], and the reason the trait exists:
//! `denise-render` turns the same calls into pixels on a CPU, this crate turns
//! them into instanced triangles on whatever wgpu can find. Widgets cannot tell
//! which one they are drawing through, and that is the whole point.
//!
//! **This is for the desktop.** A kiosk on a Pi draws with the software
//! rasteriser and always will: it needs no Mesa, no compositor and no window
//! system, and at the sizes a panel runs it was never the bottleneck. A GPU
//! earns its keep where the designer runs — a Retina display, a large window, a
//! canvas that zooms — and that is the workload this crate is shaped for.
//!
//! # How a frame is drawn
//!
//! Every call on the painter appends vertices. Rectangles are plain triangles;
//! everything with a curve — rounded corners, circles, arcs, lines — is a
//! bounding quad whose fragment shader evaluates a signed distance and turns it
//! into one pixel of anti-aliasing. A polygon has more edges than a vertex can
//! carry, so its quad carries a range of a buffer that holds them and its
//! fragment shader walks that range for the nearest edge and for which side of
//! the outline it is on. Images and glyph masks are the same quads with a
//! texture bound. The clip is carried per vertex and applied per fragment, so a
//! frame is one pipeline and as few draws as the textures force: a whole widget
//! tree with no images is a single draw call.
//!
//! [`GpuPainter::finish`] then encodes the frame into a texture view — a
//! swapchain's, or an offscreen one — clearing it first.
//! [`GpuPainter::finish_onto`] is the incremental form: it keeps what is
//! already on the target and scissors to the damage, for a caller that owns its
//! target between frames. [`GpuPainter::finish_to_pixels`]
//! renders offscreen and reads the result back as `0xAARRGGBB` words, which is
//! how the parity tests compare it to the software rasteriser.
//!
//! ```no_run
//! use denise::{BufferAge, Pen, Size};
//! use denise_wgpu::Gpu;
//!
//! # fn paint(ui: &mut denise_ui::Ui<()>) -> Result<(), denise_wgpu::Error> {
//! let gpu = Gpu::headless()?;
//! let mut painter = gpu.painter(Size::new(640, 400));
//! ui.paint_with(&mut Pen::new(&mut painter), BufferAge::Undefined);
//! let pixels: Vec<u32> = painter.finish_to_pixels()?;
//! # Ok(())
//! # }
//! ```
//!
//! # Glyphs
//!
//! Text arrives through [`blit_glyph`](Painter::blit_glyph) as a rectangle of
//! an atlas page with an id and a version. The page is uploaded once per
//! version — that is, whenever the text engine packs a glyph it has not seen —
//! and every glyph after that is six vertices sampling it. A label costs what a
//! rectangle costs.
//!
//! # Images
//!
//! Pictures arrive through [`blit_image`](Painter::blit_image) with an id and a
//! version, the same way a glyph page does, and are cached the same way: one
//! upload when the pixels change, a quad every time after. A photo in a
//! carousel costs what a rectangle costs.
//!
//! # What it does not do yet
//!
//! The raw [`blit`](Painter::blit) family — a `PixelView` with no identity —
//! still uploads per call. Nothing in the widget set uses it any more; it is
//! there for a caller with pixels that genuinely are different every frame,
//! which is what an upload per call is the honest price of.

#![forbid(unsafe_code)]

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ops::Range;

use denise::angle::{ONE, TURN};
use denise::painter::ClipToken;
use denise::{
    AtlasPage, Color, ImageRef, Mask, Paint, Painter, PixelFormat, PixelView, Point, Rect, Size,
};
pub use wgpu;

use wgpu::util::DeviceExt as _;

/// What can go wrong between asking for a GPU and reading pixels back.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// wgpu found no adapter at all. Headless CI runners without a software
    /// Vulkan are the usual reason.
    #[error("no GPU adapter is available")]
    NoAdapter,
    /// The adapter refused to hand out a device.
    #[error("requesting a device")]
    Device(#[from] wgpu::RequestDeviceError),
    /// Mapping the readback buffer failed.
    #[error("mapping the readback buffer")]
    Map(#[from] wgpu::BufferAsyncError),
    /// The device did not finish the work it was asked to wait for.
    #[error("waiting for the GPU")]
    Poll(#[from] wgpu::PollError),
    /// The readback buffer could not be read once mapped.
    #[error("reading the readback buffer")]
    Read(#[from] wgpu::MapRangeError),
}

/// One vertex. Eighty-eight bytes, all of them `f32` or `u32`, so it is `Pod`.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    pos: [f32; 2],
    clip: [f32; 4],
    color: [f32; 4],
    a: [f32; 4],
    b: [f32; 4],
    kind: u32,
    /// Where this polygon's edges start in the frame's edge buffer, and how
    /// many there are. Zero for every other kind, and it costs nothing: the
    /// vertex was padded to a multiple of sixteen bytes anyway.
    poly: [u32; 2],
    _pad: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct Globals {
    size: [f32; 2],
    srgb: u32,
    _pad: u32,
}

const KIND_SOLID: u32 = 0;
const KIND_ROUNDED_FILL: u32 = 1;
const KIND_ROUNDED_STROKE: u32 = 2;
const KIND_CIRCLE_FILL: u32 = 3;
const KIND_CIRCLE_STROKE: u32 = 4;
const KIND_ARC: u32 = 5;
const KIND_LINE: u32 = 6;
const KIND_TEXTURED: u32 = 7;
const KIND_MASK: u32 = 8;
const KIND_TEXTURED_ROUNDED: u32 = 9;
const KIND_POLYGON: u32 = 10;

/// The UV rectangle that samples a whole texture.
const WHOLE: [f32; 4] = [0.0, 0.0, 1.0, 1.0];

/// A device, a queue, and the one pipeline every frame is drawn with.
///
/// Built once per device and kept; [`Gpu::painter`] hands out a painter per
/// frame.
pub struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    format: wgpu::TextureFormat,
    pipeline: wgpu::RenderPipeline,
    globals_layout: wgpu::BindGroupLayout,
    texture_layout: wgpu::BindGroupLayout,
    edges_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    /// A one-pixel white texture bound while drawing shapes, so the pipeline
    /// never has to change.
    white: wgpu::BindGroup,
    /// One empty edge, bound by every frame that draws no polygon. The
    /// pipeline layout says the group exists, so something must fill it, and
    /// most frames draw no polygon at all: this keeps them from allocating a
    /// buffer to say so.
    no_edges: wgpu::BindGroup,
    /// Glyph atlas pages, by atlas id: the version uploaded and its texture.
    ///
    /// One entry per atlas, replaced when its version moves on. Interior
    /// mutability because painters borrow the `Gpu` shared, and a cache that
    /// only a `&mut Gpu` could fill would never fill.
    pages: RefCell<HashMap<u64, (u64, wgpu::BindGroup)>>,
    /// How many atlas pages have been uploaded, ever. The number a profile
    /// wants, and the number the tests hold to "once per version".
    page_uploads: Cell<u64>,
    /// Images, by image id: the version uploaded and its texture. The same
    /// arrangement as `pages`, for the same reason.
    images: RefCell<HashMap<u64, (u64, wgpu::BindGroup)>>,
    /// How many images have been uploaded, ever.
    image_uploads: Cell<u64>,
    /// The globals buffer and its bind group, with the size they describe.
    /// They change only when the target does, so they are built on a resize
    /// rather than on a frame.
    globals: RefCell<Option<(Size, wgpu::Buffer, wgpu::BindGroup)>>,
}

impl Gpu {
    /// Wraps a device the caller already has, drawing into textures of `format`.
    ///
    /// `format` is what [`GpuPainter::finish`] will be handed views of — a
    /// swapchain's, typically. Prefer a non-sRGB format: Denise's colours are
    /// bytes meant for the screen, and an sRGB target forces a conversion that
    /// the software rasteriser never does.
    pub fn new(device: wgpu::Device, queue: wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("denise shapes"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });

        let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("denise globals"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("denise texture"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let edges_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("denise polygon edges"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("denise"),
            bind_group_layouts: &[
                Some(&globals_layout),
                Some(&texture_layout),
                Some(&edges_layout),
            ],
            ..Default::default()
        });

        let vertex_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &wgpu::vertex_attr_array![
                0 => Float32x2,
                1 => Float32x4,
                2 => Float32x4,
                3 => Float32x4,
                4 => Float32x4,
                5 => Uint32,
                6 => Uint32x2,
            ],
        };

        // Premultiplied source-over, the one blend mode the software rasteriser
        // has, so a translucent fill composites the same way on both.
        let blend = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
        };

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("denise"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[Some(vertex_layout)],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(blend),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        // Nearest, because the software blitter is nearest: a scaled image looks
        // the same through either painter.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("denise nearest"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        let white = upload(
            &device,
            &queue,
            &texture_layout,
            &sampler,
            1,
            1,
            wgpu::TextureFormat::Rgba8Unorm,
            &[255, 255, 255, 255],
        );

        let no_edges = {
            let empty = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("denise polygon edges (none)"),
                contents: &[0u8; std::mem::size_of::<[f32; 4]>()],
                usage: wgpu::BufferUsages::STORAGE,
            });
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("denise polygon edges (none)"),
                layout: &edges_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: empty.as_entire_binding(),
                }],
            })
        };

        Self {
            device,
            queue,
            format,
            pipeline,
            globals_layout,
            texture_layout,
            edges_layout,
            sampler,
            white,
            no_edges,
            pages: RefCell::new(HashMap::new()),
            page_uploads: Cell::new(0),
            images: RefCell::new(HashMap::new()),
            image_uploads: Cell::new(0),
            globals: RefCell::new(None),
        }
    }

    /// Any adapter wgpu can find, drawing into `Rgba8Unorm`.
    ///
    /// For tests, tools and `--snapshot` paths: no window, no surface. Fails
    /// with [`Error::NoAdapter`] where there is nothing to draw with, which a
    /// test should treat as "skip", not "fail".
    pub fn headless() -> Result<Self, Error> {
        let instance = wgpu::Instance::default();
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::None,
            force_fallback_adapter: false,
            compatible_surface: None,
            ..Default::default()
        }))
        .map_err(|_| Error::NoAdapter)?;
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("denise headless"),
                ..Default::default()
            }))?;
        Ok(Self::new(device, queue, wgpu::TextureFormat::Rgba8Unorm))
    }

    /// The device frames are drawn with.
    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    /// The queue frames are submitted to.
    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    /// The globals buffer and bind group for a target of `size`, built only if
    /// the last pair was for a different size.
    ///
    /// Both are reference-counted handles, so the clones are pointer bumps and
    /// a render pass can hold them without keeping the cache borrowed.
    fn globals_for(&self, size: Size) -> wgpu::BindGroup {
        if let Some((cached, _, group)) = self.globals.borrow().as_ref()
            && *cached == size
        {
            return group.clone();
        }
        let buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("denise globals"),
                contents: bytemuck::bytes_of(&Globals {
                    size: [size.width as f32, size.height as f32],
                    srgb: u32::from(self.format.is_srgb()),
                    _pad: 0,
                }),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("denise globals"),
            layout: &self.globals_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            }],
        });
        *self.globals.borrow_mut() = Some((size, buffer, group.clone()));
        group
    }

    /// Reads a texture back as `0xAARRGGBB` words, row after row with no
    /// padding — the layout a [`denise::Frame`] uses.
    ///
    /// The texture must carry [`COPY_SRC`](wgpu::TextureUsages::COPY_SRC) and
    /// this device's [`format`](Gpu::format). Blocks until the GPU is done; for
    /// tests, snapshots and tools.
    pub fn read_texture(&self, texture: &wgpu::Texture) -> Result<Vec<u32>, Error> {
        let (width, height) = (texture.width().max(1), texture.height().max(1));
        // Buffer rows must be 256-byte aligned for a texture-to-buffer copy.
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let unpadded = width * 4;
        let padded = unpadded.div_ceil(align) * align;
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("denise readback"),
            size: u64::from(padded) * u64::from(height),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("denise readback"),
            });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([encoder.finish()]);

        let slice = readback.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        self.device.poll(wgpu::PollType::wait_indefinitely())?;
        rx.recv().map_err(|_| Error::NoAdapter)??;

        let bgra = matches!(
            self.format,
            wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
        );
        let data = slice.get_mapped_range()?;
        let mut pixels = Vec::with_capacity((width * height) as usize);
        for row in data.chunks_exact(padded as usize) {
            for &[c0, c1, c2, c3] in row[..unpadded as usize].as_chunks::<4>().0 {
                let (r, g, b, a) = if bgra {
                    (c2, c1, c0, c3)
                } else {
                    (c0, c1, c2, c3)
                };
                pixels.push(u32::from_be_bytes([a, r, g, b]));
            }
        }
        drop(data);
        readback.unmap();
        Ok(pixels)
    }

    /// How many glyph atlas pages this device has uploaded, in total.
    ///
    /// A page is uploaded once per [`AtlasPage::version`], so for a text engine
    /// whose glyphs have all been seen this stops moving; a number that keeps
    /// climbing means an atlas too small for its working set, which the
    /// engine's own `resets` will confirm.
    pub fn page_uploads(&self) -> u64 {
        self.page_uploads.get()
    }

    /// How many images this device has uploaded, in total.
    ///
    /// An image is uploaded once per [`ImageRef::version`]: a picture that has
    /// been drawn before costs a quad, and only replacing its pixels costs an
    /// upload.
    pub fn image_uploads(&self) -> u64 {
        self.image_uploads.get()
    }

    /// The texture format [`GpuPainter::finish`] expects its target to have.
    pub fn format(&self) -> wgpu::TextureFormat {
        self.format
    }

    /// A painter for one frame of `size` pixels.
    pub fn painter(&self, size: Size) -> GpuPainter<'_> {
        GpuPainter {
            gpu: self,
            size,
            clip: Rect::from_size(size),
            vertices: Vec::with_capacity(4096),
            draws: Vec::new(),
            textures: Vec::new(),
            edges: Vec::new(),
        }
    }

    /// The texture holding `page`, uploaded now if this version has not been.
    fn page_texture(&self, page: &AtlasPage<'_>) -> wgpu::BindGroup {
        if let Some((version, group)) = self.pages.borrow().get(&page.id)
            && *version == page.version
        {
            return group.clone();
        }
        let mask = &page.mask;
        let (w, h) = (mask.width().max(1) as u32, mask.height().max(1) as u32);
        let mut bytes = Vec::with_capacity((w * h) as usize);
        for y in 0..mask.height() {
            bytes.extend_from_slice(mask.row(y));
        }
        bytes.resize((w * h) as usize, 0);
        let group = self.upload(w, h, wgpu::TextureFormat::R8Unorm, &bytes);
        self.page_uploads.set(self.page_uploads.get() + 1);
        self.pages
            .borrow_mut()
            .insert(page.id, (page.version, group.clone()));
        group
    }

    /// The texture holding `src`, uploaded now if this version has not been.
    fn image_texture(&self, src: &ImageRef<'_>) -> wgpu::BindGroup {
        if let Some((version, group)) = self.images.borrow().get(&src.id)
            && *version == src.version
        {
            return group.clone();
        }
        let size = src.view.size();
        let bytes = rgba_bytes(&src.view);
        let group = self.upload(
            size.width.max(1),
            size.height.max(1),
            wgpu::TextureFormat::Rgba8Unorm,
            &bytes,
        );
        self.image_uploads.set(self.image_uploads.get() + 1);
        self.images
            .borrow_mut()
            .insert(src.id, (src.version, group.clone()));
        group
    }

    fn upload(
        &self,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
        bytes: &[u8],
    ) -> wgpu::BindGroup {
        upload(
            &self.device,
            &self.queue,
            &self.texture_layout,
            &self.sampler,
            width,
            height,
            format,
            bytes,
        )
    }
}

/// Uploads one texture and binds it with the nearest sampler.
#[allow(clippy::too_many_arguments)]
fn upload(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
    bytes: &[u8],
) -> wgpu::BindGroup {
    let bytes_per_pixel = match format {
        wgpu::TextureFormat::R8Unorm => 1,
        _ => 4,
    };
    debug_assert_eq!(bytes.len(), (width * height * bytes_per_pixel) as usize);
    let texture = device.create_texture_with_data(
        queue,
        &wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        bytes,
    );
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
}

/// Which bind group a run of vertices draws with.
#[derive(Debug)]
enum Draw {
    /// Shapes: the white texture, never sampled.
    Shapes(Range<u32>),
    /// A blit: the texture at this index in the painter's list.
    Textured { texture: usize, range: Range<u32> },
}

/// One frame's worth of drawing, recorded and then encoded by
/// [`GpuPainter::finish`].
///
/// Obtained from [`Gpu::painter`]. Implements [`Painter`], so it is what a
/// [`Pen`](denise::Pen) wraps and what `denise-ui`'s `Ui::paint_with`
/// draws through.
pub struct GpuPainter<'g> {
    gpu: &'g Gpu,
    size: Size,
    clip: Rect,
    vertices: Vec<Vertex>,
    draws: Vec<Draw>,
    textures: Vec<wgpu::BindGroup>,
    /// Every edge of every polygon in the frame, as `x0, y0, x1, y1`. A
    /// polygon's vertices carry where its own run begins and how long it is,
    /// and the fragment shader reads the run back out.
    edges: Vec<[f32; 4]>,
}

impl GpuPainter<'_> {
    /// Encodes and submits the frame into `target`, which must have the
    /// format the [`Gpu`] was built for. The target is cleared first.
    pub fn finish(self, target: &wgpu::TextureView) {
        let gpu = self.gpu;
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("denise frame"),
            });
        self.encode(
            &mut encoder,
            target,
            wgpu::LoadOp::Clear(wgpu::Color::BLACK),
            None,
        );
        gpu.queue.submit([encoder.finish()]);
    }

    /// Draws onto `target` **without clearing it**, restricted to `damage`.
    ///
    /// For a target the caller keeps between frames. What `damage` does not
    /// cover is left exactly as the previous frame left it, which is what makes
    /// an incremental repaint possible: a swapchain image cannot be used this
    /// way because it rotates and its age cannot be trusted, so the caller owns
    /// a texture of its own and copies from it.
    ///
    /// `damage` is scissored to its **union**, not rectangle by rectangle. Two
    /// distant rectangles therefore cost their bounding box in fragments — the
    /// per-vertex clip still makes each region exact, so this decides how much
    /// is skipped rather than what is drawn. One pass is worth more than the
    /// tightest possible scissor, because doing better means replaying the
    /// vertices once per region.
    ///
    /// An empty `damage` draws nothing at all.
    pub fn finish_onto(self, target: &wgpu::TextureView, damage: &[Rect]) {
        let Some(union) = damage
            .iter()
            .filter(|r| !r.is_empty())
            .copied()
            .reduce(|a, b| a.union(&b))
        else {
            return;
        };
        let gpu = self.gpu;
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("denise damaged frame"),
            });
        self.encode(&mut encoder, target, wgpu::LoadOp::Load, Some(union));
        gpu.queue.submit([encoder.finish()]);
    }

    /// Renders offscreen and reads the frame back as `0xAARRGGBB` words, row
    /// after row with no padding — the layout a [`denise::Frame`] uses.
    ///
    /// Blocks until the GPU is done. For tests, snapshots and tools; a window
    /// should use [`finish`](GpuPainter::finish).
    pub fn finish_to_pixels(self) -> Result<Vec<u32>, Error> {
        let gpu = self.gpu;
        let (width, height) = (self.size.width.max(1), self.size.height.max(1));
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("denise offscreen"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: gpu.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("denise offscreen frame"),
            });
        self.encode(
            &mut encoder,
            &view,
            wgpu::LoadOp::Clear(wgpu::Color::BLACK),
            None,
        );
        gpu.queue.submit([encoder.finish()]);
        gpu.read_texture(&texture)
    }

    fn encode(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        load: wgpu::LoadOp<wgpu::Color>,
        scissor: Option<Rect>,
    ) {
        let gpu = self.gpu;
        let globals_group = gpu.globals_for(self.size);
        // An empty frame still clears; a zero-sized buffer is not allowed.
        let vertex_bytes: &[u8] = if self.vertices.is_empty() {
            &[0u8; std::mem::size_of::<Vertex>()]
        } else {
            bytemuck::cast_slice(&self.vertices)
        };
        // Allocated per frame, not written into a kept buffer: `write_buffer`
        // stages the copy, and for the small payload a damaged frame carries
        // that machinery costs more than the allocation it saves. Measured, in
        // both directions -- see the crate README.
        let vertices = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("denise vertices"),
                contents: vertex_bytes,
                usage: wgpu::BufferUsages::VERTEX,
            });

        // Most frames draw no polygon and bind the empty group the device was
        // built with; the rest pay for a buffer, per frame and for the same
        // reason the vertices are allocated per frame.
        let edges_group = if self.edges.is_empty() {
            None
        } else {
            let edges = gpu
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("denise polygon edges"),
                    contents: bytemuck::cast_slice(&self.edges),
                    usage: wgpu::BufferUsages::STORAGE,
                });
            Some(gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("denise polygon edges"),
                layout: &gpu.edges_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: edges.as_entire_binding(),
                }],
            }))
        };

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("denise"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&gpu.pipeline);
        pass.set_bind_group(0, &globals_group, &[]);
        pass.set_bind_group(2, edges_group.as_ref().unwrap_or(&gpu.no_edges), &[]);
        pass.set_vertex_buffer(0, vertices.slice(..));
        // The rasteriser skips everything outside the damage, so those
        // fragments never run; the per-vertex clip is what makes each region
        // exact within it.
        if let Some(r) = scissor {
            let x = r.x.clamp(0, self.size.width as i32) as u32;
            let y = r.y.clamp(0, self.size.height as i32) as u32;
            let w = (r.right().clamp(0, self.size.width as i32) as u32).saturating_sub(x);
            let h = (r.bottom().clamp(0, self.size.height as i32) as u32).saturating_sub(y);
            if w == 0 || h == 0 {
                return;
            }
            pass.set_scissor_rect(x, y, w, h);
        }
        for draw in &self.draws {
            match draw {
                Draw::Shapes(range) => {
                    pass.set_bind_group(1, &gpu.white, &[]);
                    pass.draw(range.clone(), 0..1);
                }
                Draw::Textured { texture, range } => {
                    pass.set_bind_group(1, &self.textures[*texture], &[]);
                    pass.draw(range.clone(), 0..1);
                }
            }
        }
    }

    // ---- recording ----------------------------------------------------------

    fn clip_f(&self) -> [f32; 4] {
        [
            self.clip.x as f32,
            self.clip.y as f32,
            self.clip.right() as f32,
            self.clip.bottom() as f32,
        ]
    }

    /// Appends one triangle to the current shape run, opening one if the last
    /// draw was a blit.
    fn triangle(
        &mut self,
        kind: u32,
        color: [f32; 4],
        a: [f32; 4],
        b: [f32; 4],
        pts: [[f32; 2]; 3],
    ) {
        let clip = self.clip_f();
        let start = self.vertices.len() as u32;
        debug_assert!(
            !is_textured(kind),
            "textured triangles go through `textured_quad`"
        );
        for pos in pts {
            self.vertices.push(Vertex {
                pos,
                clip,
                color,
                a,
                b,
                kind,
                poly: [0; 2],
                _pad: 0,
            });
        }
        let end = start + 3;
        match self.draws.last_mut() {
            Some(Draw::Shapes(range)) if range.end == start => range.end = end,
            _ => self.draws.push(Draw::Shapes(start..end)),
        }
    }

    /// A quad from `x0,y0` to `x1,y1` in pixels, as two shape triangles.
    fn quad(&mut self, kind: u32, color: [f32; 4], a: [f32; 4], b: [f32; 4], bounds: [f32; 4]) {
        let [x0, y0, x1, y1] = bounds;
        self.triangle(kind, color, a, b, [[x0, y0], [x1, y0], [x1, y1]]);
        self.triangle(kind, color, a, b, [[x0, y0], [x1, y1], [x0, y1]]);
    }

    /// The quad a polygon is drawn over, carrying its run of the frame's edge
    /// buffer. The only shape whose vertices say anything the fragment shader
    /// has to go and look up.
    fn polygon_quad(&mut self, color: [f32; 4], bounds: [f32; 4], run: [u32; 2]) {
        let clip = self.clip_f();
        let [x0, y0, x1, y1] = bounds;
        let start = self.vertices.len() as u32;
        for pos in [[x0, y0], [x1, y0], [x1, y1], [x0, y0], [x1, y1], [x0, y1]] {
            self.vertices.push(Vertex {
                pos,
                clip,
                color,
                a: [0.0; 4],
                b: [0.0; 4],
                kind: KIND_POLYGON,
                poly: run,
                _pad: 0,
            });
        }
        let end = start + 6;
        match self.draws.last_mut() {
            Some(Draw::Shapes(range)) if range.end == start => range.end = end,
            _ => self.draws.push(Draw::Shapes(start..end)),
        }
    }

    /// A textured quad covering `dest`, sampling the whole of texture `index`.
    /// A textured quad covering `dest`, sampling `uv` (`u0, v0, u1, v1`) of
    /// texture `index` — the whole of it for a blit, a glyph's rectangle for
    /// an atlas page.
    fn textured_quad(
        &mut self,
        kind: u32,
        color: [f32; 4],
        index: usize,
        dest: Rect,
        uv: [f32; 4],
        radius_box: ([f32; 4], f32),
    ) {
        let clip = self.clip_f();
        let (x0, y0) = (dest.x as f32, dest.y as f32);
        let (x1, y1) = (dest.right() as f32, dest.bottom() as f32);
        let [u0, v0, u1, v1] = uv;
        let corners = [
            ([x0, y0], [u0, v0]),
            ([x1, y0], [u1, v0]),
            ([x1, y1], [u1, v1]),
            ([x0, y1], [u0, v1]),
        ];
        let (b, radius) = radius_box;
        let start = self.vertices.len() as u32;
        for i in [0usize, 1, 2, 0, 2, 3] {
            let (pos, uv) = corners[i];
            self.vertices.push(Vertex {
                pos,
                clip,
                color,
                a: [uv[0], uv[1], radius, 0.0],
                b,
                kind,
                poly: [0; 2],
                _pad: 0,
            });
        }
        self.draws.push(Draw::Textured {
            texture: index,
            range: start..start + 6,
        });
    }

    fn upload_view(&mut self, src: &PixelView<'_>) -> usize {
        let size = src.size();
        let bytes = rgba_bytes(src);
        self.textures.push(self.gpu.upload(
            size.width,
            size.height,
            wgpu::TextureFormat::Rgba8Unorm,
            &bytes,
        ));
        self.textures.len() - 1
    }
}

/// A premultiplied `0xAARRGGBB` view as the R G B A bytes a texture wants.
fn rgba_bytes(src: &PixelView<'_>) -> Vec<u8> {
    let size = src.size();
    let mut bytes = Vec::with_capacity((size.width * size.height * 4) as usize);
    for y in 0..size.height as i32 {
        let row = src.row(y, 0, size.width as i32).unwrap_or(&[]);
        for &word in row {
            bytes.extend_from_slice(&[
                (word >> 16) as u8,
                (word >> 8) as u8,
                word as u8,
                (word >> 24) as u8,
            ]);
        }
    }
    // A view narrower than it claims still fills its texture.
    bytes.resize((size.width.max(1) * size.height.max(1) * 4) as usize, 0);
    bytes
}

fn is_textured(kind: u32) -> bool {
    matches!(kind, KIND_TEXTURED | KIND_MASK | KIND_TEXTURED_ROUNDED)
}

/// A premultiplied `Paint` as the `[r, g, b, a]` floats the shader blends with.
fn rgba(paint: Paint) -> [f32; 4] {
    let w = paint.premultiplied();
    [
        ((w >> 16) & 0xFF) as f32 / 255.0,
        ((w >> 8) & 0xFF) as f32 / 255.0,
        (w & 0xFF) as f32 / 255.0,
        ((w >> 24) & 0xFF) as f32 / 255.0,
    ]
}

/// Centre and half-extents of `rect`, in continuous pixel coordinates.
fn box_of(rect: Rect) -> [f32; 4] {
    let hw = rect.width as f32 / 2.0;
    let hh = rect.height as f32 / 2.0;
    [rect.x as f32 + hw, rect.y as f32 + hh, hw, hh]
}

impl Painter for GpuPainter<'_> {
    fn size(&self) -> Size {
        self.size
    }

    fn format(&self) -> PixelFormat {
        PixelFormat::Argb8888
    }

    fn clip(&self) -> Rect {
        self.clip
    }

    fn push_clip(&mut self, rect: Rect) -> ClipToken {
        let previous = self.clip;
        self.clip = self.clip.intersect(&rect).unwrap_or(Rect::ZERO);
        ClipToken::restoring(previous)
    }

    fn pop_clip(&mut self, token: ClipToken) {
        self.clip = token.previous();
    }

    fn clear(&mut self, color: Color) {
        let clip = self.clip;
        self.fill_rect(clip, Paint::new(Color::rgb(color.r, color.g, color.b)));
    }

    fn fill_rect(&mut self, rect: Rect, paint: Paint) {
        if paint.is_invisible() || rect.is_empty() || self.clip.is_empty() {
            return;
        }
        let c = rgba(paint);
        self.quad(
            KIND_SOLID,
            c,
            [0.0; 4],
            [0.0; 4],
            [
                rect.x as f32,
                rect.y as f32,
                rect.right() as f32,
                rect.bottom() as f32,
            ],
        );
    }

    fn fill_rounded_rect(&mut self, rect: Rect, radius: i32, paint: Paint) {
        if paint.is_invisible() || rect.is_empty() || self.clip.is_empty() {
            return;
        }
        let r = radius.clamp(0, rect.width.min(rect.height) / 2);
        if r == 0 {
            return self.fill_rect(rect, paint);
        }
        let c = rgba(paint);
        self.quad(
            KIND_ROUNDED_FILL,
            c,
            box_of(rect),
            [r as f32, 0.0, 0.0, 0.0],
            [
                rect.x as f32,
                rect.y as f32,
                rect.right() as f32,
                rect.bottom() as f32,
            ],
        );
    }

    fn stroke_rounded_rect(&mut self, rect: Rect, radius: i32, thickness: i32, paint: Paint) {
        let t = thickness.max(0);
        if t == 0 || paint.is_invisible() || rect.is_empty() || self.clip.is_empty() {
            return;
        }
        if t * 2 >= rect.width.min(rect.height) {
            return self.fill_rounded_rect(rect, radius, paint);
        }
        let r = radius.clamp(0, rect.width.min(rect.height) / 2);
        let c = rgba(paint);
        self.quad(
            KIND_ROUNDED_STROKE,
            c,
            box_of(rect),
            [r as f32, t as f32, 0.0, 0.0],
            [
                rect.x as f32,
                rect.y as f32,
                rect.right() as f32,
                rect.bottom() as f32,
            ],
        );
    }

    fn fill_circle(&mut self, centre: Point, radius: i32, paint: Paint) {
        if radius <= 0 || paint.is_invisible() || self.clip.is_empty() {
            return;
        }
        let (cx, cy, r) = (centre.x as f32, centre.y as f32, radius as f32);
        let c = rgba(paint);
        self.quad(
            KIND_CIRCLE_FILL,
            c,
            [cx, cy, r, 0.0],
            [0.0; 4],
            [cx - r - 1.0, cy - r - 1.0, cx + r + 1.0, cy + r + 1.0],
        );
    }

    fn stroke_circle(&mut self, centre: Point, radius: i32, thickness: i32, paint: Paint) {
        let t = thickness.max(0);
        if t == 0 || radius <= 0 || paint.is_invisible() || self.clip.is_empty() {
            return;
        }
        if t >= radius {
            return self.fill_circle(centre, radius, paint);
        }
        let (cx, cy, r) = (centre.x as f32, centre.y as f32, radius as f32);
        let c = rgba(paint);
        self.quad(
            KIND_CIRCLE_STROKE,
            c,
            [cx, cy, r, t as f32],
            [0.0; 4],
            [cx - r - 1.0, cy - r - 1.0, cx + r + 1.0, cy + r + 1.0],
        );
    }

    fn stroke_arc(
        &mut self,
        centre: Point,
        radius: i32,
        thickness: i32,
        start: i32,
        sweep: i32,
        paint: Paint,
    ) {
        let t = thickness.max(0);
        if t == 0 || radius <= 0 || sweep == 0 || paint.is_invisible() || self.clip.is_empty() {
            return;
        }
        // Negative sweeps go anticlockwise: the same arc, described from its
        // other end.
        let (start, sweep) = if sweep < 0 {
            (start.wrapping_add(sweep), -(sweep as i64))
        } else {
            (start, sweep as i64)
        };
        if sweep >= TURN as i64 {
            return self.stroke_circle(centre, radius, thickness, paint);
        }
        let start = start.rem_euclid(TURN) as f32 / TURN as f32;
        let sweep = sweep as f32 / TURN as f32;
        let (cx, cy, r) = (centre.x as f32, centre.y as f32, radius as f32);
        let c = rgba(paint);
        self.quad(
            KIND_ARC,
            c,
            [cx, cy, r, t.min(radius) as f32],
            [start, sweep, 0.0, 0.0],
            [cx - r - 1.0, cy - r - 1.0, cx + r + 1.0, cy + r + 1.0],
        );
    }

    fn draw_line(&mut self, a: Point, b: Point, paint: Paint) {
        if paint.is_invisible() || self.clip.is_empty() {
            return;
        }
        if a == b {
            return self.fill_rect(Rect::new(a.x, a.y, 1, 1), paint);
        }
        // Endpoints at pixel centres, a half-pixel wide capsule: the fragment
        // whose centre a Bresenham line would light is exactly the one the
        // distance says is covered.
        let (ax, ay) = (a.x as f32 + 0.5, a.y as f32 + 0.5);
        let (bx, by) = (b.x as f32 + 0.5, b.y as f32 + 0.5);
        let (dx, dy) = (bx - ax, by - ay);
        let len = (dx * dx + dy * dy).sqrt();
        let (ux, uy) = (dx / len, dy / len);
        let (px, py) = (-uy, ux);
        let c = rgba(paint);
        let pa = [ax, ay, bx, by];
        let pb = [0.5, 0.0, 0.0, 0.0];
        let corners = [
            [ax - ux - px, ay - uy - py],
            [bx + ux - px, by + uy - py],
            [bx + ux + px, by + uy + py],
            [ax - ux + px, ay - uy + py],
        ];
        self.triangle(KIND_LINE, c, pa, pb, [corners[0], corners[1], corners[2]]);
        self.triangle(KIND_LINE, c, pa, pb, [corners[0], corners[2], corners[3]]);
    }

    fn fill_polygon_fx(&mut self, points: &[(i32, i32)], paint: Paint) {
        if points.len() < 3 || paint.is_invisible() || self.clip.is_empty() {
            return;
        }
        let first = self.edges.len() as u32;
        let (mut left, mut top) = (f32::MAX, f32::MAX);
        let (mut right, mut bottom) = (f32::MIN, f32::MIN);
        let at = |(x, y): (i32, i32)| [x as f32 / ONE as f32, y as f32 / ONE as f32];
        for i in 0..points.len() {
            let p = at(points[i]);
            let q = at(points[(i + 1) % points.len()]);
            self.edges.push([p[0], p[1], q[0], q[1]]);
            left = left.min(p[0]);
            top = top.min(p[1]);
            right = right.max(p[0]);
            bottom = bottom.max(p[1]);
        }
        // One quad over the bounds, grown by a pixel so the fragments the
        // outline passes through are inside it and can be partly covered. The
        // shader does the rest: nothing here decides what is filled.
        let bounds = [left - 1.0, top - 1.0, right + 1.0, bottom + 1.0];
        let run = [first, points.len() as u32];
        self.polygon_quad(rgba(paint), bounds, run);
    }

    fn blit_mask(&mut self, at: Point, mask: &Mask<'_>, paint: Paint) {
        if paint.is_invisible() || self.clip.is_empty() {
            return;
        }
        let (w, h) = (mask.width(), mask.height());
        if w <= 0 || h <= 0 {
            return;
        }
        let mut bytes = Vec::with_capacity((w * h) as usize);
        for y in 0..h {
            bytes.extend_from_slice(mask.row(y));
        }
        self.textures.push(self.gpu.upload(
            w as u32,
            h as u32,
            wgpu::TextureFormat::R8Unorm,
            &bytes,
        ));
        let index = self.textures.len() - 1;
        self.textured_quad(
            KIND_MASK,
            rgba(paint),
            index,
            mask.bounds_at(at),
            WHOLE,
            ([0.0; 4], 0.0),
        );
    }

    fn blit_glyph(&mut self, at: Point, page: &AtlasPage<'_>, rect: Rect, paint: Paint) {
        if paint.is_invisible() || rect.is_empty() || self.clip.is_empty() {
            return;
        }
        let (pw, ph) = (page.mask.width() as f32, page.mask.height() as f32);
        if pw <= 0.0 || ph <= 0.0 {
            return;
        }
        // The page is uploaded once per version; this is six vertices.
        let group = self.gpu.page_texture(page);
        self.textures.push(group);
        let index = self.textures.len() - 1;
        let uv = [
            rect.x as f32 / pw,
            rect.y as f32 / ph,
            rect.right() as f32 / pw,
            rect.bottom() as f32 / ph,
        ];
        let dest = Rect::new(at.x, at.y, rect.width, rect.height);
        self.textured_quad(KIND_MASK, rgba(paint), index, dest, uv, ([0.0; 4], 0.0));
    }

    fn blit_image(&mut self, src: &ImageRef<'_>, dest: Rect) {
        if src.view.size().is_empty() || dest.is_empty() || self.clip.is_empty() {
            return;
        }
        // Uploaded once per version; this is six vertices.
        let group = self.gpu.image_texture(src);
        self.textures.push(group);
        let index = self.textures.len() - 1;
        self.textured_quad(KIND_TEXTURED, [1.0; 4], index, dest, WHOLE, ([0.0; 4], 0.0));
    }

    fn blit_image_rounded(&mut self, src: &ImageRef<'_>, dest: Rect, shape: Rect, radius: i32) {
        if src.view.size().is_empty() || dest.is_empty() || self.clip.is_empty() {
            return;
        }
        let group = self.gpu.image_texture(src);
        self.textures.push(group);
        let index = self.textures.len() - 1;
        let r = radius.clamp(0, shape.width.min(shape.height) / 2) as f32;
        self.textured_quad(
            KIND_TEXTURED_ROUNDED,
            [1.0; 4],
            index,
            dest,
            WHOLE,
            (box_of(shape), r),
        );
    }

    fn blit(&mut self, src: &PixelView<'_>, at: Point) {
        let size = src.size();
        if size.is_empty() || self.clip.is_empty() {
            return;
        }
        let index = self.upload_view(src);
        let dest = Rect::new(at.x, at.y, size.width as i32, size.height as i32);
        self.textured_quad(KIND_TEXTURED, [1.0; 4], index, dest, WHOLE, ([0.0; 4], 0.0));
    }

    fn blit_scaled(&mut self, src: &PixelView<'_>, dest: Rect) {
        if src.size().is_empty() || dest.is_empty() || self.clip.is_empty() {
            return;
        }
        let index = self.upload_view(src);
        self.textured_quad(KIND_TEXTURED, [1.0; 4], index, dest, WHOLE, ([0.0; 4], 0.0));
    }

    fn blit_rounded(&mut self, src: &PixelView<'_>, dest: Rect, shape: Rect, radius: i32) {
        if src.size().is_empty() || dest.is_empty() || self.clip.is_empty() {
            return;
        }
        let index = self.upload_view(src);
        let r = radius.clamp(0, shape.width.min(shape.height) / 2) as f32;
        self.textured_quad(
            KIND_TEXTURED_ROUNDED,
            [1.0; 4],
            index,
            dest,
            WHOLE,
            (box_of(shape), r),
        );
    }
}

/// Compiles the examples in this crate's README, so they cannot drift from the API
/// they claim to demonstrate. Never built except under `cargo test --doc`.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct Readme;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paint_converts_to_premultiplied_floats() {
        let c = rgba(Paint::new(Color::rgba(255, 0, 0, 128)));
        assert!((c[3] - 128.0 / 255.0).abs() < 1e-6);
        assert!(c[0] > 0.49 && c[0] < 0.51, "red is premultiplied: {}", c[0]);
        assert_eq!(c[1], 0.0);
    }
}
