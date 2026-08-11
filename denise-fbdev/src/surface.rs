//! The framebuffer surface: map it, draw into a shadow, publish the damage.

use std::fs::OpenOptions;
use std::path::{Path, PathBuf};

use denise::{BufferAge, Frame, PixelFormat, Rect, Size, Surface, SurfaceError};
use memmap2::{MmapMut, MmapOptions};

use crate::error::FbdevError;
use crate::info::{FbInfo, PixelLayout};

/// Where framebuffer nodes live.
const DEV_DIR: &str = "/dev";
/// Where their attributes live.
const SYSFS_DIR: &str = "/sys/class/graphics";

/// A framebuffer, drawn into through a shadow buffer.
///
/// # Why a shadow buffer
///
/// The mapped framebuffer is being scanned out continuously; there is nothing to
/// flip to. Rendering into it directly would put every intermediate state on
/// screen — the clear before the redraw above all — which reads as flicker on
/// exactly the panels this backend exists for. So drawing goes to memory we own
/// and [`present`](Surface::present) copies out only the damaged rows.
///
/// That also makes the buffer age honest: the shadow persists frame to frame, so
/// it is [`BufferAge::Frames(1)`](BufferAge::Frames) and incremental repaint works
/// exactly as it does on DRM.
///
/// # What this cannot do
///
/// There is no page flip and no vsync. A copy that lands mid-scanout will tear,
/// and nothing here can prevent it — `FBIO_WAITFORVSYNC` is optional and widely
/// unimplemented. Keeping damage small keeps the tear small, which is the only
/// mitigation fbdev offers.
#[derive(Debug)]
pub struct FbdevSurface {
    map: MmapMut,
    shadow: Vec<u32>,
    info: FbInfo,
    path: PathBuf,
    /// Set until the first present, so the whole surface is published once.
    first_frame: bool,
}

impl FbdevSurface {
    /// Opens a specific node, such as `/dev/fb0`.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, FbdevError> {
        let path = path.as_ref().to_path_buf();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or(FbdevError::NoDevice)?;

        let info = read_info(name)?;

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .map_err(|source| FbdevError::Open {
                path: path.clone(),
                source,
            })?;

        // The length has to be given explicitly. A device node reports a size of
        // zero from `stat`, so letting memmap2 infer it maps nothing at all.
        let required = info.required_bytes();

        // SAFETY: a framebuffer is device memory whose extent is fixed by the mode,
        // and `required` is derived from the geometry the driver just reported, so
        // the mapping cannot outrun it. The usual hazard of mapping a file —
        // another process truncating it underneath us — does not apply to a device
        // node. Nothing else in this process maps it.
        let map =
            unsafe { MmapOptions::new().len(required).map_mut(&file) }.map_err(FbdevError::Map)?;

        if map.len() < required {
            return Err(FbdevError::TooSmall {
                required,
                actual: map.len(),
            });
        }

        Ok(Self {
            map,
            shadow: vec![0; info.size.width as usize * info.size.height as usize],
            info,
            path,
            first_frame: true,
        })
    }

    /// Opens the first framebuffer node that can be read and understood.
    pub fn open_first() -> Result<Self, FbdevError> {
        let dir = std::fs::read_dir(DEV_DIR).map_err(|source| FbdevError::Sysfs {
            path: PathBuf::from(DEV_DIR),
            source,
        })?;

        let mut nodes: Vec<PathBuf> = dir
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| {
                        name.strip_prefix("fb").is_some_and(|rest| {
                            !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit())
                        })
                    })
            })
            .collect();
        nodes.sort();

        let mut last = None;
        for path in nodes {
            match Self::open(&path) {
                Ok(surface) => return Ok(surface),
                Err(err) => last = Some(err),
            }
        }
        Err(last.unwrap_or(FbdevError::NoDevice))
    }

    /// The geometry in force.
    pub fn info(&self) -> FbInfo {
        self.info
    }

    /// The node this surface was opened from.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Copies one damaged region from the shadow into the framebuffer.
    fn publish(&mut self, region: Rect) {
        let Some(r) = region.clip_to_size(self.info.size) else {
            return;
        };

        let width = self.info.size.width as usize;
        let bpp = self.info.layout.bytes_per_pixel();
        let stride = self.info.stride_bytes as usize;
        let x0 = r.x as usize;
        let run = r.width as usize;

        for y in r.y as usize..r.bottom() as usize {
            let src = &self.shadow[y * width + x0..y * width + x0 + run];
            let start = y * stride + x0 * bpp;

            match self.info.layout {
                PixelLayout::Xrgb8888 => {
                    // Same word layout on both sides, so this is a plain copy.
                    let bytes: &[u8] = bytemuck::cast_slice(src);
                    self.map[start..start + bytes.len()].copy_from_slice(bytes);
                }
                PixelLayout::Rgb565 => {
                    let dst = &mut self.map[start..start + run * 2];
                    for (pixel, out) in src.iter().zip(dst.chunks_exact_mut(2)) {
                        out.copy_from_slice(&PixelLayout::to_rgb565(*pixel).to_le_bytes());
                    }
                }
            }
        }
    }
}

impl Surface for FbdevSurface {
    fn size(&self) -> Size {
        self.info.size
    }

    fn scale_factor(&self) -> f32 {
        1.0
    }

    fn format(&self) -> PixelFormat {
        // Always what the shadow is, whatever the panel turns out to want.
        PixelFormat::Xrgb8888
    }

    fn acquire(&mut self) -> Result<Frame<'_>, SurfaceError> {
        let size = self.info.size;
        let age = if self.first_frame {
            BufferAge::Undefined
        } else {
            // One shadow, kept between frames: exactly one frame stale.
            BufferAge::Frames(1)
        };

        Frame::new(
            &mut self.shadow,
            size,
            size.width,
            PixelFormat::Xrgb8888,
            age,
        )
    }

    fn present(&mut self, damage: &[Rect]) -> Result<(), SurfaceError> {
        if self.first_frame {
            // The framebuffer holds whatever the console left behind.
            self.first_frame = false;
            self.publish(Rect::from_size(self.info.size));
            return Ok(());
        }

        for region in damage {
            self.publish(*region);
        }
        Ok(())
    }
}

/// Reads a device's geometry from `/sys/class/graphics/<name>/`.
fn read_info(name: &str) -> Result<FbInfo, FbdevError> {
    let base = Path::new(SYSFS_DIR).join(name);
    let read = |attribute: &str| -> Result<String, FbdevError> {
        let path = base.join(attribute);
        std::fs::read_to_string(&path).map_err(|source| FbdevError::Sysfs { path, source })
    };

    // `modes` is optional: an empty one falls back to `virtual_size`.
    let modes = read("modes").unwrap_or_default();

    Ok(FbInfo::from_sysfs(
        &read("virtual_size")?,
        &modes,
        &read("stride")?,
        &read("bits_per_pixel")?,
    )?)
}
