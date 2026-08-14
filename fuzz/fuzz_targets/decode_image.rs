//! Anything at all, through the front door of `denise-image`.
//!
//! `decode` picks the format from the magic bytes, so this one target reaches
//! the PNG, JPEG, GIF and BMP paths. Three of those delegate to crates that are
//! fuzzed upstream; **the BMP decoder is this workspace's own**, about a hundred
//! lines of offsets and strides, and it is the reason this target exists.
//!
//! A panic is a finding. There is no `unsafe` in the crate, so what this hunts
//! is a denial of service: a panel that dies on a malformed asset is a panel
//! that dies in the field.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(picture) = denise_image::decode(data) {
        // The invariant every decoder is now routed through. Asserting it here
        // as well means the fuzzer hunts for a way past `Picture::checked`, not
        // merely for a crash inside it.
        let size = picture.size();
        assert_eq!(
            picture.pixels().len(),
            size.width as usize * size.height as usize,
            "a decoded picture disagreed with its own size",
        );
    }
});
