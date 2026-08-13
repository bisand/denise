//! Plays an `.h264` elementary stream on a DRM plane — milestone 2: decode to
//! scanout with no tree involved, over a plain cleared surface.
//!
//! ```text
//! # on the board, from a console (not under X/Wayland):
//! ffmpeg -i in.mp4 -c:v libx264 -profile:v main -pix_fmt yuv420p -an \
//!        -bsf:v h264_mp4toannexb promo.h264
//! cargo run -p denise-video --example player -- promo.h264
//! ```
//!
//! Ctrl-C exits; the surface restores the console on drop, as `denise-drm`
//! always does.

#[cfg(target_os = "linux")]
fn main() {
    use denise::{Color, Rect, Surface as _};
    use denise_drm::{DrmSurface, SurfaceConfig};
    use denise_render::Canvas;
    use denise_video::{Asset, Player, annexb::Codec};

    let path = std::env::args().nth(1).unwrap_or_else(|| {
        eprintln!("usage: player <file.h264|file.h265>");
        std::process::exit(2);
    });
    let asset = if path.ends_with(".h265") || path.ends_with(".hevc") {
        Asset {
            codec: Codec::H265,
            path: path.into(),
        }
    } else {
        Asset::h264(path)
    };

    let mut surface = DrmSurface::open(SurfaceConfig::default()).expect("open the display");
    let size = surface.size();

    // Paint the UI's buffer once: a dark ground with a hole-free border, so
    // it is visible that the video is a plane over a live surface, not a
    // takeover of it.
    {
        let mut frame = surface.acquire().expect("frame");
        let mut canvas = Canvas::new(&mut frame);
        canvas.clear(Color::from_rgb888(0x1E1E2E));
    }
    surface.present(&[Rect::from_size(size)]).expect("present");

    // The video sits centred, quarter-inset — the plane is positioned, not
    // fullscreen, to demonstrate that it composes with the UI.
    let dst = Rect::new(
        size.width as i32 / 8,
        size.height as i32 / 8,
        size.width as i32 * 3 / 4,
        size.height as i32 * 3 / 4,
    );

    let mut player =
        Player::open(&[asset], surface.card(), surface.crtc(), dst).expect("open the player");

    println!("playing; Ctrl-C to stop");
    loop {
        match player.pump(surface.card()) {
            Ok(_flipped) => {}
            Err(e) => {
                eprintln!("playback failed: {e}");
                break;
            }
        }
        // A promo loop needs no better pacing than the decoder's own: pump
        // rests briefly so a Pi Zero is not spun at 100%.
        std::thread::sleep(std::time::Duration::from_millis(4));
    }
    let _ = player.stop(surface.card());
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("DRM and V4L2 are Linux; run this on the board.");
}
