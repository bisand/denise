#!/usr/bin/env python3
"""Set the workspace version and every sibling pin to match.

    scripts/bump-version.py 0.1.0

A line per crate in the root `Cargo.toml` has to move with the one under
`[workspace.package]`: every `denise* = { version = "..." }` pin under
`[workspace.dependencies]`. Editing them by hand works right up until it does
not, and the failure is the quiet kind — a pin left behind publishes a crate
requiring a sibling nobody built it against, and no local build can notice
because locally the `path` wins.

CI asserts the invariant either way (`the workspace versions agree`). This is
just the thing that makes getting it right the default rather than the careful
option.

Deliberately not a dependency. `cargo set-version` from `cargo-edit` does this
and more, and this is forty lines of `re` against one file we control.
"""

import pathlib
import re
import sys

REPO = pathlib.Path(__file__).resolve().parent.parent
ROOT = REPO / "Cargo.toml"
README = REPO / "README.md"

# `0.1.0`, `1.0.0-rc.1`. Not a full semver grammar — enough to refuse a typo.
VERSION = re.compile(r"^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$")


def main(argv: list[str]) -> int:
    if len(argv) != 2 or not VERSION.match(argv[1]):
        print(f"usage: {argv[0]} <version>   e.g. 0.1.0", file=sys.stderr)
        return 2
    new = argv[1]

    text = ROOT.read_text()

    # The workspace version, which is the one every crate inherits.
    package = re.search(
        r"(\[workspace\.package\][^\[]*?\nversion = \")([^\"]+)(\")", text, re.S
    )
    if not package:
        print("no version under [workspace.package]", file=sys.stderr)
        return 1
    old = package.group(2)
    if old == new:
        print(f"already at {new}")
        return 0
    text = text[: package.start(2)] + new + text[package.end(2) :]

    # The sibling pins. Anchored to a line starting with a `denise` crate name so
    # that a third-party dependency which happens to be at the same version is
    # left alone — `windows = "0.62"` must not move because we bumped to 0.62.
    text, pins = re.subn(
        r'(?m)^(denise[a-z0-9-]* = \{ version = ")' + re.escape(old) + r'(")',
        r"\g<1>" + new + r"\g<2>",
        text,
    )

    ROOT.write_text(text)

    # The README's install snippet pins major.minor. Anchored to lines of the
    # form `denise-x = "0.1"` (commented or not) so nothing else in the file can
    # match. `\d+\.\d+` rather than the old value, because the snippet is
    # major.minor while the manifest is major.minor.patch — a patch release
    # leaves it alone by producing the same text.
    minor = ".".join(new.split(".")[:2])
    readme = README.read_text()
    readme, snippets = re.subn(
        r'(?m)^(#?\s*denise[a-z0-9-]* = ")\d+\.\d+(")',
        r"\g<1>" + minor + r"\g<2>",
        readme,
    )
    README.write_text(readme)

    print(
        f"{old} -> {new}   (workspace.package.version, {pins} sibling pins, "
        f"{snippets} README lines)"
    )
    print("\nRemember: cargo update --workspace   # refresh Cargo.lock")
    print(f"Releasing is just: gh release create v{new} --notes '...'")
    print("The Release workflow sets the version from the tag by itself; running")
    print("this script first is only for landing the bump as your own commit.")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
