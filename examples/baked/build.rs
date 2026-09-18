//! Rasterises a face at build time, into `$OUT_DIR/FACE.rs`.
//!
//! A real product names its own font file here — the one it was designed
//! around, checked in beside the code. This example runs on whoever's machine
//! it is on, so it takes what `system-font` finds there, and where that is
//! nothing it bakes the built-in bitmap font instead: the program compiles
//! either way, and says which it got.

use denise_text::bake::{self, LATIN};

/// The sizes the program draws at. Anything else snaps to the nearest.
const SIZES: [u16; 3] = [14, 20, 32];

/// Latin, plus the two the sample uses that Latin-1 has not got. A character
/// that is not baked draws as the box, so the set is the program's to choose.
fn characters() -> impl Iterator<Item = char> {
    bake::chars(LATIN).chain(['—', '→'])
}

fn main() {
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").expect("a build script"));
    let written = match system_font::find() {
        Some(path) => bake::to_out_dir(&path, "FACE", &SIZES, characters()),
        None => {
            let mut bitmap = denise_text::BitmapSource::new();
            bake::bake(&mut bitmap, &SIZES, characters()).and_then(|baked| {
                let path = out.join("FACE.rs");
                std::fs::write(&path, baked.to_source("FACE"))
                    .map(|()| path)
                    .map_err(|e| e.to_string())
            })
        }
    };
    if let Err(why) = written {
        panic!("{why}");
    }
}
