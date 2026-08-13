//! Decoded frames onto a DRM video plane, zero-copy.
//!
//! The display controller composites planes during scanout, so a frame shown
//! this way never touches `denise-render` or the CPU at all: the decoder's
//! dmabuf is imported as a DRM framebuffer and the plane is pointed at it.
//! The UI keeps painting its own primary buffer; on vc4 the video plane
//! stacks with it, positioned wherever the tree put the `Video` rectangle.
//!
//! # The card is shared, not reopened
//!
//! One process is DRM master, and it is the process driving the surface. The
//! plane is therefore driven through **the same [`Card`]** —
//! [`DrmSurface::card`](denise_drm::DrmSurface::card) is the seam — never a
//! second open, which would either fail or fight.

use std::collections::HashMap;
use std::os::fd::BorrowedFd;

use drm::buffer::{DrmFourcc, Handle as BufferHandle, PlanarBuffer};
use drm::control::{Device as ControlDevice, crtc, framebuffer, plane};

use denise::Rect;
use denise_drm::Card;

use crate::VideoError;
use crate::decode::{DecodedFrame, Decoder};
use crate::v4l2;

/// A dmabuf described well enough for `AddFB2`.
struct Imported {
    size: (u32, u32),
    format: DrmFourcc,
    pitches: [u32; 4],
    handles: [Option<BufferHandle>; 4],
    offsets: [u32; 4],
}

impl PlanarBuffer for Imported {
    fn size(&self) -> (u32, u32) {
        self.size
    }
    fn format(&self) -> DrmFourcc {
        self.format
    }
    fn modifier(&self) -> Option<drm::buffer::DrmModifier> {
        None
    }
    fn pitches(&self) -> [u32; 4] {
        self.pitches
    }
    fn handles(&self) -> [Option<BufferHandle>; 4] {
        self.handles
    }
    fn offsets(&self) -> [u32; 4] {
        self.offsets
    }
}

/// A video plane on the surface's CRTC, fed dmabuf frames.
pub struct VideoPlane {
    plane: plane::Handle,
    crtc: crtc::Handle,
    /// Framebuffers by capture-buffer index: each dmabuf is imported once and
    /// flipped many times.
    framebuffers: HashMap<u32, framebuffer::Handle>,
    /// Where on screen the video sits, in surface pixels.
    dst: Rect,
    shown: bool,
}

impl VideoPlane {
    /// Finds a plane on `card` that can sit on `crtc` and scan out NV12 or
    /// YUV420 — the formats the menu's decoders produce.
    pub fn new(card: &Card, crtc: crtc::Handle, dst: Rect) -> Result<Self, VideoError> {
        let resources = card
            .resource_handles()
            .map_err(|e| VideoError::drm("resources", e))?;
        let planes = card
            .plane_handles()
            .map_err(|e| VideoError::drm("planes", e))?;
        for handle in planes {
            let Ok(info) = card.get_plane(handle) else {
                continue;
            };
            // The filter names which CRTCs this plane may sit on.
            if !resources
                .filter_crtcs(info.possible_crtcs())
                .contains(&crtc)
            {
                continue;
            }
            // The primary plane is the UI's; never take it. Planes already in
            // use carry a framebuffer.
            if info.crtc().is_some() || info.framebuffer().is_some() {
                continue;
            }
            let formats = info.formats();
            let takes = |f: DrmFourcc| formats.contains(&(f as u32));
            if takes(DrmFourcc::Nv12) || takes(DrmFourcc::Yuv420) {
                return Ok(Self {
                    plane: handle,
                    crtc,
                    framebuffers: HashMap::new(),
                    dst,
                    shown: false,
                });
            }
        }
        Err(VideoError::NoPlane)
    }

    /// Moves or resizes where the video sits. Takes effect on the next frame.
    pub fn set_dst(&mut self, dst: Rect) {
        self.dst = dst;
    }

    /// Shows `frame` on the plane, importing its dmabuf on first sight.
    pub fn show(
        &mut self,
        card: &Card,
        decoder: &Decoder,
        frame: &DecodedFrame,
    ) -> Result<(), VideoError> {
        let fb = match self.framebuffers.get(&frame.index) {
            Some(&fb) => fb,
            None => {
                let dmabuf = decoder.dmabuf(frame.index).ok_or(VideoError::NoFrames)?;
                let fb = import(card, dmabuf, frame)?;
                self.framebuffers.insert(frame.index, fb);
                fb
            }
        };
        card.set_plane(
            self.plane,
            self.crtc,
            Some(fb),
            0,
            (
                self.dst.x,
                self.dst.y,
                self.dst.width.max(0) as u32,
                self.dst.height.max(0) as u32,
            ),
            // Source is in 16.16 fixed point, whole frame.
            (0, 0, frame.width << 16, frame.height << 16),
        )
        .map_err(|e| VideoError::drm("set_plane", e))?;
        self.shown = true;
        Ok(())
    }

    /// Takes the plane off screen. The UI underneath is untouched — it was
    /// never painted over.
    pub fn hide(&mut self, card: &Card) -> Result<(), VideoError> {
        if !self.shown {
            return Ok(());
        }
        card.set_plane(self.plane, self.crtc, None, 0, (0, 0, 0, 0), (0, 0, 0, 0))
            .map_err(|e| VideoError::drm("set_plane off", e))?;
        self.shown = false;
        Ok(())
    }
}

/// Imports one dmabuf as a planar framebuffer.
///
/// The V4L2 formats here are single-memory-plane: every picture plane lives in
/// the one dmabuf at offsets derived from the stride, which is exactly what
/// `AddFB2`'s per-plane offsets express.
fn import(
    card: &Card,
    dmabuf: BorrowedFd<'_>,
    frame: &DecodedFrame,
) -> Result<framebuffer::Handle, VideoError> {
    let handle = card
        .prime_fd_to_buffer(dmabuf)
        .map_err(|e| VideoError::drm("prime import", e))?;
    let luma = frame.stride * frame.height;
    let imported = match frame.pixelformat {
        v4l2::PIX_FMT_NV12 => Imported {
            size: (frame.width, frame.height),
            format: DrmFourcc::Nv12,
            pitches: [frame.stride, frame.stride, 0, 0],
            handles: [Some(handle), Some(handle), None, None],
            offsets: [0, luma, 0, 0],
        },
        v4l2::PIX_FMT_YUV420 => Imported {
            size: (frame.width, frame.height),
            format: DrmFourcc::Yuv420,
            pitches: [frame.stride, frame.stride / 2, frame.stride / 2, 0],
            handles: [Some(handle), Some(handle), Some(handle), None],
            offsets: [0, luma, luma + luma / 4, 0],
        },
        other => return Err(VideoError::UnsupportedFormat(other)),
    };
    card.add_planar_framebuffer(&imported, drm::control::FbCmd2Flags::empty())
        .map_err(|e| VideoError::drm("addfb2", e))
}
