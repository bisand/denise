#!/usr/bin/env python3
"""Set the workspace version and every sibling pin to match.

    scripts/bump-version.py 0.1.0

Thirteen lines in the root `Cargo.toml` have to move together: the one under
`[workspace.package]`, and the twelve `denise* = { version = "..." }` pins under
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

ROOT = pathlib.Path(__file__).resolve().parent.parent / "Cargo.toml"

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
    print(f"{old} -> {new}   (workspace.package.version and {pins} sibling pins)")
    print("\nNext:")
    print("  cargo check --workspace          # refresh Cargo.lock")
    print(f'  git commit -am "chore: release {new}" && git push')
    print("  # then, once CI is green on that commit:")
    print(f"  gh release create v{new} --generate-notes   # publishing it is the trigger")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
