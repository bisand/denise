//! Draws to `/dev/fb0` for a few seconds through the damage path.
//!
//! ```text
//! cargo run -p denise-fbdev --example smoke -- [seconds]
//! ```

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("denise-fbdev only does anything on Linux");
}

#[cfg(target_os = "linux")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::time::{Duration, Instant};

    use denise::{DamageTracker, MAX_DAMAGE_RECTS, Radius, Rect, Role, Surface, Theme, theme};
    use denise_fbdev::FbdevSurface;
    use denise_render::Canvas;

    let seconds: u64 = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(4)
        .clamp(1, 60);

    let mut surface = FbdevSurface::open_first()?;
    let size = surface.size();
    eprintln!("{} — {}", surface.path().display(), surface.info());

    let active: Theme = theme::DARK;
    let mut tracker = DamageTracker::new(size);

    let card_w = (size.width as i32 / 3).max(120);
    let card_h = (size.height as i32 / 4).max(90);
    let mut card = Rect::new(20, (size.height as i32 - card_h) / 2, card_w, card_h);
    let mut velocity = 6;

    let deadline = Instant::now() + Duration::from_secs(seconds);
    let started = Instant::now();
    let mut frames = 0u64;
    let mut damaged = 0u64;

    while Instant::now() < deadline {
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
                c.clear(active.color(Role::Base100));
                c.fill_rounded_rect(
                    card,
                    active.radius(Radius::Box),
                    active.color(Role::Primary),
                );
                c.stroke_rounded_rect(
                    card,
                    active.radius(Radius::Box),
                    2,
                    active.color(Role::Base300),
                );
            }
        }

        drop(frame);
        surface.present(damage)?;
        tracker.end_frame();
        frames += 1;

        // No vsync to wait on, so pace by hand or this spins.
        std::thread::sleep(Duration::from_millis(16));
    }

    let elapsed = started.elapsed().as_secs_f64();
    eprintln!(
        "{frames} frames in {elapsed:.2}s = {:.1} fps, {:.2}% of the surface repainted per frame",
        frames as f64 / elapsed,
        damaged as f64 / (size.area() * frames.max(1)) as f64 * 100.0
    );
    Ok(())
}
