//! How fast pointer motion actually reaches this process.
//!
//! The question this answers is "is the lag in the input or in the drawing?",
//! and it answers it by removing the drawing. Nothing here opens a DRM device,
//! sets a mode or touches a framebuffer: it blocks on the input descriptors,
//! reads what arrives, and prints numbers. If the numbers keep up with your hand
//! and stop the moment you do, input is not the problem and the delay is
//! downstream — in painting, in presenting, or in the frame loop's pacing.
//!
//! ```text
//! cargo run -p denise-evdev --example pointer -- [seconds]
//! /tmp/pointer 60
//! ```
//!
//! # The numbers
//!
//! **age** is the important one: the gap between the kernel timestamping the
//! event and this process reading it. The kernel stamps an event when the driver
//! receives it, so this covers the time spent queued — which no measurement taken
//! after the read can see. Under a millisecond means the loop is keeping up.
//! Growing while you move means it is not, and every position drawn is one the
//! user has already moved on from.
//!
//! **gap** is the time between one motion report and the next, which is the
//! *hardware's* rate and nothing to do with this program. A common USB mouse
//! reports at 125 Hz, so 8 ms is normal and not a fault. At 8 ms granularity the
//! pointer cannot be smoother than 8 ms no matter what the renderer does.
//!
//! **wakeups** should track **moves** closely. Many more wakeups than moves means
//! the loop is being woken for something it does not care about; many fewer means
//! events are arriving in batches, which shows up as a pointer that jumps.

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("this needs Linux and evdev");
}

#[cfg(target_os = "linux")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::io::Write;
    use std::os::fd::BorrowedFd;
    use std::time::{Duration, Instant};

    use denise::{ElementState, InputEvent, InputSource, KeyCode, Size};
    use denise_evdev::InputBackend;
    use rustix::event::{PollFd, PollFlags, Timespec, poll};

    let seconds: u64 = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(60)
        .clamp(1, 3600);

    // The size only scales absolute (touch) coordinates; for a relative mouse it
    // just bounds the position. Nothing is drawn at it.
    let mut input = InputBackend::open_all(Size::new(1920, 1080))?;
    for device in input.devices() {
        eprintln!("input   {}: {}", device.capabilities(), device.name());
    }
    eprintln!(
        "\nno display is opened — this measures input only\n\
         move the pointer; Escape quits, stats every second\n"
    );

    // Blocking on the descriptors is the whole point: a loop that sleeps a fixed
    // interval would add its own delay to every measurement and report it as the
    // kernel's.
    let raw_fds = input.raw_fds();
    let borrowed: Vec<BorrowedFd<'_>> = raw_fds
        .iter()
        // SAFETY: every descriptor belongs to `input`, which outlives this loop
        // and holds each device open until the process exits.
        .map(|&fd| unsafe { BorrowedFd::borrow_raw(fd) })
        .collect();
    let mut poll_fds: Vec<PollFd<'_>> = borrowed
        .iter()
        .map(|fd| PollFd::new(fd, PollFlags::IN))
        .collect();

    let started = Instant::now();
    let deadline = started + Duration::from_secs(seconds);
    let mut events = Vec::new();

    // Per-second accumulators.
    let mut ages_ms: Vec<f64> = Vec::new();
    let mut gaps_ms: Vec<f64> = Vec::new();
    let mut moves = 0u32;
    let mut wakeups = 0u32;
    let mut window = Instant::now();
    let mut last_move: Option<Instant> = None;
    let mut quit = false;

    // Totals, for the summary.
    let mut total_moves = 0u64;
    let mut worst_age_ms = 0.0f64;
    let mut all_ages_ms: Vec<f64> = Vec::new();

    while !quit && Instant::now() < deadline {
        // One second, so a quiet pointer still prints a line and the absence of
        // motion is visible rather than looking like a hang.
        let timeout = Timespec {
            tv_sec: 1,
            tv_nsec: 0,
        };
        let ready = poll(&mut poll_fds, Some(&timeout)).unwrap_or(0);
        if ready > 0 {
            wakeups += 1;
        }

        events.clear();
        input.poll(&mut events);

        for event in &events {
            match event {
                InputEvent::PointerMoved { position } => {
                    moves += 1;
                    total_moves += 1;
                    let now = Instant::now();
                    if let Some(previous) = last_move {
                        gaps_ms.push(now.duration_since(previous).as_secs_f64() * 1000.0);
                    }
                    last_move = Some(now);

                    let age_ms = input
                        .last_event_age()
                        .map(|d| d.as_secs_f64() * 1000.0)
                        .unwrap_or(0.0);
                    ages_ms.push(age_ms);
                    all_ages_ms.push(age_ms);
                    worst_age_ms = worst_age_ms.max(age_ms);

                    // Overwritten in place, so the live line does not scroll the
                    // per-second statistics off the screen.
                    print!(
                        "  pos {:>5},{:<5} age {age_ms:5.2} ms\r",
                        position.x, position.y
                    );
                    let _ = std::io::stdout().flush();
                }
                InputEvent::Key {
                    code: KeyCode::Escape,
                    state: ElementState::Down,
                    ..
                } => quit = true,
                _ => {}
            }
        }

        if window.elapsed() >= Duration::from_secs(1) {
            let elapsed = window.elapsed().as_secs_f64();
            if moves == 0 {
                println!("{:>5.1}s  idle{:60}", started.elapsed().as_secs_f64(), "");
            } else {
                println!(
                    "{:>5.1}s  moves {moves:4} ({:4.0}/s)  wakeups {wakeups:4}  \
                     age p50 {:5.2} p95 {:5.2} max {:5.2} ms  gap p50 {:5.2} ms",
                    started.elapsed().as_secs_f64(),
                    moves as f64 / elapsed,
                    percentile(&mut ages_ms, 0.50),
                    percentile(&mut ages_ms, 0.95),
                    percentile(&mut ages_ms, 1.00),
                    percentile(&mut gaps_ms, 0.50),
                );
            }
            ages_ms.clear();
            gaps_ms.clear();
            moves = 0;
            wakeups = 0;
            window = Instant::now();
        }
    }

    println!();
    if total_moves == 0 {
        eprintln!("no pointer motion was seen at all.");
        eprintln!("either no pointing device was opened — check the list above —");
        eprintln!("or this process cannot read it: reading /dev/input/event* needs");
        eprintln!("membership in the `input` group, or root.");
        return Ok(());
    }

    let p50 = percentile(&mut all_ages_ms, 0.50);
    let p95 = percentile(&mut all_ages_ms, 0.95);
    println!(
        "{total_moves} motion events over {:.1}s\nage p50 {p50:.2} ms, p95 {p95:.2} ms, max {worst_age_ms:.2} ms",
        started.elapsed().as_secs_f64()
    );
    println!();

    // The point of the exercise, spelled out, because the number alone does not
    // say which half of the system to go and look at.
    if p95 < 2.0 {
        println!("input is keeping up. Events reach this process within a couple of");
        println!("milliseconds of the kernel taking them, so a pointer that lags on");
        println!("screen is lagging somewhere after this: in painting, in presenting,");
        println!("or in how the frame loop is paced. Compare with `kiosk`, which");
        println!("measures the same thing with a display attached.");
    } else if p95 < 10.0 {
        println!("input is arriving late enough to see. Something is holding this");
        println!("process off the descriptors for milliseconds at a time — under a");
        println!("frame loop that would be the display wait, but nothing here draws,");
        println!("so suspect CPU contention or a slow USB path instead.");
    } else {
        println!("input is badly delayed, and no renderer can hide this. Events are");
        println!("sitting queued for longer than a frame before this process reads");
        println!("them, which points at the kernel side: USB polling rate, an");
        println!("overloaded CPU, or a device on a shared and saturated bus.");
    }
    Ok(())
}

/// The `q`-th percentile of `values`, which is sorted in place. `0.0` for empty.
#[cfg(target_os = "linux")]
fn percentile(values: &mut [f64], q: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
    let index = ((values.len() as f64 - 1.0) * q).round() as usize;
    values[index.min(values.len() - 1)]
}
