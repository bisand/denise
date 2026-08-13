//! Finds a text face on whatever machine an example is running on.
//!
//! ```no_run
//! match system_font::load(None) {
//!     Some((name, source)) => { eprintln!("using {name}"); }
//!     None => eprintln!("no TrueType font found; using the built-in 8x8 bitmap font"),
//! }
//! ```
//!
//! Not part of the toolkit and deliberately not published. A panel ships the one
//! face it was designed around, usually embedded in the binary — it does not go
//! looking, because what it finds on the machine is not what the layout was
//! measured against. **The examples are the exception**: they run on the
//! reader's machine, and asking them to pass `--font` before anything looks
//! right is a poor greeting.
//!
//! This lives in its own crate because two examples wanted it and a copy in each
//! is a list of directories that drifts. The list is the part that goes wrong;
//! see [`pick`]'s tests.

use std::path::{Path, PathBuf};

use denise_text::{GlyphSource, TrueTypeSource};

/// Directories systems keep fonts in.
///
/// Directories, not files. The first version listed full paths and missed
/// Alpine, which puts DejaVu in `/usr/share/fonts/dejavu/` where Debian uses
/// `/usr/share/fonts/truetype/dejavu/` and Arch uses `/usr/share/fonts/TTF/`.
/// Guessing one more path would have been wrong on the next distribution too,
/// so this looks instead.
pub const FONT_DIRS: &[&str] = &[
    "/usr/share/fonts",
    "/usr/local/share/fonts",
    "/System/Library/Fonts",
    "/System/Library/Fonts/Supplemental",
    "/Library/Fonts",
    "C:\\Windows\\Fonts",
];

/// Faces worth having, best first.
///
/// A regular upright sans, because that is what a grid of records wants.
/// Nothing here is required — it is the order of preference among whatever
/// turns up.
pub const PREFERRED: &[&str] = &[
    "DejaVuSans.ttf",
    "LiberationSans-Regular.ttf",
    "NotoSans-Regular.ttf",
    "Inter-Regular.ttf",
    "segoeui.ttf",
    "Arial.ttf",
    "Helvetica.ttc",
];

/// Loads the named font, or the best one this machine has.
///
/// Returns the name it settled on alongside the face, because an example that
/// says which file it drew with can be told apart from one that quietly fell
/// back to the bitmap font.
pub fn load(requested: Option<&str>) -> Option<(String, Box<dyn GlyphSource>)> {
    let path = match requested {
        Some(path) => PathBuf::from(path),
        None => match find() {
            Some(found) => found,
            None => {
                eprintln!("no font found under {}", FONT_DIRS.join(", "));
                return None;
            }
        },
    };

    let name = path.display().to_string();
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) => {
            eprintln!("{name}: {e}");
            return None;
        }
    };
    match TrueTypeSource::from_bytes(&name, &bytes) {
        Ok(source) => Some((name, Box::new(source))),
        // Says *why*, rather than falling back in silence: a font that is
        // present and unreadable is a different problem from one that is not
        // there, and only one of them is fixed by installing something.
        Err(why) => {
            eprintln!("{name}: {why}");
            None
        }
    }
}

/// The best face under [`FONT_DIRS`], or `None` if the machine has none.
pub fn find() -> Option<PathBuf> {
    let mut found = Vec::new();
    for dir in FONT_DIRS {
        collect(Path::new(dir), 0, &mut found);
    }
    pick(&found).cloned()
}

/// Every font file under `dir`, to a bounded depth.
///
/// Bounded because a font directory is somebody else's, and following it without
/// a limit is how a panel spends its startup walking a symlink loop.
fn collect(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > 3 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, depth + 1, out);
        } else if matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("ttf" | "otf" | "ttc")
        ) {
            out.push(path);
        }
    }
}

/// Chooses among the faces that were found.
///
/// Separated from the walking so it can be tested: the choosing is the part that
/// goes wrong, and picking `DejaVuSans-BoldOblique` over `DejaVuSans` is exactly
/// the sort of wrong that nobody notices until they see a screenshot.
pub fn pick(found: &[PathBuf]) -> Option<&PathBuf> {
    let name_of = |p: &PathBuf| {
        p.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase()
    };

    for wanted in PREFERRED {
        let wanted = wanted.to_ascii_lowercase();
        if let Some(hit) = found.iter().find(|p| name_of(p) == wanted) {
            return Some(hit);
        }
    }

    // Nothing preferred, so anything upright and proportional. A grid set in
    // Bold Italic Condensed is worse than one set in the bitmap font.
    found.iter().find(|p| {
        let name = name_of(p);
        !["bold", "italic", "oblique", "condensed", "mono", "light"]
            .iter()
            .any(|bad| name.contains(bad))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(names: &[&str]) -> Vec<PathBuf> {
        names.iter().map(PathBuf::from).collect()
    }

    /// The bug this replaced: hardcoded full paths that knew Debian's layout and
    /// Arch's, and not Alpine's. Every one of these is the same face on a
    /// different distribution, and all three have to be found.
    #[test]
    fn dejavu_is_found_wherever_a_distribution_puts_it() {
        for dir in [
            "/usr/share/fonts/dejavu",          // Alpine
            "/usr/share/fonts/truetype/dejavu", // Debian, Raspberry Pi OS
            "/usr/share/fonts/TTF",             // Arch
        ] {
            let found = paths(&[
                &format!("{dir}/DejaVuSerif.ttf"),
                &format!("{dir}/DejaVuSans.ttf"),
            ]);
            assert_eq!(
                pick(&found).map(|p| p.file_name().unwrap().to_str().unwrap()),
                Some("DejaVuSans.ttf"),
                "not found under {dir}"
            );
        }
    }

    /// A directory of one face is mostly its variants, and the plain one is
    /// usually not first. Alpine's dejavu directory lists ExtraLight before Sans.
    #[test]
    fn the_plain_face_wins_over_its_variants() {
        let found = paths(&[
            "/usr/share/fonts/dejavu/DejaVuSans-ExtraLight.ttf",
            "/usr/share/fonts/dejavu/DejaVuSansCondensed-BoldOblique.ttf",
            "/usr/share/fonts/dejavu/DejaVuSans-Bold.ttf",
            "/usr/share/fonts/dejavu/DejaVuSans.ttf",
        ]);
        assert_eq!(
            pick(&found).map(|p| p.file_name().unwrap().to_str().unwrap()),
            Some("DejaVuSans.ttf")
        );
    }

    /// Preference order, not directory order: a machine with both should get the
    /// one that reads better in a data grid.
    #[test]
    fn preference_beats_whatever_was_listed_first() {
        let found = paths(&["/x/Arial.ttf", "/x/DejaVuSans.ttf"]);
        assert_eq!(
            pick(&found).map(|p| p.file_name().unwrap().to_str().unwrap()),
            Some("DejaVuSans.ttf")
        );
    }

    /// Nothing preferred is not nothing usable — but a face that is only
    /// available bold and italic is worse than the built-in bitmap font, so it is
    /// refused and the caller falls back.
    #[test]
    fn an_unknown_upright_face_is_taken_and_a_slanted_one_is_not() {
        assert_eq!(
            pick(&paths(&["/x/SomethingElse.ttf"])).map(|p| p.display().to_string()),
            Some("/x/SomethingElse.ttf".to_string())
        );
        assert_eq!(pick(&paths(&["/x/Whatever-BoldItalic.ttf"])), None);
        assert_eq!(pick(&[]), None);
    }
}
