//! Reports what the DRM device offers and what Denise would choose.
//!
//! Read-only: it opens the card, enumerates connectors and runs the selection
//! policy, but sets no mode and takes no master lock. Safe to run over SSH on a
//! machine whose console you would rather not black out.
//!
//! ```text
//! cargo run -p denise-drm --example probe
//! ```

#[cfg(target_os = "linux")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use denise_drm::{Card, ModePreference, OutputPreference, mode};

    let card = Card::open_first()?;
    println!(
        "device: {}",
        card.path()
            .map_or("<inherited fd>".into(), |p| p.display().to_string())
    );

    let (handles, connectors) = card.connectors()?;
    println!("connectors: {}", connectors.len());

    for (i, c) in connectors.iter().enumerate() {
        println!(
            "\n  [{i}] id={} {:?} {} — {} modes",
            c.id,
            c.kind,
            if c.connected {
                "connected"
            } else {
                "disconnected"
            },
            c.modes.len()
        );
        for (m, mode) in c.modes.iter().enumerate() {
            println!(
                "        {m:>2}: {mode}{}",
                if mode.preferred { "  [preferred]" } else { "" }
            );
        }
        if c.connected {
            match card.crtc_for(handles[i]) {
                Ok(crtc) => println!("        crtc: {}", u32::from(crtc)),
                Err(err) => println!("        crtc: none ({err})"),
            }
        }
    }

    println!("\nselection:");
    for pref in [
        ModePreference::Preferred,
        ModePreference::Largest,
        ModePreference::Exact {
            width: 1280,
            height: 720,
        },
    ] {
        match mode::select(&connectors, OutputPreference::Auto, pref) {
            Ok(s) => {
                let c = &connectors[s.connector];
                println!(
                    "  {pref:?} -> connector {} ({:?}) mode {}{}",
                    c.id,
                    c.kind,
                    c.modes[s.mode],
                    if s.fell_back { "  [fell back]" } else { "" }
                );
            }
            Err(err) => println!("  {pref:?} -> {err}"),
        }
    }

    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("denise-drm only does anything on Linux");
}
