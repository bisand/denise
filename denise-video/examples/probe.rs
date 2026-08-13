//! Lists what this board's hardware decodes — milestone 1 of the video work,
//! and the same enumeration [`Decoders::detect`] runs at runtime.
//!
//! ```text
//! cargo run -p denise-video --example probe
//! ```
//!
//! On a Pi 4 expect two rows: `bcm2835-codec` (H.264, stateful — the path
//! this crate drives today) and `rpivid` (HEVC, stateless — detected and
//! reported, decode tracked separately). On a Pi 5 only `rpivid`; on a Pi
//! Zero through 3, only `bcm2835-codec`.

#[cfg(target_os = "linux")]
fn main() {
    use denise_video::{Asset, Decoders};

    let decoders = Decoders::detect();
    if decoders.found.is_empty() {
        println!("no V4L2 memory-to-memory decoders found");
        println!("(is this a board with a stateful decoder, and is /dev/video* readable?)");
        return;
    }
    println!(
        "{:<16} {:<18} {:<6} {:<6} path",
        "driver", "kind", "H.264", "HEVC"
    );
    for d in &decoders.found {
        println!(
            "{:<16} {:<18} {:<6} {:<6} {}",
            d.driver,
            if d.stateful {
                "stateful"
            } else {
                "stateless (todo)"
            },
            if d.h264 { "yes" } else { "-" },
            if d.hevc { "yes" } else { "-" },
            d.path.display(),
        );
    }

    // The menu's rule, applied to a hypothetical kiosk shipping both files.
    let assets = [Asset::h264("promo.h264"), Asset::h265("promo.h265")];
    match decoders.pick(&assets) {
        Some((asset, node)) => println!(
            "\nthis board would play {} via {}",
            asset.path.display(),
            node.path.display()
        ),
        None => println!("\nno stateful decoder for either menu codec on this board"),
    }
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("V4L2 is Linux; run this on the board.");
}
