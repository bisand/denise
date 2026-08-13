# Releasing

All twelve crates share one version and go to crates.io together.

## Doing it

Run the **Prepare release** workflow (Actions tab, or below), giving it the one
input a release needs — the version:

```bash
gh workflow run prepare-release.yml -f version=0.2.0
```

It puts that number everywhere it has to go — the workspace version, the twelve
sibling pins, the README's install snippet, `Cargo.lock` — commits, starts CI on
that commit, and opens a **draft** release pointing at it. Review the notes, then
**publish the draft. Publishing the release is the trigger**:
`.github/workflows/release.yml` checks its guards, rehearses the whole publish,
uploads to crates.io, and lists what went out in the run summary.

Nothing before the publish is irreversible: the draft can be deleted and the bump
commit is just a commit. Publish the draft before CI finishes and the guard
refuses; publish again once it is green.

By hand, the same thing is:

```bash
scripts/bump-version.py 0.2.0     # manifest, twelve pins, README snippet
cargo update --workspace          # refresh Cargo.lock
git commit -am "chore: release 0.2.0"
git push
# once CI is green on that commit:
gh release create v0.2.0 --generate-notes
```

The script is shared, so the workflow and a human produce the identical commit —
there is one place that knows where version numbers live.

One deliberate wrinkle inside the workflow: a push made with the workflow token
starts no `on: push` run — GitHub's guard against recursive workflows — so it
dispatches CI onto the bump commit explicitly. Without that, every release
prepared this way would fail the released-commit-passed-CI guard, permanently.

To rehearse without publishing anything, run the **Release** workflow by hand from the Actions tab. With no release attached it stops after the dry run.

### Why the release comes first

The usual arrangement is a tag that triggers a publish that then opens a release. This is the other way round, and deliberately: the release exists and describes what is about to go out, then crates.io follows.

The reason is what each failure leaves behind. If the publish fails here, the release stays visible with a red workflow beside it — a thing with a name, notes and a URL, that plainly did not finish. Tag-first leaves a tag nobody can explain and no page to hang the explanation on. Given the upload cannot be undone, the arrangement where a half-finished release is legible is the better one.

It also means the notes are written by a person before the irreversible step rather than generated after it.

## Why lockstep

Because the version number is worth more as a compatibility statement than as a change log. Under independent versioning you get `denise-ui 0.3.1` requiring `denise-render 0.2.4` within about three releases, and from then on "which versions work together" needs a table somebody maintains and readers have to find. Sharing one number makes the answer "the matching ones", permanently.

These twelve crates are one product split up for compilation reasons — nobody uses `denise-render` without `denise` — so that is the property worth keeping. The cost is that a crate gets a new version when nothing in it changed, which matters to somebody depending on exactly one of them. Nobody does yet.

**When to revisit.** Approaching 1.0, a breaking change in `denise-drm` forcing `denise` to a major it did not earn stops being noise and becomes a semver lie. At `0.0.x` it is noise.

## The guards, and what each is for

The workflow fails closed at four points, because an upload cannot be taken back — a version can be yanked, never replaced.

| | |
|---|---|
| **The release tag matches the manifest** | A `v0.1.0` release on a tree that says `0.0.9` publishes one version under another's name, and the release page then describes a tree that never carried that number |
| **The released commit passed CI** | A release can be cut at any commit, including one CI never saw or one that went red. The job asks GitHub about that exact commit rather than assuming |
| **`--locked`** | A version bump that did not refresh `Cargo.lock` fails in rehearsal instead of halfway through an upload |
| **A full dry run first** | Packages and verifies all twelve before anything real goes |

Separately, on every push, the `the workspace versions agree` job asserts that all twelve are at one version and that every sibling pin equals that version. That one exists because the failure is invisible: `[workspace.dependencies]` pins each sibling by version *and* path, the path is what resolves locally, and the version is what gets uploaded. Miss a pin and `denise-ui` goes to crates.io requiring a `denise-render` nobody built it against. It publishes, it resolves, and no local build can notice.

## One thing to know about verification

`cargo publish` verifies by building, and the release runs on Linux. `denise-macos` is `#![cfg(target_os = "macos")]`, and `denise-win32` and `denise-activex` are `cfg(windows)` throughout — so what the release runner compiles for those three is an **empty crate**.

That still checks the packaging: the manifest, the file list, the dependency versions. It does not check the platform code. The CI gate is what covers that half — the Windows runner built and tested `denise-win32` and `denise-activex` on the same commit, minutes earlier, and the release refuses to run if it did not.

This was the one thing expected to need a matrix or a `--no-verify`, and it turned out not to. `cargo publish --workspace` builds a temporary local registry out of the packaged crates and verifies each against that, so it never waits on the real index either:

```
Unpacking denise-ui v0.0.1 (registry `target/package/tmp-registry`)
```

## Setup, once

`CARGO_REGISTRY_TOKEN` has to exist as a repository secret, from <https://crates.io/settings/tokens>. Scope it to publishing updates for the existing crates; it does not need permission to create new ones.

## What is already published

| Version | |
|---|---|
| [v0.2.0](https://github.com/bisand/denise/releases/tag/v0.2.0) | The widget wave: fourteen widgets, word wrapping, per-crate READMEs. First release through the Prepare release workflow — which is how its two bugs were found |
| [v0.1.0](https://github.com/bisand/denise/releases/tag/v0.1.0) | M4 and M5: the text engine and the embedding backends |
| [v0.0.1](https://github.com/bisand/denise/releases/tag/v0.0.1) | The first real release. All twelve, uploaded within ten seconds of each other |
| [v0.0.0](https://github.com/bisand/denise/releases/tag/v0.0.0) | Not a release — name reservations for three crates, over two days |

Both were tagged after the fact, from crates.io upload timestamps against the commit history. `0.0.1` is exact: the commit `38568ae` is titled *"chore: bump to 0.0.1 so the published core carries M4 and M5"*, and the first upload followed it by nineteen seconds. `0.0.0` is a reconstruction and its release notes say so.
