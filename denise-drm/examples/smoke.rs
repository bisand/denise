//! Drives a real display through the whole pipeline for a few seconds.
//!
//! Takes DRM master, sets a mode, and page-flips a themed scene with damage
//! tracking, then restores the console. There is a hard time limit so it always
//! gives the display back — a smoke test that can strand a VT is not a smoke test.
//!
//! ```text
//! cargo run -p denise-drm --example smoke -- [seconds]
//! ```

#[cfg(target_os = "linux")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::time::{Duration, Instant};

    use denise::{
        Color, DamageTracker, MAX_DAMAGE_RECTS, Radius, Rect, Role, Surface, Theme, theme,
    };
    use denise_drm::{DrmSurface, SurfaceConfig};
    use denise_render::Canvas;

    let seconds: u64 = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(6)
        .clamp(1, 60);

    let mut surface = DrmSurface::open(SurfaceConfig::default())?;
    let size = surface.size();
    eprintln!(
        "mode {} — {} buffers, stride {} px for {} px of width",
        surface.mode_name(),
        surface.buffer_count(),
        surface.stride(),
        size.width
    );

    let theme: Theme = theme::DARK;
    let mut tracker = DamageTracker::new(size);

    // A card sliding across the panel. What matters is not the animation but that
    // only the vacated and newly covered strips are ever repainted.
    let card_w = (size.width as i32 / 3).max(120);
    let card_h = (size.height as i32 / 4).max(90);
    let mut card = Rect::new(20, (size.height as i32 - card_h) / 2, card_w, card_h);
    let mut velocity = 7;

    let deadline = Instant::now() + Duration::from_secs(seconds);
    let mut frames = 0u64;
    let mut damaged = 0u64;
    let started = Instant::now();

    while Instant::now() < deadline {
        // Advance, marking both the old and new positions dirty.
        let bounds = Rect::from_size(size);
        if card.x + velocity < 0 || card.right() + velocity > bounds.width {
            velocity = -velocity;
        }
        tracker.add(card);
        card = card.translate(velocity, 0);
        tracker.add(card);

        let mut frame = surface.acquire()?;

        let mut regions = [Rect::ZERO; MAX_DAMAGE_RECTS];
        let count = {
            let resolved = tracker.resolve(frame.age());
            regions[..resolved.len()].copy_from_slice(resolved);
            resolved.len()
        };
        let damage = &regions[..count];
        damaged += damage.iter().map(Rect::area).sum::<u64>();

        {
            let mut canvas = Canvas::new(&mut frame);
            for region in damage {
                let mut c = canvas.with_clip(*region);
                c.clear(theme.color(Role::Base100));
                c.fill_rounded_rect(card, theme.radius(Radius::Box), theme.color(Role::Primary));
                c.stroke_rounded_rect(
                    card,
                    theme.radius(Radius::Box),
                    2,
                    theme.color(Role::Base300),
                );
                // A strip of the accent colour, to prove alpha blending survives
                // the trip through a real scanout buffer.
                c.fill_rect(
                    Rect::new(card.x + 16, card.y + 16, card.width - 32, 24),
                    Color::rgba(
                        theme.color(Role::Accent).r,
                        theme.color(Role::Accent).g,
                        theme.color(Role::Accent).b,
                        150,
                    ),
                );
            }
        }

        drop(frame);
        surface.present(damage)?;
        tracker.end_frame();
        frames += 1;
    }

    let elapsed = started.elapsed().as_secs_f64();
    let total = size.area() * frames;
    eprintln!(
        "{frames} frames in {elapsed:.2}s = {:.1} fps",
        frames as f64 / elapsed
    );
    eprintln!(
        "repainted {:.2}% of the surface per frame on average",
        if total == 0 {
            0.0
        } else {
            damaged as f64 / total as f64 * 100.0
        }
    );

    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("denise-drm only does anything on Linux");
}
