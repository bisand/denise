//! The stateful decode session: compressed bytes in, dmabuf frames out.
//!
//! This is the half of the format menu that most of the embedded world
//! shares: the driver parses everything, userspace only moves buffers. The
//! canonical M2M flow, spelled out because every step is load-bearing:
//!
//! 1. `S_FMT` the **output** (compressed) queue to the codec, allocate and
//!    mmap its buffers, `STREAMON`.
//! 2. Feed access units. The driver parses the parameter sets and raises
//!    `SOURCE_CHANGE` when it knows the real dimensions.
//! 3. Only then negotiate the **capture** queue: `G_FMT` for what the driver
//!    chose, allocate its buffers, export each as a **dmabuf**, queue them
//!    all, `STREAMON`.
//! 4. Loop: dequeue a decoded frame, hand its dmabuf to the plane, requeue it
//!    once scanout has moved on.
//!
//! Everything is non-blocking; [`Decoder::pump`] is called from the
//! application's event loop and never waits.

use std::fs::File;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::path::Path;

use crate::VideoError;
use crate::annexb::Codec;
use crate::v4l2;

/// Compressed buffers on the output queue: enough to keep the decoder fed
/// without pumping every frame.
const OUTPUT_BUFFERS: u32 = 4;
/// Room for one access unit. 1 MiB holds any 1080p AU with head to spare.
const OUTPUT_BUFFER_BYTES: u32 = 1 << 20;

/// One mmapped compressed-input buffer.
struct OutputBuffer {
    /// The mapping, kept alive for the session; unmapped on drop.
    map: MmapRegion,
    queued: bool,
}

/// An owned `mmap` region.
///
/// A hand-rolled holder rather than `memmap2`, because the mapping must come
/// from the V4L2 buffer offset protocol and be unmapped exactly once.
struct MmapRegion {
    ptr: *mut core::ffi::c_void,
    len: usize,
}

// SAFETY: the region is exclusively owned; the pointer is only dereferenced
// through `&mut self`.
unsafe impl Send for MmapRegion {}

impl MmapRegion {
    fn map(fd: BorrowedFd<'_>, offset: u64, len: usize) -> Result<Self, VideoError> {
        use rustix::mm::{MapFlags, ProtFlags, mmap};
        // SAFETY: mapping a fresh region chosen by the kernel (addr null); the
        // fd and offset come from QUERYBUF on this very device, which is the
        // documented way to reach a V4L2 MMAP buffer.
        let ptr = unsafe {
            mmap(
                core::ptr::null_mut(),
                len,
                ProtFlags::READ | ProtFlags::WRITE,
                MapFlags::SHARED,
                fd,
                offset,
            )
        }
        .map_err(|e| VideoError::v4l2("mmap", e))?;
        Ok(Self { ptr, len })
    }

    fn write(&mut self, bytes: &[u8]) -> usize {
        let n = bytes.len().min(self.len);
        // SAFETY: the region is `len` bytes, mapped read-write, exclusively
        // owned; `n` is clamped to it.
        unsafe {
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), self.ptr.cast::<u8>(), n);
        }
        n
    }
}

impl Drop for MmapRegion {
    fn drop(&mut self) {
        // SAFETY: exactly the region mmap returned, unmapped once.
        unsafe {
            let _ = rustix::mm::munmap(self.ptr, self.len);
        }
    }
}

/// The negotiated capture side, once the stream's dimensions are known.
struct CaptureSide {
    /// One exported dmabuf per capture buffer, index-aligned.
    dmabufs: Vec<OwnedFd>,
    width: u32,
    height: u32,
    /// The driver's chosen fourcc: NV12 or YU12 on the boards this targets.
    pixelformat: u32,
    /// Bytes per luma row.
    stride: u32,
    /// Bytes per buffer — where the chroma planes live is derived from this
    /// and the stride by the plane importer.
    sizeimage: u32,
}

/// One decoded picture, ready for scanout.
///
/// Borrows nothing: the dmabuf stays owned by the [`Decoder`], and the frame
/// names it by capture-buffer `index`. Display it, then give the buffer back
/// with [`Decoder::recycle`] once scanout has moved past it.
#[derive(Debug, Clone, Copy)]
pub struct DecodedFrame {
    /// Which capture buffer holds the picture — the key for
    /// [`Decoder::dmabuf`] and [`Decoder::recycle`].
    pub index: u32,
    /// Picture width in pixels.
    pub width: u32,
    /// Picture height in pixels.
    pub height: u32,
    /// The V4L2 fourcc of the pixel data.
    pub pixelformat: u32,
    /// Bytes per luma row.
    pub stride: u32,
    /// Total bytes in the buffer.
    pub sizeimage: u32,
}

/// A stateful V4L2 decode session on one device node.
pub struct Decoder {
    file: File,
    codec: Codec,
    output: Vec<OutputBuffer>,
    capture: Option<CaptureSide>,
    /// Set once `STREAMON` has run on the output queue.
    streaming: bool,
}

impl Decoder {
    /// Opens `path` and prepares the compressed side for `codec`.
    ///
    /// The capture side is deliberately not touched: its geometry belongs to
    /// the stream, and the driver announces it through `SOURCE_CHANGE` once
    /// it has parsed the headers this session feeds it.
    pub fn open(path: impl AsRef<Path>, codec: Codec) -> Result<Self, VideoError> {
        use rustix::fs::{OFlags, fcntl_setfl};
        let path = path.as_ref();
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .map_err(|source| VideoError::Open {
                path: path.to_path_buf(),
                source,
            })?;
        fcntl_setfl(&file, OFlags::NONBLOCK).map_err(|e| VideoError::v4l2("fcntl", e))?;
        let fd = file.as_fd();

        v4l2::subscribe_event(fd, v4l2::EVENT_SOURCE_CHANGE)
            .map_err(|e| VideoError::v4l2("subscribe source_change", e))?;

        // The compressed side: fourcc and a buffer size, no geometry — the
        // stream knows its own.
        let mut format = v4l2::Format::zeroed(v4l2::BUF_TYPE_OUTPUT_MPLANE);
        {
            let pix = format.pix_mp_mut();
            pix.pixelformat = match codec {
                Codec::H264 => v4l2::PIX_FMT_H264,
                Codec::H265 => v4l2::PIX_FMT_HEVC,
            };
            pix.num_planes = 1;
            pix.plane_fmt[0].sizeimage = OUTPUT_BUFFER_BYTES;
        }
        v4l2::s_fmt(fd, &mut format).map_err(|e| VideoError::v4l2("s_fmt output", e))?;

        let granted = v4l2::reqbufs(fd, v4l2::BUF_TYPE_OUTPUT_MPLANE, OUTPUT_BUFFERS)
            .map_err(|e| VideoError::v4l2("reqbufs output", e))?;
        let mut output = Vec::with_capacity(granted as usize);
        for index in 0..granted {
            let mut planes = [v4l2::Plane::zeroed()];
            v4l2::querybuf(fd, v4l2::BUF_TYPE_OUTPUT_MPLANE, index, &mut planes)
                .map_err(|e| VideoError::v4l2("querybuf output", e))?;
            // SAFETY-relevant reads of the union: for MEMORY_MMAP the kernel
            // filled `mem_offset`.
            let offset = unsafe { planes[0].m.mem_offset } as u64;
            let len = planes[0].length as usize;
            output.push(OutputBuffer {
                map: MmapRegion::map(fd, offset, len)?,
                queued: false,
            });
        }

        Ok(Self {
            file,
            codec,
            output,
            capture: None,
            streaming: false,
        })
    }

    /// Which codec this session decodes.
    pub fn codec(&self) -> Codec {
        self.codec
    }

    /// Whether a compressed buffer is free right now — feed only when true.
    pub fn ready_for_input(&mut self) -> bool {
        self.reclaim_output();
        self.output.iter().any(|b| !b.queued)
    }

    /// Queues one access unit, starting the stream on the first.
    ///
    /// Returns `false` — feeding nothing — when every buffer is in flight;
    /// call [`Decoder::pump`] and try again. An access unit larger than the
    /// buffer is truncated, which cannot happen inside the menu's 1080p bound.
    pub fn feed(&mut self, access_unit: &[u8]) -> Result<bool, VideoError> {
        self.reclaim_output();
        let Some(index) = self.output.iter().position(|b| !b.queued) else {
            return Ok(false);
        };
        let used = self.output[index].map.write(access_unit);
        let fd = self.file.as_fd();
        let mut planes = [v4l2::Plane::zeroed()];
        planes[0].bytesused = used as u32;
        let mut buffer =
            v4l2::Buffer::mplane(index as u32, v4l2::BUF_TYPE_OUTPUT_MPLANE, &mut planes);
        v4l2::qbuf(fd, &mut buffer).map_err(|e| VideoError::v4l2("qbuf output", e))?;
        self.output[index].queued = true;
        if !self.streaming {
            v4l2::streamon(fd, v4l2::BUF_TYPE_OUTPUT_MPLANE)
                .map_err(|e| VideoError::v4l2("streamon output", e))?;
            self.streaming = true;
        }
        Ok(true)
    }

    /// Takes back compressed buffers the driver has consumed.
    fn reclaim_output(&mut self) {
        let fd = self.file.as_fd();
        loop {
            let mut planes = [v4l2::Plane::zeroed()];
            let mut buffer = v4l2::Buffer::mplane(0, v4l2::BUF_TYPE_OUTPUT_MPLANE, &mut planes);
            match v4l2::dqbuf(fd, &mut buffer) {
                Ok(Some(())) => {
                    if let Some(slot) = self.output.get_mut(buffer.index as usize) {
                        slot.queued = false;
                    }
                }
                _ => break,
            }
        }
    }

    /// Advances the session: handles the source-change handshake, and returns
    /// a decoded frame when one is ready. Never blocks.
    pub fn pump(&mut self) -> Result<Option<DecodedFrame>, VideoError> {
        self.reclaim_output();

        // The driver announcing it has parsed the stream is what makes the
        // capture side negotiable at all.
        while let Some(event) =
            v4l2::dqevent(self.file.as_fd()).map_err(|e| VideoError::v4l2("dqevent", e))?
        {
            if event.event_type == v4l2::EVENT_SOURCE_CHANGE && self.capture.is_none() {
                self.setup_capture()?;
            }
        }

        let Some(capture) = &self.capture else {
            return Ok(None);
        };
        let mut planes = [v4l2::Plane::zeroed()];
        let mut buffer = v4l2::Buffer::mplane(0, v4l2::BUF_TYPE_CAPTURE_MPLANE, &mut planes);
        match v4l2::dqbuf(self.file.as_fd(), &mut buffer)
            .map_err(|e| VideoError::v4l2("dqbuf capture", e))?
        {
            None => Ok(None),
            Some(()) => Ok(Some(DecodedFrame {
                index: buffer.index,
                width: capture.width,
                height: capture.height,
                pixelformat: capture.pixelformat,
                stride: capture.stride,
                sizeimage: capture.sizeimage,
            })),
        }
    }

    /// Negotiates the capture side after the driver has parsed the stream.
    fn setup_capture(&mut self) -> Result<(), VideoError> {
        let fd = self.file.as_fd();
        let format = v4l2::g_fmt(fd, v4l2::BUF_TYPE_CAPTURE_MPLANE)
            .map_err(|e| VideoError::v4l2("g_fmt capture", e))?;
        let pix = format.pix_mp();
        match pix.pixelformat {
            v4l2::PIX_FMT_NV12 | v4l2::PIX_FMT_YUV420 => {}
            other => return Err(VideoError::UnsupportedFormat(other)),
        }

        let granted = v4l2::reqbufs(fd, v4l2::BUF_TYPE_CAPTURE_MPLANE, 4)
            .map_err(|e| VideoError::v4l2("reqbufs capture", e))?;
        let mut dmabufs = Vec::with_capacity(granted as usize);
        for index in 0..granted {
            let raw = v4l2::expbuf(fd, v4l2::BUF_TYPE_CAPTURE_MPLANE, index, 0)
                .map_err(|e| VideoError::v4l2("expbuf", e))?;
            // SAFETY: EXPBUF returns a fresh fd owned by nobody else; wrapping
            // it transfers that ownership exactly once.
            dmabufs.push(unsafe { OwnedFd::from_raw_fd_checked(raw) });
            let mut planes = [v4l2::Plane::zeroed()];
            let mut buffer =
                v4l2::Buffer::mplane(index, v4l2::BUF_TYPE_CAPTURE_MPLANE, &mut planes);
            v4l2::qbuf(fd, &mut buffer).map_err(|e| VideoError::v4l2("qbuf capture", e))?;
        }
        v4l2::streamon(fd, v4l2::BUF_TYPE_CAPTURE_MPLANE)
            .map_err(|e| VideoError::v4l2("streamon capture", e))?;

        self.capture = Some(CaptureSide {
            dmabufs,
            width: pix.width,
            height: pix.height,
            pixelformat: pix.pixelformat,
            stride: pix.plane_fmt[0].bytesperline,
            sizeimage: pix.plane_fmt[0].sizeimage,
        });
        Ok(())
    }

    /// The dmabuf backing capture buffer `index`.
    pub fn dmabuf(&self, index: u32) -> Option<BorrowedFd<'_>> {
        self.capture
            .as_ref()
            .and_then(|c| c.dmabufs.get(index as usize))
            .map(|fd| fd.as_fd())
    }

    /// Gives a displayed frame's buffer back to the decoder.
    ///
    /// Call once scanout has moved past it — in practice, when the *next*
    /// frame has been flipped onto the plane.
    pub fn recycle(&mut self, index: u32) -> Result<(), VideoError> {
        let mut planes = [v4l2::Plane::zeroed()];
        let mut buffer = v4l2::Buffer::mplane(index, v4l2::BUF_TYPE_CAPTURE_MPLANE, &mut planes);
        v4l2::qbuf(self.file.as_fd(), &mut buffer).map_err(|e| VideoError::v4l2("qbuf capture", e))
    }

    /// Stops both queues, forgetting all stream state; the next
    /// [`Decoder::feed`] starts the stream over. This is the whole of
    /// seeking: a promo loop restarts from its parameter sets.
    pub fn restart(&mut self) -> Result<(), VideoError> {
        let fd = self.file.as_fd();
        if self.streaming {
            v4l2::streamoff(fd, v4l2::BUF_TYPE_OUTPUT_MPLANE)
                .map_err(|e| VideoError::v4l2("streamoff output", e))?;
            self.streaming = false;
        }
        for slot in &mut self.output {
            slot.queued = false;
        }
        if self.capture.is_some() {
            v4l2::streamoff(fd, v4l2::BUF_TYPE_CAPTURE_MPLANE)
                .map_err(|e| VideoError::v4l2("streamoff capture", e))?;
            // Requeue every capture buffer for the fresh run; geometry is
            // unchanged — a loop plays the same file.
            let count = self
                .capture
                .as_ref()
                .map(|c| c.dmabufs.len() as u32)
                .unwrap_or(0);
            v4l2::streamon(fd, v4l2::BUF_TYPE_CAPTURE_MPLANE)
                .map_err(|e| VideoError::v4l2("streamon capture", e))?;
            for index in 0..count {
                let mut planes = [v4l2::Plane::zeroed()];
                let mut buffer =
                    v4l2::Buffer::mplane(index, v4l2::BUF_TYPE_CAPTURE_MPLANE, &mut planes);
                v4l2::qbuf(fd, &mut buffer).map_err(|e| VideoError::v4l2("qbuf capture", e))?;
            }
        }
        Ok(())
    }
}

/// `OwnedFd` construction from a raw fd, named so the SAFETY reasoning has an
/// address.
trait FromRawChecked {
    /// SAFETY: `raw` must be an open fd owned by nobody else.
    unsafe fn from_raw_fd_checked(raw: i32) -> OwnedFd;
}

impl FromRawChecked for OwnedFd {
    unsafe fn from_raw_fd_checked(raw: i32) -> OwnedFd {
        use std::os::fd::FromRawFd;
        // SAFETY: forwarded from the caller's contract.
        unsafe { OwnedFd::from_raw_fd(raw) }
    }
}
