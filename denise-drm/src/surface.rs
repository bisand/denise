//! The scanout surface: dumb buffers, modeset, and page flips.

use denise::{Frame, PixelFormat, Rect, Size, Surface, SurfaceError};
use drm::Device as _;
use drm::DriverCapability;
use drm::buffer::Buffer as _;
use drm::control::{
    Device as ControlDevice, Event, Mode, PageFlipFlags, connector, crtc, dumbbuffer::DumbBuffer,
    framebuffer,
};
use drm_fourcc::DrmFourcc;

use crate::device::Card;
use crate::error::DrmError;
use crate::mode::{self, ModePreference, OutputPreference};
use crate::swapchain::Swapchain;

/// Bits per pixel of the scanout format.
const BPP: u32 = 32;
/// Colour depth, excluding the ignored high byte.
const DEPTH: u32 = 24;

/// When a queued flip actually reaches the panel.
///
/// A real trade, not a quality setting. Which way it should go depends on what is
/// on the screen, and the default here is chosen for the kind of thing Denise is
/// built for rather than for the kind of thing a compositor is built for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PresentMode {
    /// Flip immediately, part-way through a scan-out if necessary. Tears.
    ///
    /// The default, and measured on a Pi 3 A+ driving 1920x1080 the reason is
    /// plain: waiting for vblank costs about 17 ms of latency and was described by
    /// the person operating it as lagging several milliseconds behind the whole
    /// time. Flipping immediately removes that wait entirely.
    ///
    /// The cost is a horizontal seam where the panel switched buffers mid-frame.
    /// With damage tracking most updates are a few thousand pixels, so the seam is
    /// small, brief and in practice invisible — a control panel redrawing a button
    /// is nothing like a compositor scrolling a window.
    ///
    /// Reconsider for signage or anything with large fast-moving content, where a
    /// tear crosses something worth looking at. Requires `DRM_CAP_ASYNC_PAGE_FLIP`;
    /// drivers without it fall back to [`PresentMode::Vsync`], and
    /// [`DrmSurface::present_mode`] reports what was actually obtained.
    ///
    /// **This mode does not pace the caller.** See [`DrmSurface`].
    #[default]
    Immediate,

    /// Flip at the next vblank. Never tears.
    ///
    /// A flip queued just after a vblank cannot land before the next one, so this
    /// costs on the order of one refresh period — about 17 ms at 60 Hz. Every
    /// tear-free system pays it.
    ///
    /// In exchange, [`Surface::acquire`] blocks until the flip retires, so the
    /// display paces the render loop for free and an application needs no frame
    /// timing of its own.
    Vsync,
}

/// How to bring the display up.
#[derive(Clone, Copy, Debug)]
pub struct SurfaceConfig {
    /// Which output to drive.
    pub output: OutputPreference,
    /// Which mode to set on it.
    pub mode: ModePreference,
    /// How many scanout buffers to rotate through.
    ///
    /// Two by default. Three trades latency for smoothness, which is the wrong
    /// trade for a panel someone is touching.
    pub buffers: usize,

    /// Whether to wait for vblank before showing a frame.
    pub present_mode: PresentMode,
}

impl Default for SurfaceConfig {
    fn default() -> Self {
        Self {
            output: OutputPreference::Auto,
            mode: ModePreference::Preferred,
            buffers: 2,
            present_mode: PresentMode::Vsync,
        }
    }
}

/// One scanout buffer: the allocation, its framebuffer id, and its CPU mapping.
#[derive(Debug)]
struct Scanout {
    dumb: DumbBuffer,
    fb: framebuffer::Handle,
    /// Start of the mapping, as `u32` words.
    ptr: *mut u32,
    /// Length of the mapping in words.
    words: usize,
    /// Length of the mapping in bytes, for `munmap`.
    bytes: usize,
}

impl Scanout {
    fn new(card: &Card, size: Size) -> Result<Self, DrmError> {
        let mut dumb = card
            .create_dumb_buffer((size.width, size.height), DrmFourcc::Xrgb8888, BPP)
            .map_err(|source| DrmError::Allocate {
                width: size.width,
                height: size.height,
                source,
            })?;

        let fb = card
            .add_framebuffer(&dumb, DEPTH, BPP)
            .map_err(DrmError::AddFramebuffer)?;

        // The `drm` crate's mapping unmaps itself on drop, which cannot work here:
        // the mapping has to outlive the call that made it, and a `Frame` handed to
        // the renderer borrows from it. So take the pointer and forget the guard,
        // making this code responsible for the `munmap` in `DrmSurface::drop`.
        // Mapping once at start-up also saves an mmap/munmap pair every frame.
        let (ptr, bytes) = {
            let mut mapping = card.map_dumb_buffer(&mut dumb).map_err(DrmError::Map)?;
            let slice: &mut [u8] = &mut mapping;
            let ptr = slice.as_mut_ptr();
            let bytes = slice.len();
            core::mem::forget(mapping);
            (ptr, bytes)
        };

        let mut scanout = Self {
            dumb,
            fb,
            // SAFETY: `mmap` returns page-aligned memory, which satisfies `u32`
            // alignment. The cast does not change the region's extent; `words`
            // below accounts for the narrower element type.
            ptr: ptr.cast::<u32>(),
            words: bytes / 4,
            bytes,
        };

        // A freshly allocated dumb buffer holds whatever was in that memory.
        // Without this, the modeset shows one frame of garbage before the first
        // repaint lands.
        scanout.pixels_mut().fill(0);

        Ok(scanout)
    }

    fn pixels_mut(&mut self) -> &mut [u32] {
        // SAFETY: `ptr` and `words` come from a single successful mapping of this
        // buffer, which stays mapped until `DrmSurface::drop` unmaps it. `&mut
        // self` rules out any other live reference to the same region.
        unsafe { core::slice::from_raw_parts_mut(self.ptr, self.words) }
    }
}

/// A display brought up under our control, scanning out CPU-rendered buffers.
///
/// Takes DRM master on construction and gives it back on drop, restoring whatever
/// the CRTC was showing before. A clean exit and a panic both hand the console
/// back, rather than leaving a black screen that needs a power cycle.
///
/// # Pacing is the caller's job under [`PresentMode::Immediate`]
///
/// Under [`PresentMode::Vsync`], [`acquire`](Surface::acquire) blocks until the
/// previous flip retires, so a bare `loop { acquire; draw; present }` runs at
/// exactly the refresh rate and costs nothing extra.
///
/// Under [`PresentMode::Immediate`] — the default — nothing waits. The same loop
/// runs as fast as the CPU allows and will happily use a whole core drawing
/// frames no one will ever see. An application must either draw only when
/// something changed, which damage tracking makes natural, or keep a frame
/// deadline of its own. `examples/kiosk` does both.
///
/// This is not a flaw in async flips; it is what removing the wait means.
#[derive(Debug)]
pub struct DrmSurface {
    // `pub(crate)` for the cursor plane, which lives in its own module and needs
    // the card and the CRTC to talk to.
    pub(crate) card: Card,
    pub(crate) crtc: crtc::Handle,
    connector: connector::Handle,
    buffers: Vec<Scanout>,
    swapchain: Swapchain,
    size: Size,
    /// Row stride in pixels, from the driver's pitch. Rarely equals the width.
    stride: u32,
    /// A flip has been queued and its completion event not yet read.
    flip_pending: bool,
    /// The mode actually in force, after checking what the driver supports.
    present_mode: PresentMode,
    flip_flags: PageFlipFlags,
    saved_crtc: Option<crtc::Info>,
    mode_name: String,
    /// The hardware cursor plane's buffer, allocated on first use. `None` until
    /// an application asks for a sprite, because a panel driven by touch never
    /// wants one and should not pay for the allocation.
    pub(crate) cursor: Option<crate::cursor::CursorBuffer>,
}

impl DrmSurface {
    /// Brings up the display.
    pub fn new(card: Card, config: SurfaceConfig) -> Result<Self, DrmError> {
        card.become_master()?;

        let (handles, infos) = card.connectors()?;
        let selection = mode::select(&infos, config.output, config.mode)?;
        let connector = handles[selection.connector];
        let crtc = card.crtc_for(connector)?;

        // Re-read the connector for the driver's own `Mode`, since the selection
        // policy works on a copy that deliberately drops the timing details.
        let info = card
            .get_connector(connector, false)
            .map_err(DrmError::Resources)?;
        let mode: Mode = info.modes()[selection.mode];
        let (width, height) = mode.size();
        let size = Size::new(u32::from(width), u32::from(height));

        let saved_crtc = card.get_crtc(crtc).ok();

        let mut buffers = Vec::with_capacity(config.buffers);
        for _ in 0..Swapchain::new(config.buffers).count() {
            buffers.push(Scanout::new(&card, size)?);
        }

        // The pitch is the driver's, not ours: it is padded for alignment and is
        // routinely wider than the visible row. Everything downstream addresses
        // rows through this, never through the width.
        let pitch = buffers[0].dumb.pitch();
        if !pitch.is_multiple_of(4) {
            return Err(DrmError::UnalignedPitch { pitch });
        }

        // Ask the driver rather than assume. Requesting an async flip on hardware
        // that cannot do one fails the ioctl every frame, which would turn a
        // latency preference into a display that never updates.
        let async_capable = card
            .get_driver_capability(DriverCapability::ASyncPageFlip)
            .is_ok_and(|supported| supported != 0);

        let present_mode = match config.present_mode {
            PresentMode::Immediate if async_capable => PresentMode::Immediate,
            _ => PresentMode::Vsync,
        };
        let flip_flags = match present_mode {
            PresentMode::Vsync => PageFlipFlags::EVENT,
            PresentMode::Immediate => PageFlipFlags::EVENT | PageFlipFlags::ASYNC,
        };

        card.set_crtc(crtc, Some(buffers[0].fb), (0, 0), &[connector], Some(mode))
            .map_err(|source| DrmError::SetMode {
                mode: format!("{width}x{height}"),
                crtc: u32::from(crtc),
                source,
            })?;

        // Buffer 0 is now being scanned out, so the next frame must not draw into
        // it. Recording the modeset as a presentation advances past it.
        let mut swapchain = Swapchain::new(config.buffers);
        swapchain.presented();

        Ok(Self {
            card,
            crtc,
            connector,
            buffers,
            swapchain,
            size,
            stride: pitch / 4,
            flip_pending: false,
            present_mode,
            flip_flags,
            saved_crtc,
            mode_name: format!("{width}x{height}@{}", mode.vrefresh()),
            cursor: None,
        })
    }

    /// Opens the first display-capable device and brings it up.
    pub fn open(config: SurfaceConfig) -> Result<Self, DrmError> {
        Self::new(Card::open_first()?, config)
    }

    /// The open card, for driving DRM objects the surface does not own —
    /// video planes above all. One process is DRM master, so anything else
    /// touching the display **must** go through this card rather than a
    /// second open, which would either fail or fight. `denise-video` is the
    /// consumer this seam exists for.
    pub fn card(&self) -> &Card {
        &self.card
    }

    /// The CRTC being driven, for placing planes on it.
    pub fn crtc(&self) -> drm::control::crtc::Handle {
        self.crtc
    }

    /// The mode in force, for logging.
    pub fn mode_name(&self) -> &str {
        &self.mode_name
    }

    /// Row stride in pixels.
    pub fn stride(&self) -> u32 {
        self.stride
    }

    /// Number of buffers in rotation.
    pub fn buffer_count(&self) -> usize {
        self.buffers.len()
    }

    /// The presentation mode actually in force.
    ///
    /// May be [`PresentMode::Vsync`] even when [`PresentMode::Immediate`] was
    /// asked for, if the driver does not advertise `DRM_CAP_ASYNC_PAGE_FLIP`.
    pub fn present_mode(&self) -> PresentMode {
        self.present_mode
    }

    /// Blocks until any queued flip has actually happened.
    ///
    /// This is the vsync wait, and it is where the frame loop should spend its
    /// idle time: the process sleeps in the kernel until the scanout engine is
    /// done, instead of spinning to guess when that was.
    ///
    /// How long that sleep lasts is the driver's business, not ours, and not every
    /// driver makes it last. `virtio-gpu` under a hypervisor completes the flip as
    /// soon as the host acknowledges it, so this returns immediately and the loop
    /// runs at thousands of frames a second on a 75 Hz mode. Real scanout hardware
    /// — vc4 on a Pi, for one — retires the flip at vblank and this blocks for the
    /// rest of the frame.
    ///
    /// A caller that must not spin when the driver declines to pace it needs its
    /// own frame deadline on top. That belongs in the event loop, with input, and
    /// arrives with it.
    fn wait_for_flip(&mut self) -> Result<(), DrmError> {
        while self.flip_pending {
            let events = self.card.receive_events().map_err(DrmError::WaitVblank)?;
            for event in events {
                if matches!(event, Event::PageFlip(_)) {
                    self.flip_pending = false;
                }
            }
        }
        Ok(())
    }
}

impl Surface for DrmSurface {
    fn size(&self) -> Size {
        self.size
    }

    fn scale_factor(&self) -> f32 {
        // DRM has no notion of a scale factor. A panel's physical size is known,
        // but turning that into a UI scale is policy, and policy does not belong
        // in the backend.
        1.0
    }

    fn format(&self) -> PixelFormat {
        PixelFormat::Xrgb8888
    }

    fn acquire(&mut self) -> Result<Frame<'_>, SurfaceError> {
        // The buffer we are about to hand out may still be on screen until the
        // previous flip retires. Drawing into it before then is what tearing is.
        self.wait_for_flip()?;

        let index = self.swapchain.current();
        let age = self.swapchain.age();
        let size = self.size;
        let stride = self.stride;

        Frame::new(
            self.buffers[index].pixels_mut(),
            size,
            stride,
            PixelFormat::Xrgb8888,
            age,
        )
    }

    fn present(&mut self, _damage: &[Rect]) -> Result<(), SurfaceError> {
        // The damage list is deliberately ignored. A page flip swaps whole
        // buffers; there is no partial upload to restrict. Damage still pays for
        // itself here, upstream, in the pixels the rasteriser never touched —
        // which is the larger win anyway. Wiring damage into the presentation
        // would need atomic modesetting and FB_DAMAGE_CLIPS, and most drivers
        // ignore that property regardless.
        let index = self.swapchain.current();
        let fb = self.buffers[index].fb;

        self.card
            .page_flip(self.crtc, fb, self.flip_flags, None)
            .map_err(DrmError::PageFlip)?;

        self.flip_pending = true;
        self.swapchain.presented();
        Ok(())
    }
}

impl Drop for DrmSurface {
    fn drop(&mut self) {
        // Let the last flip retire before pulling the buffers out from under the
        // scanout engine.
        let _ = self.wait_for_flip();

        if let Some(saved) = self.saved_crtc.as_ref() {
            let _ = self.card.set_crtc(
                self.crtc,
                saved.framebuffer(),
                saved.position(),
                &[self.connector],
                saved.mode(),
            );
        }

        if let Some(cursor) = self.cursor.take() {
            // Off the CRTC before the memory goes, or the scanout engine keeps
            // compositing a freed buffer.
            #[allow(deprecated)]
            let _ = self
                .card
                .set_cursor(self.crtc, None::<&drm::control::dumbbuffer::DumbBuffer>);
            cursor.release(&self.card);
        }

        for buffer in self.buffers.drain(..) {
            // SAFETY: `ptr`/`bytes` describe exactly the mapping made in
            // `Scanout::new`, whose guard was forgotten so that this code owns it.
            // Nothing else can reference the region: the buffer has been moved out
            // of `self.buffers` and any `Frame` borrowing it is long dropped.
            unsafe {
                let _ = rustix::mm::munmap(buffer.ptr.cast::<core::ffi::c_void>(), buffer.bytes);
            }
            let _ = self.card.destroy_framebuffer(buffer.fb);
            let _ = self.card.destroy_dumb_buffer(buffer.dumb);
        }

        self.card.release_master();
    }
}
