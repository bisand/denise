//! `include/denise.h` and the Rust must agree, and neither generates the other.
//!
//! The header is the contract. Generating it from the Rust would mean the
//! contract silently follows every refactor, which is the opposite of what a
//! stable ABI is for; generating the Rust from the header is not a thing. So both
//! are written by hand and this test is what stops them drifting.
//!
//! It is worth more than it looks. A missing declaration is a link error the
//! first time somebody tries. A key number that differs between the two is not:
//! the host presses Enter, the field receives Home, and nothing anywhere says so.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn header() -> String {
    fs::read_to_string(crate_dir().join("include/denise.h")).expect("include/denise.h")
}

/// Every `.rs` under `src/`, concatenated. Read rather than `include_str!`ed so
/// that a new module cannot be added without this test seeing it.
fn sources() -> String {
    fn walk(dir: &Path, out: &mut String) {
        for entry in fs::read_dir(dir).expect("readable src directory") {
            let path = entry.expect("readable entry").path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push_str(&fs::read_to_string(&path).expect("readable source"));
                out.push('\n');
            }
        }
    }
    let mut out = String::new();
    walk(&crate_dir().join("src"), &mut out);
    out
}

/// The names of every `extern "C"` function the library exports.
fn exported_functions(sources: &str) -> Vec<String> {
    let mut names: Vec<String> = sources
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if !line.starts_with("pub extern \"C\" fn ")
                && !line.starts_with("pub unsafe extern \"C\" fn ")
            {
                return None;
            }
            let after = line.split("fn ").nth(1)?;
            Some(after.split('(').next()?.trim().to_owned())
        })
        .collect();
    names.sort();
    names
}

/// The names of every function the header declares.
fn declared_functions(header: &str) -> Vec<String> {
    let mut names: Vec<String> = header
        .lines()
        .filter(|line| line.trim_start().starts_with("DENISE_API"))
        .filter_map(|line| {
            let before = line.split('(').next()?;
            // `DENISE_API uint64_t denise_ui_root` -> the last word, minus any `*`
            // that belongs to the return type rather than the name.
            let name = before.split_whitespace().last()?.trim_start_matches('*');
            Some(name.to_owned())
        })
        .collect();
    names.sort();
    names
}

/// Every `#define NAME value` whose value is an integer, with any C suffix or
/// cast stripped.
fn defines(header: &str) -> BTreeMap<String, i64> {
    let mut out = BTreeMap::new();
    for line in header.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("#define ") else {
            continue;
        };
        let mut parts = rest.splitn(2, char::is_whitespace);
        let (Some(name), Some(value)) = (parts.next(), parts.next()) else {
            continue;
        };
        if let Some(value) = parse_c_integer(value) {
            out.insert(name.to_owned(), value);
        }
    }
    out
}

/// Every `NAME = value,` inside the named `typedef enum`.
fn enum_body(header: &str, name: &str) -> BTreeMap<String, i64> {
    let start = header
        .find(&format!("typedef enum {name} {{"))
        .unwrap_or_else(|| panic!("no `typedef enum {name}` in the header"));
    let end = header[start..]
        .find(&format!("}} {name};"))
        .unwrap_or_else(|| panic!("no end to `enum {name}`"))
        + start;

    let mut out = BTreeMap::new();
    for line in header[start..end].lines().skip(1) {
        let line = line.trim().trim_end_matches(',');
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if let Some(value) = parse_c_integer(value.trim()) {
            out.insert(key.trim().to_owned(), value);
        }
    }
    out
}

/// `0x41`, `16`, `-3`, `0x100u`, `((uint64_t)0)`, and any of those followed by a
/// comma and a `/* comment */`.
fn parse_c_integer(text: &str) -> Option<i64> {
    let text = text
        .split("/*")
        .next()?
        .trim()
        .trim_end_matches(',')
        .trim()
        .trim_start_matches("((uint64_t)")
        .trim_end_matches(')')
        .trim()
        .trim_end_matches('u');
    match text.strip_prefix("0x") {
        Some(hex) => i64::from_str_radix(hex, 16).ok(),
        None => text.parse().ok(),
    }
}

#[test]
fn every_export_is_declared_and_every_declaration_exists() {
    let exported = exported_functions(&sources());
    let declared = declared_functions(&header());

    assert!(!exported.is_empty(), "found no exports; the scan is broken");

    let missing: Vec<_> = exported
        .iter()
        .filter(|name| !declared.contains(name))
        .collect();
    assert!(
        missing.is_empty(),
        "exported but not in denise.h — a caller cannot reach these: {missing:?}"
    );

    let extra: Vec<_> = declared
        .iter()
        .filter(|name| !exported.contains(name))
        .collect();
    assert!(
        extra.is_empty(),
        "declared in denise.h but not exported — these link-error on first use: {extra:?}"
    );
}

#[test]
fn the_key_numbers_are_the_same_on_both_sides() {
    let header = enum_body(&header(), "DeniseKey");
    assert_eq!(
        header.len(),
        denise_ffi::keys::TABLE.len(),
        "the header and the table disagree about how many keys there are"
    );
    for &(name, value) in denise_ffi::keys::TABLE {
        let declared = header
            .get(name)
            .unwrap_or_else(|| panic!("{name} is missing from DeniseKey in denise.h"));
        assert_eq!(
            *declared, value as i64,
            "{name} is {declared:#x} in denise.h and {value:#x} in Rust"
        );
    }
}

#[test]
fn the_status_codes_are_the_same_on_both_sides() {
    use denise_ffi::*;
    let header = enum_body(&header(), "DeniseStatus");
    for (name, value) in [
        ("DENISE_OK", DENISE_OK),
        ("DENISE_ERR_NULL", DENISE_ERR_NULL),
        ("DENISE_ERR_INVALID", DENISE_ERR_INVALID),
        ("DENISE_ERR_NO_NODE", DENISE_ERR_NO_NODE),
        ("DENISE_ERR_BUFFER_TOO_SMALL", DENISE_ERR_BUFFER_TOO_SMALL),
        ("DENISE_ERR_WRONG_WIDGET", DENISE_ERR_WRONG_WIDGET),
        ("DENISE_ERR_PANIC", DENISE_ERR_PANIC),
    ] {
        assert_eq!(header.get(name), Some(&(value as i64)), "{name}");
    }
    assert_eq!(header.len(), 7, "denise.h has a status Rust does not");
}

#[test]
fn the_role_numbers_are_the_same_on_both_sides() {
    let header = enum_body(&header(), "DeniseRole");

    for &(name, value) in denise_ffi::ROLE_TABLE {
        let declared = header
            .get(name)
            .unwrap_or_else(|| panic!("{name} is missing from DeniseRole in denise.h"));
        assert_eq!(
            *declared, value as i64,
            "{name} is {declared} in denise.h and {value} in Rust"
        );
        assert!(
            denise_ffi::types::role(value).is_some(),
            "{name} names no role in Rust"
        );
    }

    // The one value in the enum that must *not* resolve. A host asking for "no
    // fill" and getting Base100 would be a silently wrong panel, not an error.
    assert_eq!(
        header.get("DENISE_ROLE_NONE"),
        Some(&(denise_ffi::DENISE_ROLE_NONE as i64))
    );
    assert_eq!(denise_ffi::types::role(denise_ffi::DENISE_ROLE_NONE), None);

    assert_eq!(
        denise_ffi::ROLE_TABLE.len(),
        denise::Role::COUNT,
        "the core has a role this ABI does not name"
    );
    assert_eq!(
        header.len(),
        denise::Role::COUNT + 1,
        "denise.h lists a different number of roles than the core has, plus NONE"
    );
}

#[test]
fn the_constants_are_the_same_on_both_sides() {
    use denise_ffi::*;
    let defines = defines(&header());
    let expected: [(&str, i64); 16] = [
        ("DENISE_ABI_VERSION", DENISE_ABI_VERSION as i64),
        ("DENISE_NODE_NONE", 0),
        ("DENISE_MAX_DAMAGE_RECTS", denise::MAX_DAMAGE_RECTS as i64),
        ("DENISE_THEME_LIGHT", DENISE_THEME_LIGHT as i64),
        ("DENISE_THEME_DARK", DENISE_THEME_DARK as i64),
        (
            "DENISE_THEME_HIGH_CONTRAST",
            DENISE_THEME_HIGH_CONTRAST as i64,
        ),
        ("DENISE_FORMAT_ARGB8888", DENISE_FORMAT_ARGB8888 as i64),
        ("DENISE_FORMAT_XRGB8888", DENISE_FORMAT_XRGB8888 as i64),
        ("DENISE_BUTTON_LEFT", DENISE_BUTTON_LEFT as i64),
        ("DENISE_BUTTON_RIGHT", DENISE_BUTTON_RIGHT as i64),
        ("DENISE_BUTTON_MIDDLE", DENISE_BUTTON_MIDDLE as i64),
        ("DENISE_BUTTON_OTHER", DENISE_BUTTON_OTHER as i64),
        ("DENISE_MOD_SHIFT", DENISE_MOD_SHIFT as i64),
        ("DENISE_MOD_CTRL", DENISE_MOD_CTRL as i64),
        ("DENISE_MOD_ALT", DENISE_MOD_ALT as i64),
        ("DENISE_MOD_SUPER", DENISE_MOD_SUPER as i64),
    ];
    for (name, value) in expected {
        assert_eq!(
            defines.get(name),
            Some(&value),
            "{name} differs between denise.h and Rust"
        );
    }

    // The touch phases are declared as `#define`s in the header and consts in
    // Rust, and are the numbers `denise_ui_touch` switches on.
    for (name, value) in [
        ("DENISE_TOUCH_DOWN", denise_ffi::input::DENISE_TOUCH_DOWN),
        ("DENISE_TOUCH_MOVED", denise_ffi::input::DENISE_TOUCH_MOVED),
        ("DENISE_TOUCH_UP", denise_ffi::input::DENISE_TOUCH_UP),
        (
            "DENISE_TOUCH_CANCELLED",
            denise_ffi::input::DENISE_TOUCH_CANCELLED,
        ),
    ] {
        assert_eq!(defines.get(name), Some(&(value as i64)), "{name}");
    }

    assert_eq!(
        defines.get("DENISE_KEY_UNIDENTIFIED"),
        Some(&(denise_ffi::keys::UNIDENTIFIED as i64))
    );
}
