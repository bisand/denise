# Releasing

All thirteen crates share one version and go to crates.io together.

## Doing it

Create a GitHub release. That is the whole of it:

```bash
gh release create v0.3.0 --title v0.3.0 --generate-notes
```

or the web UI's *Draft a new release* — a draft is the better habit, since it
lets the notes be written properly and nothing happens until it is published.

**The tag is the version.** On publish, `.github/workflows/release.yml` does
everything the number touches:

1. Writes it everywhere it lives — the workspace version, the sibling
   pins, the README's install snippet, `Cargo.lock` — as a commit on top of the
   released tree (`scripts/bump-version.py`, the one place that knows where
   version numbers live).
2. **Moves the tag onto that commit**, so the release page and crates.io
   describe the same tree — the invariant holds by construction, not by
   discipline. main is fast-forwarded to the bump when it can be.
3. Dispatches CI onto the commit and **waits** for the verdict — the commit was
   born seconds ago, so demanding it had already passed would fail every
   release. Red CI still stops everything, before anything is uploaded.
4. Rehearses with a full `--dry-run`, then publishes, and lists what went out
   in the run summary.

A release cut on a tree whose manifests already carry the tag's version — a
manual `bump-version.py` commit, or a re-run — skips 1 and 2 and just verifies
and publishes.

### When a release fails partway

Nothing is uploaded until every guard has passed, so a red run leaves crates.io
untouched and the release visible with a red workflow beside it — that is the
designed failure state. If the cause was red CI or a bad tree: fix main, then
**delete the release and its tag** and release again from the fixed tree.
Re-publishing the same release (draft-toggle) re-runs the workflow against its
existing tag, which is only right when the tree it points at was fine and the
failure was transient.

To rehearse without publishing anything, run the **Release** workflow by hand
from the Actions tab. With no release attached it stops after the dry run.

### Why the release comes first

The usual arrangement is a tag that triggers a publish that then opens a release. This is the other way round, and deliberately: the release exists and describes what is about to go out, then crates.io follows.

The reason is what each failure leaves behind. If the publish fails here, the release stays visible with a red workflow beside it — a thing with a name, notes and a URL, that plainly did not finish. Tag-first leaves a tag nobody can explain and no page to hang the explanation on. Given the upload cannot be undone, the arrangement where a half-finished release is legible is the better one.

It also means the notes are written by a person before the irreversible step rather than generated after it.

## Why lockstep

Because the version number is worth more as a compatibility statement than as a change log. Under independent versioning you get `denise-ui 0.3.1` requiring `denise-render 0.2.4` within about three releases, and from then on "which versions work together" needs a table somebody maintains and readers have to find. Sharing one number makes the answer "the matching ones", permanently.

These thirteen crates are one product split up for compilation reasons — nobody uses `denise-render` without `denise` — so that is the property worth keeping. The cost is that a crate gets a new version when nothing in it changed, which matters to somebody depending on exactly one of them. Nobody does yet.

**When to revisit.** Approaching 1.0, a breaking change in `denise-drm` forcing `denise` to a major it did not earn stops being noise and becomes a semver lie. At `0.0.x` it is noise.

## The guards, and what each is for

The workflow fails closed at four points, because an upload cannot be taken back — a version can be yanked, never replaced.

| | |
|---|---|
| **The release tag matches the manifest** | True by construction now that the workflow writes the version from the tag — and asserted anyway, because if the bump path ever grows a bug, this is what stops one version being published under another's name |
| **CI passes on the exact commit being published** | The workflow waits for CI's verdict on the bump commit — per check *name*, from its most recent run, so a rerun or a duplicate cannot wedge it. A red check fails the release before anything is uploaded |
| **`--locked`** | A version bump that did not refresh `Cargo.lock` fails in rehearsal instead of halfway through an upload |
| **A full dry run first** | Packages and verifies all thirteen before anything real goes |

Separately, on every push, the `the workspace versions agree` job asserts that all of them are at one version and that every sibling pin equals that version. That one exists because the failure is invisible: `[workspace.dependencies]` pins each sibling by version *and* path, the path is what resolves locally, and the version is what gets uploaded. Miss a pin and `denise-ui` goes to crates.io requiring a `denise-render` nobody built it against. It publishes, it resolves, and no local build can notice.

## One thing to know about verification

`cargo publish` verifies by building, and the release runs on Linux. `denise-macos` is `#![cfg(target_os = "macos")]`, and `denise-win32` and `denise-activex` are `cfg(windows)` throughout — so what the release runner compiles for those three is an **empty crate**.

That still checks the packaging: the manifest, the file list, the dependency versions. It does not check the platform code. The CI gate is what covers that half — the Windows runner built and tested `denise-win32` and `denise-activex` on the same commit, minutes earlier, and the release refuses to run if it did not.

This was the one thing expected to need a matrix or a `--no-verify`, and it turned out not to. `cargo publish --workspace` builds a temporary local registry out of the packaged crates and verifies each against that, so it never waits on the real index either:

```
Unpacking denise-ui v0.0.1 (registry `target/package/tmp-registry`)
```

## Setup, once

`CARGO_REGISTRY_TOKEN` has to exist as a repository secret, from <https://crates.io/settings/tokens>. It needs **both `publish-new` and `publish-update`**, scoped to `denise-*`.

`publish-new` is the one that looks unnecessary and is not. This document used to say the token only ever updates crates that already exist — true right up until the workspace grew a thirteenth member, and a release that has to *create* `denise-image` fails on the last step of an otherwise green run. The workspace gains crates about once a year, which is exactly the interval at which nobody remembers this.

## A new crate cannot be published before its siblings are

A crate added between releases depends on sibling versions that are not on crates.io yet — `denise-image 0.4.0` wants `denise-render 0.4.0` to already contain a function added after v0.4.0 shipped. Publishing it on its own fails to verify, and there is no way to reserve a name without publishing.

So a new crate joins at the next release and not before, which `cargo publish --workspace` handles by building the whole set against a temporary local registry. Nothing needs doing; it is only worth knowing so that a failed solo `cargo publish -p` reads as expected rather than as a problem.

## What is already published

| Version | |
|---|---|
| [v0.5.0](https://github.com/bisand/denise/releases/tag/v0.5.0) | Pictures end to end: the blit, the `Image` widget, and `denise-image` — the first release to **create** a crate rather than update the ones that existed. Clean run |
| [v0.4.0](https://github.com/bisand/denise/releases/tag/v0.4.0) | What the foundations were for: RadialProgress, Spinner, Tooltip, Select and Toast — the widgets that were waiting on popups. Clean run, four minutes gate to upload |
| [v0.3.0](https://github.com/bisand/denise/releases/tag/v0.3.0) | The five foundations: arcs, requested animation, popups, DPI, scrolling. First real release through the tag-driven pipeline, clean on the first run |
| [v0.2.1-rc.1](https://github.com/bisand/denise/releases/tag/v0.2.1-rc.1) | A live test of the tag-driven pipeline, and says so in its notes. The workflow bumped from the tag, moved the tag onto the bump, waited for CI — releasing eight seconds after the last check went green — and published. Invisible to `"0.2"` requirements |
| [v0.2.0](https://github.com/bisand/denise/releases/tag/v0.2.0) | The widget wave: fourteen widgets, word wrapping, per-crate READMEs. First release through the Prepare release workflow — which is how its two bugs were found |
| [v0.1.0](https://github.com/bisand/denise/releases/tag/v0.1.0) | M4 and M5: the text engine and the embedding backends |
| [v0.0.1](https://github.com/bisand/denise/releases/tag/v0.0.1) | The first real release. All twelve, uploaded within ten seconds of each other |
| [v0.0.0](https://github.com/bisand/denise/releases/tag/v0.0.0) | Not a release — name reservations for three crates, over two days |

The two `0.0.x` rows were tagged after the fact, from crates.io upload timestamps against the commit history. `0.0.1` is exact: the commit `38568ae` is titled *"chore: bump to 0.0.1 so the published core carries M4 and M5"*, and the first upload followed it by nineteen seconds. `0.0.0` is a reconstruction and its release notes say so.
