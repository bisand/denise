//! Benchmarks that run on the hardware, not on the developer's desk.
//!
//! Criterion is the better harness and stays the one CI uses, but it drags in a
//! build script that needs a C cross-compiler, which makes it useless for exactly
//! the machine whose numbers matter. This has no dependencies at all, so it
//! cross-compiles to a static binary and runs anywhere.
//!
//! It reports the same workloads as `benches/damage.rs` so the two can be
//! compared, and the number to look at is the last one: what a frame costs when
//! only a little changed, against what it costs to redraw everything.
//!
//! ```text
//! cargo build -p denise-benches --bin on-target --release --target <triple>
//! ```

use std::hint::black_box;
use std::time::{Duration, Instant};

use denise::{Color, Point, Rect, Size};
use denise_benches::{Target, paint_scene, scene, typical_damage};

/// Time one operation, returning the best of several runs.
///
/// The minimum rather than the mean: the workload is deterministic, so everything
/// above the floor is scheduler noise, and on a single-core-ish Pi under an SSH
/// session there is plenty of that.
fn measure(label: &str, pixels: u64, mut op: impl FnMut()) -> Timing {
    // Warm up, so the first run does not pay for cold caches and lazy paging.
    for _ in 0..3 {
        op();
    }

    let mut best = Duration::MAX;
    let mut runs = 0;
    let deadline = Instant::now() + Duration::from_millis(600);

    while Instant::now() < deadline || runs < 5 {
        let start = Instant::now();
        op();
        best = best.min(start.elapsed());
        runs += 1;
        if runs >= 200 {
            break;
        }
    }

    Timing {
        label: label.to_owned(),
        elapsed: best,
        pixels,
    }
}

struct Timing {
    label: String,
    elapsed: Duration,
    pixels: u64,
}

impl Timing {
    fn report(&self) {
        let micros = self.elapsed.as_secs_f64() * 1e6;
        let mpx = if self.elapsed.is_zero() {
            0.0
        } else {
            self.pixels as f64 / self.elapsed.as_secs_f64() / 1e6
        };
        // A 60 Hz frame is 16.67 ms; how much of it did this cost?
        let budget = micros / 16_667.0 * 100.0;
        println!(
            "  {:<28} {:>9.1} us  {:>8.1} Mpx/s  {:>6.1}% of a 60 Hz frame",
            self.label, micros, mpx, budget
        );
    }
}

fn main() {
    // Match the panel this is running on where possible; fall back to a 7" Pi
    // touchscreen, which is what most of these actually ship with.
    let size: Size = std::env::args()
        .nth(1)
        .and_then(|arg| {
            let (w, h) = arg.split_once('x')?;
            Some(Size::new(w.parse().ok()?, h.parse().ok()?))
        })
        .unwrap_or(Size::new(800, 480));

    println!(
        "denise on-target benchmarks — {}x{} ({:.2} Mpx)\n",
        size.width,
        size.height,
        size.area() as f64 / 1e6
    );

    let mut target = Target::new(size);
    let full = Rect::from_size(size);

    println!("primitives");
    measure("clear", size.area(), || {
        target
            .canvas()
            .clear(black_box(Color::from_rgb888(0x1E1E2E)))
    })
    .report();

    measure("fill_rect opaque", full.area(), || {
        target
            .canvas()
            .fill_rect(black_box(full), Color::from_rgb888(0x313244))
    })
    .report();

    measure("fill_rect alpha", full.area(), || {
        target
            .canvas()
            .fill_rect(black_box(full), Color::rgba(0, 0, 0, 128))
    })
    .report();

    measure("rounded_rect fill r=12", full.area(), || {
        target
            .canvas()
            .fill_rounded_rect(black_box(full), 12, Color::from_rgb888(0x313244))
    })
    .report();

    // A stroke touches its band, not its bounding box. Reporting throughput
    // against the box would flatter it by two orders of magnitude.
    let band = 2 * (size.width as u64 + size.height as u64) * 2;
    measure("rounded_rect stroke r=12", band, || {
        target
            .canvas()
            .stroke_rounded_rect(black_box(full), 12, 2, Color::from_rgb888(0x89B4FA))
    })
    .report();

    // Wu blends two pixels per step along the major axis.
    let along = size.width.max(size.height) as u64 * 2;
    measure("line antialiased", along, || {
        target.canvas().draw_line(
            black_box(Point::new(0, 0)),
            black_box(Point::new(size.width as i32 - 1, size.height as i32 - 1)),
            Color::WHITE,
        )
    })
    .report();

    println!("\nwhole frames");
    let widgets = scene(size);
    let damage = typical_damage(size);
    let damaged_pixels: u64 = damage.iter().map(Rect::area).sum();

    let full_frame = measure("scene, full repaint", size.area(), || {
        paint_scene(&mut target.canvas(), black_box(&widgets))
    });
    full_frame.report();

    let damaged_frame = measure("scene, damage only", damaged_pixels, || {
        let mut canvas = target.canvas();
        for region in black_box(&damage) {
            let mut clipped = canvas.with_clip(*region);
            paint_scene(&mut clipped, &widgets);
        }
    });
    damaged_frame.report();

    let ratio = full_frame.elapsed.as_secs_f64() / damaged_frame.elapsed.as_secs_f64().max(1e-12);
    let area = damaged_pixels as f64 / size.area() as f64 * 100.0;
    println!(
        "\ndamage covers {area:.2}% of the surface and costs {:.1}x less than a full \
         repaint\nat 60 Hz that frame uses {:.1}% of one core, against {:.1}% redrawing everything",
        ratio,
        damaged_frame.elapsed.as_secs_f64() * 60.0 * 100.0,
        full_frame.elapsed.as_secs_f64() * 60.0 * 100.0,
    );
}
