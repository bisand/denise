//! The V4L2 uapi subset a stateful decoder needs, spoken directly.
//!
//! No `v4l` crate and no C library: these are the `videodev2.h` structs and
//! ioctls written out by hand, issued through `rustix`. The subset is exactly
//! what memory-to-memory decode uses — capability query, format enumeration
//! and negotiation, buffer allocation, the queue loop, and dmabuf export.
//!
//! # Layout is ABI, and 32-bit ARM is the reason for the care
//!
//! Every struct here is `#[repr(C)]` and must match the kernel's byte for
//! byte on **both** `aarch64` and `armv7`. The hazards are the ones C hides:
//! `unsigned long` and pointers change size (represented as `usize` here,
//! which tracks them), and [`Buffer`](crate::v4l2::Buffer)'s embedded `timeval` is two C `long`s —
//! the ioctl *number* encodes the struct's size, so a struct built with the
//! 32-bit layout automatically selects the kernel's matching handler. Nothing
//! here uses `time64` variants; a video frame timestamp does not live long
//! enough to meet 2038.

#![allow(dead_code)]

use std::os::fd::BorrowedFd;

use rustix::io::Errno;
use rustix::ioctl::{self, Ioctl, IoctlOutput, Opcode};

/// A fourcc, as V4L2 spells them.
pub const fn fourcc(code: &[u8; 4]) -> u32 {
    u32::from_le_bytes(*code)
}

/// Compressed H.264, Annex-B byte stream.
pub const PIX_FMT_H264: u32 = fourcc(b"H264");
/// Compressed HEVC, Annex-B byte stream.
pub const PIX_FMT_HEVC: u32 = fourcc(b"HEVC");
/// NV12: luma plane then interleaved chroma, one memory plane.
pub const PIX_FMT_NV12: u32 = fourcc(b"NV12");
/// Planar YUV 4:2:0, one memory plane.
pub const PIX_FMT_YUV420: u32 = fourcc(b"YU12");

/// `V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE`: where decoded frames come out.
pub const BUF_TYPE_CAPTURE_MPLANE: u32 = 9;
/// `V4L2_BUF_TYPE_VIDEO_OUTPUT_MPLANE`: where compressed bytes go in.
pub const BUF_TYPE_OUTPUT_MPLANE: u32 = 10;

/// `V4L2_MEMORY_MMAP`.
pub const MEMORY_MMAP: u32 = 1;

/// `V4L2_CAP_VIDEO_M2M_MPLANE`: the capability that makes a node a decoder.
pub const CAP_VIDEO_M2M_MPLANE: u32 = 0x0000_4000;
/// `V4L2_CAP_STREAMING`.
pub const CAP_STREAMING: u32 = 0x0400_0000;

/// `V4L2_FMT_FLAG_COMPRESSED` on an enumerated format.
pub const FMT_FLAG_COMPRESSED: u32 = 0x0001;

/// The kernel's `struct timeval` as V4L2 buffers carry it: two C `long`s,
/// which `isize` matches on every Linux target this crate builds for.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Timeval {
    pub tv_sec: isize,
    pub tv_usec: isize,
}

/// `struct v4l2_capability`.
#[repr(C)]
pub struct Capability {
    pub driver: [u8; 16],
    pub card: [u8; 32],
    pub bus_info: [u8; 32],
    pub version: u32,
    pub capabilities: u32,
    pub device_caps: u32,
    pub reserved: [u32; 3],
}

impl Capability {
    pub const fn zeroed() -> Self {
        // SAFETY-free: the struct is plain integers and arrays; all-zero is a
        // valid value of every field.
        Self {
            driver: [0; 16],
            card: [0; 32],
            bus_info: [0; 32],
            version: 0,
            capabilities: 0,
            device_caps: 0,
            reserved: [0; 3],
        }
    }

    /// The caps that matter: `device_caps` when the driver fills it.
    pub fn caps(&self) -> u32 {
        if self.device_caps != 0 {
            self.device_caps
        } else {
            self.capabilities
        }
    }

    /// The driver name, as far as it is printable.
    pub fn driver_name(&self) -> &str {
        let end = self.driver.iter().position(|&b| b == 0).unwrap_or(16);
        core::str::from_utf8(&self.driver[..end]).unwrap_or("?")
    }
}

/// `struct v4l2_fmtdesc`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct FmtDesc {
    pub index: u32,
    pub buf_type: u32,
    pub flags: u32,
    pub description: [u8; 32],
    pub pixelformat: u32,
    pub mbus_code: u32,
    pub reserved: [u32; 3],
}

impl FmtDesc {
    pub const fn at(index: u32, buf_type: u32) -> Self {
        Self {
            index,
            buf_type,
            flags: 0,
            description: [0; 32],
            pixelformat: 0,
            mbus_code: 0,
            reserved: [0; 3],
        }
    }
}

/// `struct v4l2_plane_pix_format`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct PlanePixFormat {
    pub sizeimage: u32,
    pub bytesperline: u32,
    pub reserved: [u16; 6],
}

/// `struct v4l2_pix_format_mplane`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct PixFormatMplane {
    pub width: u32,
    pub height: u32,
    pub pixelformat: u32,
    pub field: u32,
    pub colorspace: u32,
    pub plane_fmt: [PlanePixFormat; 8],
    pub num_planes: u8,
    pub flags: u8,
    pub ycbcr_enc: u8,
    pub quantization: u8,
    pub xfer_func: u8,
    pub reserved: [u8; 7],
}

impl PixFormatMplane {
    pub const fn zeroed() -> Self {
        Self {
            width: 0,
            height: 0,
            pixelformat: 0,
            field: 0,
            colorspace: 0,
            plane_fmt: [PlanePixFormat {
                sizeimage: 0,
                bytesperline: 0,
                reserved: [0; 6],
            }; 8],
            num_planes: 0,
            flags: 0,
            ycbcr_enc: 0,
            quantization: 0,
            xfer_func: 0,
            reserved: [0; 7],
        }
    }
}

/// The `fmt` union of `struct v4l2_format`, sized and aligned as the kernel's:
/// 200 bytes, alignment `usize` (the union contains pointers in arms this
/// crate never uses, so the alignment must still match).
#[repr(C)]
#[derive(Clone, Copy)]
pub union FormatUnion {
    pub pix_mp: PixFormatMplane,
    pub raw: [u8; 200],
    align: [usize; 1],
}

/// `struct v4l2_format`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Format {
    pub buf_type: u32,
    pub fmt: FormatUnion,
}

impl Format {
    pub const fn zeroed(buf_type: u32) -> Self {
        Self {
            buf_type,
            fmt: FormatUnion { raw: [0; 200] },
        }
    }

    /// The multiplanar pixel format arm — the only one decode uses.
    ///
    /// SAFETY: for the MPLANE buffer types this crate is restricted to, the
    /// kernel reads and writes exactly this arm of the union.
    pub fn pix_mp(&self) -> &PixFormatMplane {
        unsafe { &self.fmt.pix_mp }
    }

    /// Mutable access to the same arm, same reasoning.
    pub fn pix_mp_mut(&mut self) -> &mut PixFormatMplane {
        unsafe { &mut self.fmt.pix_mp }
    }
}

/// `struct v4l2_requestbuffers`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct RequestBuffers {
    pub count: u32,
    pub buf_type: u32,
    pub memory: u32,
    pub capabilities: u32,
    pub flags: u8,
    pub reserved: [u8; 3],
}

/// `struct v4l2_exportbuffer`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct ExportBuffer {
    pub buf_type: u32,
    pub index: u32,
    pub plane: u32,
    pub flags: u32,
    pub fd: i32,
    pub reserved: [u32; 11],
}

/// The `m` union of `struct v4l2_plane`. `userptr` is C `unsigned long`,
/// which sets the union's size per architecture exactly as the kernel's.
#[repr(C)]
#[derive(Clone, Copy)]
pub union PlaneMemory {
    pub mem_offset: u32,
    pub userptr: usize,
    pub fd: i32,
}

/// `struct v4l2_plane`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Plane {
    pub bytesused: u32,
    pub length: u32,
    pub m: PlaneMemory,
    pub data_offset: u32,
    pub reserved: [u32; 11],
}

impl Plane {
    pub const fn zeroed() -> Self {
        Self {
            bytesused: 0,
            length: 0,
            m: PlaneMemory { userptr: 0 },
            data_offset: 0,
            reserved: [0; 11],
        }
    }
}

/// `struct v4l2_timecode`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Timecode {
    pub tc_type: u32,
    pub flags: u32,
    pub frames: u8,
    pub seconds: u8,
    pub minutes: u8,
    pub hours: u8,
    pub userbits: [u8; 4],
}

/// The `m` union of `struct v4l2_buffer`: for MPLANE types, a pointer to the
/// plane array. The pointer is what gives the union — and so the struct — its
/// per-architecture layout.
#[repr(C)]
#[derive(Clone, Copy)]
pub union BufferMemory {
    pub offset: u32,
    pub userptr: usize,
    pub planes: *mut Plane,
    pub fd: i32,
}

/// `struct v4l2_buffer`.
#[repr(C)]
pub struct Buffer {
    pub index: u32,
    pub buf_type: u32,
    pub bytesused: u32,
    pub flags: u32,
    pub field: u32,
    pub timestamp: Timeval,
    pub timecode: Timecode,
    pub sequence: u32,
    pub memory: u32,
    pub m: BufferMemory,
    pub length: u32,
    pub reserved2: u32,
    pub request_fd: i32,
}

impl Buffer {
    /// A multiplanar buffer descriptor pointing at `planes`.
    pub fn mplane(index: u32, buf_type: u32, planes: &mut [Plane]) -> Self {
        Self {
            index,
            buf_type,
            bytesused: 0,
            flags: 0,
            field: 0,
            timestamp: Timeval::default(),
            timecode: Timecode::default(),
            sequence: 0,
            memory: MEMORY_MMAP,
            m: BufferMemory {
                planes: planes.as_mut_ptr(),
            },
            length: planes.len() as u32,
            reserved2: 0,
            request_fd: 0,
        }
    }
}

/// The transfer direction an ioctl's opcode encodes.
///
/// **The direction bits are part of the request number**, so getting them
/// wrong is not a style problem — the kernel fails the call with `ENOTTY`
/// because the number matches nothing. The first run of the probe on real
/// hardware failed on exactly this: every request had been built `_IOWR`,
/// and `VIDIOC_QUERYCAP` is `_IOR`.
#[derive(Clone, Copy)]
enum Dir {
    /// `_IOR`: the kernel fills the struct.
    R,
    /// `_IOW`: the kernel reads the struct.
    W,
    /// `_IOWR`: both.
    Rw,
}

/// One V4L2 ioctl over a `#[repr(C)]` struct.
struct Vidioc<'a, T> {
    nr: u8,
    dir: Dir,
    arg: &'a mut T,
}

// SAFETY: the opcode is built from `T`'s own size and the request's true
// direction, so the kernel reads and writes exactly the `T` the reference
// covers; the pointer is valid for the duration of the call because the
// borrow is.
unsafe impl<T> Ioctl for Vidioc<'_, T> {
    type Output = ();
    const IS_MUTATING: bool = true;

    fn opcode(&self) -> Opcode {
        match self.dir {
            Dir::R => ioctl::opcode::read::<T>(b'V', self.nr),
            Dir::W => ioctl::opcode::write::<T>(b'V', self.nr),
            Dir::Rw => ioctl::opcode::read_write::<T>(b'V', self.nr),
        }
    }

    fn as_ptr(&mut self) -> *mut core::ffi::c_void {
        (self.arg as *mut T).cast()
    }

    unsafe fn output_from_ptr(
        _out: IoctlOutput,
        _ptr: *mut core::ffi::c_void,
    ) -> rustix::io::Result<Self::Output> {
        Ok(())
    }
}

/// Issues one V4L2 ioctl, by uapi request number and direction.
fn vidioc<T>(fd: BorrowedFd<'_>, nr: u8, dir: Dir, arg: &mut T) -> Result<(), Errno> {
    // SAFETY: `Vidioc`'s contract above; `T` is one of this module's
    // `#[repr(C)]` uapi structs, matching what the request number expects.
    unsafe { ioctl::ioctl(fd, Vidioc { nr, dir, arg }) }
}

// The uapi request numbers, from videodev2.h.
const NR_QUERYCAP: u8 = 0;
const NR_ENUM_FMT: u8 = 2;
const NR_G_FMT: u8 = 4;
const NR_S_FMT: u8 = 5;
const NR_REQBUFS: u8 = 8;
const NR_QUERYBUF: u8 = 9;
const NR_QBUF: u8 = 15;
const NR_EXPBUF: u8 = 16;
const NR_DQBUF: u8 = 17;
const NR_STREAMON: u8 = 18;
const NR_STREAMOFF: u8 = 19;

/// `VIDIOC_QUERYCAP`.
pub fn querycap(fd: BorrowedFd<'_>) -> Result<Capability, Errno> {
    let mut cap = Capability::zeroed();
    vidioc(fd, NR_QUERYCAP, Dir::R, &mut cap)?;
    Ok(cap)
}

/// `VIDIOC_ENUM_FMT` at `index`; `Ok(None)` when the enumeration ends.
pub fn enum_fmt(fd: BorrowedFd<'_>, buf_type: u32, index: u32) -> Result<Option<FmtDesc>, Errno> {
    let mut desc = FmtDesc::at(index, buf_type);
    match vidioc(fd, NR_ENUM_FMT, Dir::Rw, &mut desc) {
        Ok(()) => Ok(Some(desc)),
        Err(Errno::INVAL) => Ok(None),
        Err(e) => Err(e),
    }
}

/// `VIDIOC_G_FMT`.
pub fn g_fmt(fd: BorrowedFd<'_>, buf_type: u32) -> Result<Format, Errno> {
    let mut format = Format::zeroed(buf_type);
    vidioc(fd, NR_G_FMT, Dir::Rw, &mut format)?;
    Ok(format)
}

/// `VIDIOC_S_FMT`, in place — the driver writes back what it actually set.
pub fn s_fmt(fd: BorrowedFd<'_>, format: &mut Format) -> Result<(), Errno> {
    vidioc(fd, NR_S_FMT, Dir::Rw, format)
}

/// `VIDIOC_REQBUFS`; returns how many the driver granted.
pub fn reqbufs(fd: BorrowedFd<'_>, buf_type: u32, count: u32) -> Result<u32, Errno> {
    let mut req = RequestBuffers {
        count,
        buf_type,
        memory: MEMORY_MMAP,
        ..RequestBuffers::default()
    };
    vidioc(fd, NR_REQBUFS, Dir::Rw, &mut req)?;
    Ok(req.count)
}

/// `VIDIOC_QUERYBUF`, filling `planes` with offsets and lengths.
pub fn querybuf(
    fd: BorrowedFd<'_>,
    buf_type: u32,
    index: u32,
    planes: &mut [Plane],
) -> Result<(), Errno> {
    let mut buffer = Buffer::mplane(index, buf_type, planes);
    vidioc(fd, NR_QUERYBUF, Dir::Rw, &mut buffer)
}

/// `VIDIOC_QBUF`.
pub fn qbuf(fd: BorrowedFd<'_>, buffer: &mut Buffer) -> Result<(), Errno> {
    vidioc(fd, NR_QBUF, Dir::Rw, buffer)
}

/// `VIDIOC_DQBUF`; `Ok(None)` when nothing is ready on a non-blocking fd.
pub fn dqbuf(fd: BorrowedFd<'_>, buffer: &mut Buffer) -> Result<Option<()>, Errno> {
    match vidioc(fd, NR_DQBUF, Dir::Rw, buffer) {
        Ok(()) => Ok(Some(())),
        Err(Errno::AGAIN) => Ok(None),
        Err(e) => Err(e),
    }
}

/// `VIDIOC_STREAMON`.
pub fn streamon(fd: BorrowedFd<'_>, buf_type: u32) -> Result<(), Errno> {
    let mut arg = buf_type;
    vidioc(fd, NR_STREAMON, Dir::W, &mut arg)
}

/// `VIDIOC_STREAMOFF`.
pub fn streamoff(fd: BorrowedFd<'_>, buf_type: u32) -> Result<(), Errno> {
    let mut arg = buf_type;
    vidioc(fd, NR_STREAMOFF, Dir::W, &mut arg)
}

/// `VIDIOC_EXPBUF`: a capture buffer's plane as a dmabuf fd.
pub fn expbuf(fd: BorrowedFd<'_>, buf_type: u32, index: u32, plane: u32) -> Result<i32, Errno> {
    let mut exp = ExportBuffer {
        buf_type,
        index,
        plane,
        // O_CLOEXEC | O_RDWR: the fd crosses into DRM and must not leak into
        // children.
        flags: 0o2000000 | 0o2,
        ..ExportBuffer::default()
    };
    vidioc(fd, NR_EXPBUF, Dir::Rw, &mut exp)?;
    Ok(exp.fd)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The layouts the ioctl numbers are derived from. Wrong sizes select
    /// wrong kernel handlers, so these are load-bearing — per architecture.
    #[test]
    fn the_struct_sizes_match_the_kernel_abi() {
        use core::mem::size_of;
        assert_eq!(size_of::<Capability>(), 104);
        assert_eq!(size_of::<FmtDesc>(), 64);
        assert_eq!(size_of::<PixFormatMplane>(), 192);
        assert_eq!(size_of::<Format>(), 204 + size_of::<usize>() - 4);
        assert_eq!(size_of::<RequestBuffers>(), 20);
        assert_eq!(size_of::<ExportBuffer>(), 64);
        assert_eq!(size_of::<Timecode>(), 16);
        #[cfg(target_pointer_width = "64")]
        {
            assert_eq!(size_of::<Plane>(), 64);
            assert_eq!(size_of::<Buffer>(), 88);
        }
        #[cfg(target_pointer_width = "32")]
        {
            assert_eq!(size_of::<Plane>(), 60);
            assert_eq!(size_of::<Buffer>(), 68);
        }
    }

    #[test]
    fn fourccs_spell_correctly() {
        assert_eq!(PIX_FMT_H264, 0x3436_3248); // 'H''2''6''4' little-endian
        assert_eq!(PIX_FMT_NV12, 0x3231_564E);
    }
}

// ------------------------------------------------------------------- events

/// `V4L2_EVENT_EOS`.
pub const EVENT_EOS: u32 = 2;
/// `V4L2_EVENT_SOURCE_CHANGE`: the decoder has parsed the stream's real
/// dimensions — the signal that the capture side can be negotiated.
pub const EVENT_SOURCE_CHANGE: u32 = 5;

/// `struct v4l2_event_subscription`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct EventSubscription {
    pub event_type: u32,
    pub id: u32,
    pub flags: u32,
    pub reserved: [u32; 5],
}

/// `struct v4l2_event`. The union's `u64` arm is what fixes the alignment to
/// eight on both architectures; only `event_type` is read here.
#[repr(C)]
pub struct Event {
    pub event_type: u32,
    pub u: [u64; 8],
    pub pending: u32,
    pub sequence: u32,
    pub timestamp: [isize; 2],
    pub id: u32,
    pub reserved: [u32; 8],
}

impl Event {
    pub const fn zeroed() -> Self {
        Self {
            event_type: 0,
            u: [0; 8],
            pending: 0,
            sequence: 0,
            timestamp: [0; 2],
            id: 0,
            reserved: [0; 8],
        }
    }
}

const NR_DQEVENT: u8 = 89;
const NR_SUBSCRIBE_EVENT: u8 = 90;

/// `VIDIOC_SUBSCRIBE_EVENT`.
pub fn subscribe_event(fd: BorrowedFd<'_>, event_type: u32) -> Result<(), Errno> {
    let mut sub = EventSubscription {
        event_type,
        ..EventSubscription::default()
    };
    vidioc(fd, NR_SUBSCRIBE_EVENT, Dir::W, &mut sub)
}

/// `VIDIOC_DQEVENT`; `Ok(None)` when the queue is empty.
pub fn dqevent(fd: BorrowedFd<'_>) -> Result<Option<Event>, Errno> {
    let mut event = Event::zeroed();
    match vidioc(fd, NR_DQEVENT, Dir::R, &mut event) {
        Ok(()) => Ok(Some(event)),
        Err(Errno::NOENT) => Ok(None),
        Err(e) => Err(e),
    }
}
