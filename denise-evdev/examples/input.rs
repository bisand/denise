//! Prints translated input events for a few seconds.
//!
//! Read-only and display-free: it touches no DRM device and sets no mode, so it is
//! safe to run over SSH while looking at the console.
//!
//! ```text
//! cargo run -p denise-evdev --example input -- [seconds]
//! ```

#[cfg(target_os = "linux")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::time::{Duration, Instant};

    use denise::{InputSource, Size};
    use denise_evdev::InputBackend;

    let seconds: u64 = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(5)
        .clamp(1, 120);

    // Pretend a panel, so absolute devices have something to map onto.
    let surface = Size::new(1280, 800);
    let mut input = InputBackend::open_all(surface)?;

    eprintln!("surface {}x{}", surface.width, surface.height);
    for device in input.devices() {
        let (ax, ay) = device.abs_ranges();
        let calibration = match (ax, ay) {
            (Some(x), Some(y)) => {
                format!("  abs x {}..{}, y {}..{}", x.min, x.max, y.min, y.max)
            }
            _ => String::new(),
        };
        eprintln!(
            "  {:?}: {} ({}){calibration}",
            device.kind(),
            device.name(),
            device.path().display()
        );
    }
    eprintln!("\nlistening for {seconds}s — move the mouse, type, click\n");

    let deadline = Instant::now() + Duration::from_secs(seconds);
    let mut events = Vec::new();
    let mut total = 0usize;

    while Instant::now() < deadline {
        events.clear();
        input.poll(&mut events);
        for event in &events {
            total += 1;
            eprintln!("  {event:?}");
        }
        if events.is_empty() {
            // A real loop waits on the descriptors instead. This is a probe.
            std::thread::sleep(Duration::from_millis(4));
        }
    }

    eprintln!("\n{total} events, pointer ended at {:?}", input.pointer());
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("denise-evdev only does anything on Linux");
}
