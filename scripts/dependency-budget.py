#!/usr/bin/env python3
"""Hold the libraries to the external crates they are known to pull in.

    scripts/dependency-budget.py            # check, the way CI does
    scripts/dependency-budget.py --write    # accept today's tree as the budget

`cargo deny` answers "is anything in the tree *bad*" — an advisory, a licence, a
git source. It has no opinion on the tree getting *bigger*, and that is the way
a small library stops being one: nobody adds three hundred crates, somebody
bumps a dependency whose new version added four. Each of those is a build
script or a proc-macro that runs on the build machine and code that runs on the
panel, written by somebody this project has never heard of.

So the closure of every library a panel links is written down, by name, in
`dependency-budget.txt`, and this fails when the tree holds a name the file
does not. The fix for a failure is one line in that file — which is the point:
it turns an arrival into a diff that somebody reads and a commit that says why.
A name that has *left* the tree fails too, so the file cannot rot into a list
of things that used to be true.

Names, not versions: a version bump is Dependabot's business and changes
nothing about who is trusted. Every feature and every target, because a crate
behind a feature is still a crate somebody ships. Where a feature is most of a
crate's tree — text shaping is forty crates and a bitmap font is none — a
second `[crate default]` section holds what everybody gets without asking, so
that the small number is a promise too rather than a thing the big one hides.

The examples, the designer, the benches and the desktop backends built on winit
and wgpu are not budgeted. They are not small and do not claim to be.

Deliberately not a dependency: this is `cargo tree` and a set difference.
"""

import pathlib
import subprocess
import sys

REPO = pathlib.Path(__file__).resolve().parent.parent
BUDGET = REPO / "dependency-budget.txt"

HEADER = """\
# The external crates each library is allowed to pull in — every feature, every
# target, build dependencies included; `[crate default]` is the same with the
# default features only. Checked by `scripts/dependency-budget.py` in CI; see
# that file for why. A crate arriving here should come with a reason in the
# commit that adds it. `--write` regenerates the lists, not the reasons.
"""


def external_closure(section: str) -> set[str]:
    """Every non-workspace crate a `[crate]` or `[crate default]` section can compile."""
    crate, _, view = section.partition(" ")
    if view not in ("", "default"):
        sys.exit(f"{BUDGET.name}: [{section}] — the only view besides all features is `default`")
    tree = subprocess.run(
        [
            "cargo", "tree", "--locked", "-p", crate,
            *([] if view else ["--all-features"]), "--target", "all",
            "--edges", "normal,build", "--prefix", "none",
        ],
        cwd=REPO, check=True, capture_output=True, text=True,
    ).stdout
    names = set()
    for line in tree.splitlines():
        # `name v1.2.3`, `name v1.2.3 (proc-macro)`, `name v1.2.3 (/a/path) (*)`.
        # A path in parentheses is a workspace member, which is not external.
        if not line.strip() or "(/" in line or "(\\" in line or ":\\" in line:
            continue
        names.add(line.split()[0])
    return names


def read_budget() -> dict[str, set[str]]:
    budget: dict[str, set[str]] = {}
    current = None
    for raw in BUDGET.read_text().splitlines():
        line = raw.split("#", 1)[0].strip()
        if not line:
            continue
        if line.startswith("[") and line.endswith("]"):
            current = budget.setdefault(line[1:-1].strip(), set())
        elif current is None:
            sys.exit(f"{BUDGET.name}: `{line}` comes before any [crate] section")
        else:
            current.add(line)
    return budget


def write_budget(crates: list[str]) -> None:
    out = [HEADER]
    for crate in crates:
        out.append(f"[{crate}]")
        out.extend(sorted(external_closure(crate)))
        out.append("")
    BUDGET.write_text("\n".join(out))


def main() -> int:
    budget = read_budget()
    if sys.argv[1:] == ["--write"]:
        write_budget(list(budget))
        return 0
    if sys.argv[1:]:
        sys.exit(__doc__)

    failed = False
    for crate, allowed in budget.items():
        actual = external_closure(crate)
        arrived, left = sorted(actual - allowed), sorted(allowed - actual)
        print(f"{crate}: {len(actual)} external crate{'s' * (len(actual) != 1)}")
        for name in arrived:
            print(f"::error::{crate} now pulls in `{name}`, which is not in {BUDGET.name}")
        for name in left:
            print(f"::error::{crate} no longer pulls in `{name}`; take it out of {BUDGET.name}")
        failed |= bool(arrived or left)
    if failed:
        print(f"\n`cargo tree -p <crate> --all-features --target all -i <name>` says who "
              f"brought it.\nIf it belongs, add it to {BUDGET.name} and say why in the commit.")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
