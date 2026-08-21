//! A [`denise::Surface`] over a winit window, presented with softbuffer.

use std::num::NonZeroU32;
use std::rc::Rc;

use denise::{
    BufferAge, DamageTracker, Frame, InputEvent, InputSource, MAX_DAMAGE_RECTS, PixelFormat, Rect,
    Size, Surface, SurfaceError,
};
use winit::window::Window;

use crate::Error;

/// A resizable window surface backed by a persistent shadow buffer.
///
/// softbuffer hands out a buffer whose contents are undefined or stale depending on
/// the platform, which would force a full repaint every frame and make the damage
/// path untestable on the desktop. Instead the application draws into a shadow
/// buffer we own — always current, so always [`BufferAge::Frames(1)`](denise::BufferAge::Frames) — and
/// `present` blits from it into softbuffer's buffer, using softbuffer's own
/// reported age to decide how much of the shadow to copy.
///
/// That extra blit is a development-backend cost and nothing else. On DRM and fbdev
/// the buffer the application draws into *is* the scanout buffer.
pub struct WinitSurface {
    window: Rc<Window>,
    softbuffer: softbuffer::Surface<Rc<Window>, Rc<Window>>,
    /// Persistent CPU-side surface. Tightly packed, so stride == width.
    shadow: Vec<u32>,
    size: Size,
    scale_factor: f32,
    /// Damage owed to softbuffer's buffer, which ages independently of the shadow.
    blit_damage: DamageTracker,
    /// Reused across frames so `present` does not allocate.
    present_rects: Vec<softbuffer::Rect>,
    events: Vec<InputEvent>,
    frame_in_flight: bool,
    shadow_initialised: bool,
}

impl WinitSurface {
    /// Binds a surface to an existing window.
    pub fn new(window: Rc<Window>) -> Result<Self, Error> {
        let context = softbuffer::Context::new(window.clone())?;
        let softbuffer = softbuffer::Surface::new(&context, window.clone())?;

        let inner = window.inner_size();
        let size = Size::new(inner.width, inner.height);

        let mut this = Self {
            window: window.clone(),
            softbuffer,
            shadow: Vec::new(),
            size: Size::ZERO,
            scale_factor: window.scale_factor() as f32,
            blit_damage: DamageTracker::new(size),
            present_rects: Vec::with_capacity(MAX_DAMAGE_RECTS),
            events: Vec::new(),
            frame_in_flight: false,
            shadow_initialised: false,
        };
        this.resize(size, window.scale_factor() as f32);
        Ok(this)
    }

    /// The window this surface draws to.
    pub fn window(&self) -> &Rc<Window> {
        &self.window
    }

    /// Reallocates for a new size or DPI. Discards all damage history.
    pub fn resize(&mut self, size: Size, scale_factor: f32) {
        if size == self.size && scale_factor == self.scale_factor {
            return;
        }
        self.size = size;
        self.scale_factor = scale_factor;
        self.blit_damage.resize(size);

        let needed = (size.area() as usize).min(isize::MAX as usize);
        self.shadow.clear();
        self.shadow.resize(needed, 0);
        self.shadow_initialised = false;
    }

    /// Queues an event for the next [`InputSource::poll`].
    pub(crate) fn push_event(&mut self, event: InputEvent) {
        self.events.push(event);
    }
}

impl Surface for WinitSurface {
    fn size(&self) -> Size {
        self.size
    }

    fn scale_factor(&self) -> f32 {
        self.scale_factor
    }

    fn format(&self) -> PixelFormat {
        // softbuffer scans out 0RGB and ignores the high byte.
        PixelFormat::Xrgb8888
    }

    fn acquire(&mut self) -> Result<Frame<'_>, SurfaceError> {
        if self.frame_in_flight {
            return Err(SurfaceError::FrameInFlight);
        }
        if self.size.is_empty() {
            return Err(SurfaceError::NotReady);
        }

        // The shadow persists across presents, so it is never more than one frame
        // behind — except the very first time, when it is uninitialised memory.
        let age = if self.shadow_initialised {
            BufferAge::Frames(1)
        } else {
            BufferAge::Undefined
        };

        let frame = Frame::new(
            &mut self.shadow,
            self.size,
            self.size.width,
            PixelFormat::Xrgb8888,
            age,
        )?;
        self.frame_in_flight = true;
        Ok(frame)
    }

    fn present(&mut self, damage: &[Rect]) -> Result<(), SurfaceError> {
        if !self.frame_in_flight {
            return Err(SurfaceError::NoFrame);
        }
        self.frame_in_flight = false;
        self.shadow_initialised = true;

        let (Some(w), Some(h)) = (
            NonZeroU32::new(self.size.width),
            NonZeroU32::new(self.size.height),
        ) else {
            return Err(SurfaceError::NotReady);
        };

        self.softbuffer
            .resize(w, h)
            .map_err(SurfaceError::backend_msg)?;
        let mut buffer = self
            .softbuffer
            .buffer_mut()
            .map_err(SurfaceError::backend_msg)?;

        // softbuffer's buffer ages independently of our shadow: it may be a
        // different physical allocation every frame. Ask it, then copy whatever it
        // has missed since it was last current.
        for r in damage {
            self.blit_damage.add(*r);
        }
        let sb_age = match buffer.age() {
            0 => BufferAge::Undefined,
            n => BufferAge::Frames(u32::from(n)),
        };

        // Copy out of the tracker so it can be advanced while the rects are in use.
        let mut resolved = [Rect::ZERO; MAX_DAMAGE_RECTS];
        let count = {
            let src = self.blit_damage.resolve(sb_age);
            resolved[..src.len()].copy_from_slice(src);
            src.len()
        };

        let stride = self.size.width as usize;
        self.present_rects.clear();
        for r in &resolved[..count] {
            let (Some(rw), Some(rh)) = (
                NonZeroU32::new(r.width as u32),
                NonZeroU32::new(r.height as u32),
            ) else {
                continue;
            };
            let x0 = r.x as usize;
            let x1 = r.right() as usize;
            for y in r.y as usize..r.bottom() as usize {
                let start = y * stride;
                buffer[start + x0..start + x1]
                    .copy_from_slice(&self.shadow[start + x0..start + x1]);
            }
            self.present_rects.push(softbuffer::Rect {
                x: r.x as u32,
                y: r.y as u32,
                width: rw,
                height: rh,
            });
        }

        buffer
            .present_with_damage(&self.present_rects)
            .map_err(SurfaceError::backend_msg)?;
        self.blit_damage.end_frame();
        Ok(())
    }
}

impl InputSource for WinitSurface {
    fn poll(&mut self, out: &mut Vec<InputEvent>) {
        out.append(&mut self.events);
    }
}
