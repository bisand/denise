//! A Denise widget tree in a winit window, painted on the GPU.
//!
//! ```text
//! cargo run -p denise-wgpu --example window
//! cargo run -p denise-wgpu --example window -- --snapshot out.ppm [scale]
//! ```
//!
//! The tree is ordinary `denise-ui`; nothing in it knows it is not being
//! rasterised. What differs from `examples/hello` is the last thirty lines: a
//! wgpu surface on the window instead of a `denise-winit` frame, and
//! `Ui::paint_with` instead of `Ui::paint`. `--snapshot` draws the same tree
//! headless and writes a PPM, which needs no display at all.

use std::sync::Arc;
use std::time::{Duration, Instant};

use denise::{
    BufferAge, ElementState, InputEvent, Modifiers, Pen, Point, PointerButton, Rect, Role, Size,
    theme,
};
use denise_ui::Ui;
use denise_ui::widgets::{
    Button, Checkbox, Label, Panel, Progress, RadialProgress, Rating, Slider, TextInput, Toggle,
};
use denise_wgpu::Gpu;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

const WINDOW: Size = Size::new(520, 360);

#[derive(Clone, Copy, Debug, PartialEq)]
enum Msg {
    Go,
    Flip(bool),
    Check(bool),
    Slide(f32),
    Rate(f32),
}

/// The tree, at a scale. The layout is designed in logical pixels and
/// multiplied once here, exactly as `examples/hello` does it.
fn build(size: Size, scale: f32) -> Ui<Msg> {
    let s = |r: Rect| r.scaled(scale);
    let px = |v: f32| (v * scale + 0.5) as u16;

    let mut ui: Ui<Msg> = Ui::new(size, theme::DARK.scaled(scale));
    let root = ui.root();
    let card_size = s(Rect::new(0, 0, 488, 328));
    let card = ui
        .add(
            root,
            Panel::default(),
            Rect::new(
                (size.width as i32 - card_size.width) / 2,
                (size.height as i32 - card_size.height) / 2,
                card_size.width,
                card_size.height,
            ),
        )
        .expect("card");

    ui.add(
        card,
        Label::new("Painted on the GPU").with_size(px(22.0)),
        s(Rect::new(20, 16, 440, 28)),
    );
    ui.add(
        card,
        Label::new("Every widget below is drawing through denise-wgpu.").with_size(px(14.0)),
        s(Rect::new(20, 48, 440, 20)),
    );

    ui.add(
        card,
        Button::new("Go", Msg::Go)
            .with_role(Role::Primary)
            .with_size(px(16.0)),
        s(Rect::new(20, 84, 100, 34)),
    );
    ui.add(
        card,
        Toggle::new("Toggle", Msg::Flip)
            .with_checked(true)
            .with_size(px(16.0)),
        s(Rect::new(140, 84, 150, 34)),
    );
    ui.add(
        card,
        Checkbox::new("Checkbox", Msg::Check)
            .with_checked(true)
            .with_size(px(16.0)),
        s(Rect::new(310, 84, 160, 34)),
    );

    ui.add(
        card,
        Slider::new(0.0, 100.0, 62.0, Msg::Slide),
        s(Rect::new(20, 136, 280, 28)),
    );
    ui.add(card, Progress::new(0.62), s(Rect::new(20, 176, 280, 12)));
    ui.add(
        card,
        RadialProgress::new(0.62).with_label("62%"),
        s(Rect::new(330, 120, 120, 120)),
    );

    ui.add(
        card,
        Rating::new(3.5, Msg::Rate),
        s(Rect::new(20, 204, 200, 30)),
    );

    ui.add(
        card,
        TextInput::<Msg>::new()
            .with_placeholder("type here")
            .with_size(px(16.0)),
        s(Rect::new(20, 252, 440, 36)),
    );
    ui
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() == Some("--snapshot") {
        let path = args.next().unwrap_or_else(|| "window.ppm".into());
        let scale: f32 = args.next().and_then(|a| a.parse().ok()).unwrap_or(1.0);
        return snapshot(&path, scale);
    }

    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Wait);
    let mut app = App { live: None };
    event_loop.run_app(&mut app)?;
    Ok(())
}

/// Headless: any adapter, no window, one frame to a file.
fn snapshot(path: &str, scale: f32) -> Result<(), Box<dyn std::error::Error>> {
    use std::io::Write as _;

    let size = Size::new(
        (WINDOW.width as f32 * scale + 0.5) as u32,
        (WINDOW.height as f32 * scale + 0.5) as u32,
    );
    let gpu = Gpu::headless()?;
    let mut ui = build(size, scale);
    let mut painter = gpu.painter(size);
    ui.paint_with(&mut Pen::new(&mut painter), BufferAge::Undefined);
    let pixels = painter.finish_to_pixels()?;

    let mut out = std::io::BufWriter::new(std::fs::File::create(path)?);
    write!(out, "P6\n{} {}\n255\n", size.width, size.height)?;
    for word in &pixels {
        out.write_all(&[(word >> 16) as u8, (word >> 8) as u8, *word as u8])?;
    }
    out.flush()?;
    eprintln!("wrote {path} at {}x{}", size.width, size.height);
    Ok(())
}

struct App {
    live: Option<Live>,
}

/// Everything that exists only while the window does.
struct Live {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    gpu: Gpu,
    ui: Ui<Msg>,
    started: Instant,
    pointer: Point,
}

impl Live {
    fn open(event_loop: &ActiveEventLoop) -> Result<Self, Box<dyn std::error::Error>> {
        let attrs = Window::default_attributes()
            .with_title("Denise — wgpu")
            .with_inner_size(LogicalSize::new(WINDOW.width, WINDOW.height))
            .with_resizable(false);
        let window = Arc::new(event_loop.create_window(attrs)?);

        let instance = wgpu::Instance::default();
        let surface = instance.create_surface(window.clone())?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: Some(&surface),
            ..Default::default()
        }))?;
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("denise window"),
                ..Default::default()
            }))?;

        // A non-sRGB format where one exists: Denise's colours are bytes meant
        // for the screen, and an sRGB target would convert them on the way in.
        let physical = window.inner_size();
        let mut config = surface
            .get_default_config(&adapter, physical.width.max(1), physical.height.max(1))
            .ok_or("the adapter cannot present to this window")?;
        let caps = surface.get_capabilities(&adapter);
        if let Some(format) = caps.formats.iter().copied().find(|f| !f.is_srgb()) {
            config.format = format;
        }
        config.present_mode = wgpu::PresentMode::AutoVsync;
        let format = config.format;
        surface.configure(&device, &config);

        let gpu = Gpu::new(device, queue, format);
        let scale = window.scale_factor() as f32;
        let size = Size::new(config.width, config.height);
        let ui = build(size, scale);

        Ok(Self {
            window,
            surface,
            config,
            gpu,
            ui,
            started: Instant::now(),
            pointer: Point::new(-1, -1),
        })
    }

    fn now_ms(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }

    fn redraw(&mut self) {
        use wgpu::CurrentSurfaceTexture as Current;
        let frame = match self.surface.get_current_texture() {
            Current::Success(frame) | Current::Suboptimal(frame) => frame,
            Current::Lost | Current::Outdated => {
                self.surface.configure(self.gpu.device(), &self.config);
                return self.window.request_redraw();
            }
            Current::Timeout | Current::Occluded => return,
            Current::Validation => {
                eprintln!("surface: validation error");
                return;
            }
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let now = self.now_ms();
        self.ui.tick(now);
        let mut painter = self
            .gpu
            .painter(Size::new(self.config.width, self.config.height));
        self.ui
            .paint_with(&mut Pen::new(&mut painter), BufferAge::Undefined);
        painter.finish(&view);

        self.window.pre_present_notify();
        self.gpu.queue().present(frame);
        self.ui.presented();
    }

    fn handle(&mut self, event: InputEvent) {
        self.ui.handle(&[event]);
        for message in self.ui.drain_messages() {
            eprintln!("{message:?}");
        }
        if self.ui.needs_paint() {
            self.window.request_redraw();
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.live.is_none() {
            match Live::open(event_loop) {
                Ok(live) => self.live = Some(live),
                Err(err) => {
                    eprintln!("{err}");
                    event_loop.exit();
                }
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(live) = self.live.as_mut() else {
            return;
        };
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::RedrawRequested => live.redraw(),
            WindowEvent::Resized(size) => {
                live.config.width = size.width.max(1);
                live.config.height = size.height.max(1);
                live.surface.configure(live.gpu.device(), &live.config);
                live.window.request_redraw();
            }
            WindowEvent::CursorMoved { position, .. } => {
                live.pointer = Point::new(position.x as i32, position.y as i32);
                let position = live.pointer;
                live.handle(InputEvent::PointerMoved { position });
            }
            WindowEvent::CursorLeft { .. } => live.handle(InputEvent::PointerLeft),
            WindowEvent::MouseInput { state, button, .. } => {
                let button = match button {
                    MouseButton::Left => PointerButton::Left,
                    MouseButton::Right => PointerButton::Right,
                    MouseButton::Middle => PointerButton::Middle,
                    _ => return,
                };
                let state = match state {
                    winit::event::ElementState::Pressed => ElementState::Down,
                    winit::event::ElementState::Released => ElementState::Up,
                };
                let position = live.pointer;
                live.handle(InputEvent::PointerButton {
                    button,
                    state,
                    position,
                    modifiers: Modifiers::default(),
                });
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let (delta_x, delta_y) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => (x * 40.0, y * 40.0),
                    MouseScrollDelta::PixelDelta(p) => (p.x as f32, p.y as f32),
                };
                let position = live.pointer;
                live.handle(InputEvent::PointerScroll {
                    delta_x,
                    delta_y,
                    position,
                });
            }
            WindowEvent::KeyboardInput { event, .. }
                if event.logical_key == Key::Named(NamedKey::Escape) =>
            {
                event_loop.exit()
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let Some(live) = self.live.as_mut() else {
            return;
        };
        // Animations advance on the clock, not on input: ask again shortly and
        // redraw only if the tick left something to draw.
        live.ui.tick(live.now_ms());
        if live.ui.needs_paint() {
            live.window.request_redraw();
        }
        event_loop.set_control_flow(ControlFlow::WaitUntil(
            Instant::now() + Duration::from_millis(16),
        ));
    }
}
