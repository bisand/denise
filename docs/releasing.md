# Releasing

All eighteen crates share one version and go to crates.io together.

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

### The downloads

The designer is a *program*, and nobody who wants a form designer will
`cargo install` one. After the publish — after the one irreversible step, since a
binary that fails to build cannot un-publish a crate — a second workflow builds
`denise-designer` and the `denise-forms` CLI for four platforms, runs each one to
check it opens the reference form, and attaches them to the release:

| | |
|---|---|
| macOS | one `.dmg` holding **Denise Designer.app**, universal for Intel and Apple silicon |
| Windows | a `.zip` with the `.exe` |
| Linux x86-64, aarch64 | a `.tar.gz` each |

Every archive carries both programs, the licence, the designer's README, and the
form files with the pictures they name — a designer that opens with nothing to
open is a blank page. Each artifact has a `.sha256` beside it, and the release
notes gain a **Download the designer** section, appended once under a marker so a
re-run does not say it twice and whatever was written above it is left alone.

To rehearse it, run the **Binaries** workflow by hand from the Actions tab with
any tag. If no release of that name exists the archives come back as the run's
own artifacts and nothing is published or edited, which makes it safe to try on a
tag that is only a tag.

It is re-runnable on its own for a tag whose release already exists: the uploads
use `--clobber`, so a second attempt replaces the first rather than failing on it.

**They are not signed.** There is no Apple Developer account behind this project
and no Windows code-signing certificate, so a first launch needs a right-click →
*Open* on macOS and *More info* → *Run anyway* on Windows. The release notes say
so. The macOS binaries *are* ad-hoc signed, which is not the same thing and is
not optional: `lipo` throws away whatever signature the two halves had, and an
arm64 Mac will not run a Mach-O with no signature at all.

### Why this is not cargo-dist

`cargo-dist` does all of the above and generates installer scripts too, and it
was the first thing tried. It wants to **own** the release flow: it plans from a
tag push, creates the release object itself, and decides what happens when.
Everything above is the other way round — the release is published first, by a
person, and the tag drives the rest — so adopting it would have meant rewriting
the one part of this process that is written down and understood, in exchange for
installer scripts. What is left once its opinions are not needed is eighty lines
of workflow.

### When a release fails partway

Nothing is uploaded until every guard has passed, so a red run leaves crates.io
untouched and the release visible with a red workflow beside it — that is the
designed failure state. If the cause was red CI or a bad tree: fix main, then
**delete the release and its tag** and release again from the fixed tree.
Re-publishing the same release (draft-toggle) re-runs the workflow against its
existing tag, which is only right when the tree it points at was fine and the
failure was transient.

To rehearse without publishing anything, run the **Release** workflow by hand
from the Actions tab. With no release attached it stops after the dry run, and
the downloads are not built — rehearse those from the **Binaries** workflow
instead, which is a job of its own for exactly this reason.

### Why the release comes first

The usual arrangement is a tag that triggers a publish that then opens a release. This is the other way round, and deliberately: the release exists and describes what is about to go out, then crates.io follows.

The reason is what each failure leaves behind. If the publish fails here, the release stays visible with a red workflow beside it — a thing with a name, notes and a URL, that plainly did not finish. Tag-first leaves a tag nobody can explain and no page to hang the explanation on. Given the upload cannot be undone, the arrangement where a half-finished release is legible is the better one.

It also means the notes are written by a person before the irreversible step rather than generated after it.

## Why lockstep

Because the version number is worth more as a compatibility statement than as a change log. Under independent versioning you get `denise-ui 0.3.1` requiring `denise-render 0.2.4` within about three releases, and from then on "which versions work together" needs a table somebody maintains and readers have to find. Sharing one number makes the answer "the matching ones", permanently.

These eighteen crates are one product split up for compilation reasons — nobody uses `denise-render` without `denise` — so that is the property worth keeping. The cost is that a crate gets a new version when nothing in it changed, which matters to somebody depending on exactly one of them. Nobody does yet.

**When to revisit.** Approaching 1.0, a breaking change in `denise-drm` forcing `denise` to a major it did not earn stops being noise and becomes a semver lie. At `0.0.x` it is noise.

## The guards, and what each is for

The workflow fails closed at four points, because an upload cannot be taken back — a version can be yanked, never replaced.

| | |
|---|---|
| **The release tag matches the manifest** | True by construction now that the workflow writes the version from the tag — and asserted anyway, because if the bump path ever grows a bug, this is what stops one version being published under another's name |
| **CI passes on the exact commit being published** | The workflow waits for CI's verdict on the bump commit — per check *name*, from its most recent run, so a rerun or a duplicate cannot wedge it. A red check fails the release before anything is uploaded |
| **`--locked`** | A version bump that did not refresh `Cargo.lock` fails in rehearsal instead of halfway through an upload |
| **A full dry run first** | Packages and verifies all eighteen before anything real goes |

Since the release waits on CI *by check name*, every job in `ci.yml` is a release
guard whether or not it was added as one. The `advisories` job is the case worth
naming: a RUSTSEC advisory against `png`, `gif`, `zune-jpeg` or `fontdue` — the
crates that parse untrusted bytes — now fails CI, and a failed CI stops the
publish. Nothing had to be wired into `release.yml` for that; adding the job to
CI was the whole of it.

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
| [v0.19.0](https://github.com/bisand/denise/releases/tag/v0.19.0) | A form can have a window of its own, on the desktop backend and deliberately nowhere else: `DeniseApp::take_windows` hands back a `WindowRequest` and gets a window with its own surface, damage tracker and frame deadline, running another `DeniseApp` — the scene stack's relationship to `Ui` with an OS window as the container, so a settings form and the main window are the same kind of thing built the same way. `denise-ui` is untouched by it and knows nothing about a second tree; a kiosk build links `denise-drm` and never compiles a line of it, and on the embedded backends `Ui::push_scene` stays the only way to ask a question, because a control that spawned a toplevel would escape its host's modality. Modality is the runner's rather than the platform's, since only one platform has one: Windows gets a real dialog box from `with_owner_window` plus `set_enable(false)`, macOS gets z-order from `addChildWindow:ordered:` and no blocking, and X11 and Wayland offer neither through winit — so a blocked window drops input and keeps painting on all three, and `owner.rs` is appearance rather than correctness. `with_parent_window` is deliberately unused, being a child control clipped to its parent's client area on both Windows and X11, which is the thing the embedding crates already are. Nothing can reach into a window it did not build: a form closes *itself*, and windows talk through state the application owns — a refusal rather than an omission, since a handle would be the seam through which a tree stops being the only thing that owns a tree. The two ownership rules that break — a cascade closes owners last, and an owner with two modals over it stays blocked until the second goes — are stated over plain data and tested everywhere rather than only where there is a display, the split `keymap` has always made. One break: `run` and `run_with` require `'static`, the loop now holding applications of several types in one collection. Watched working on macOS only; the Win32 owner path is written from the documentation, compiled and unit tested by CI, and has never been run by a person. Clean run |
| [v0.18.1](https://github.com/bisand/denise/releases/tag/v0.18.1) | Photographs, and nothing else — sixteen crates get a version so that the panel pictures have a release to belong to, which is this arrangement's own named cost showing up for the first time in a form worth pointing at: not one packaged byte differs, because the pictures are in the *root* README and every crate ships its own. Both were three features behind — no globe on the layout key, no key that puts the keyboard away, no sign of 0.17.0's gesture — and the replacement for the second is taken **mid-gesture**, a finger holding `e` with the framed strip above it offering `é è ê ë` and the one under the finger picked out. Two things in that picture had never been seen outside a unit test: the highlight following the finger, and the frame doing its job, an earlier strip having been drawn flush in the keys' own colours where it read as another row of the keyboard. The pointer is the mouse that drove the gesture, so touch on real glass stays the one unverified thing and wants a digitiser rather than more code. `pi-table-editor-keyboard.jpg` became `pi-table-editor-alternates.jpg`, no longer showing the same thing; v0.16.0's notes pin the old name by commit sha and go on resolving, which was checked rather than assumed. Clean run |
| [v0.18.0](https://github.com/bisand/denise/releases/tag/v0.18.0) | One change reaches crates.io and it is a break, which is the reason not to sit on it: `Layout` becomes `#[non_exhaustive]`. The crate had added a field to it twice — the decimal separator, then 0.17.0's alternates, which broke every struct literal building one — and `LayoutSource` had taken the same medicine in 0.16.0 while `Layout` had not, the sort of asymmetry nobody remembers a year later when the third field arrives. Nothing constructs one outside the crate today, so it costs nothing now and only now; it also says something true about the type, since these tables *are* the layouts that ship and one written elsewhere would skip the tests that walk every layout and check it can type its own alphabet. Cut two hours after 0.17.0 for that reason alone. In the browser example, which does not ship: a typed path is a path whether or not the file is there — one that was not fell past `path.exists()` to a catch-all that built `https:///Users/me/x.html`, which a URL parser does not reject but collapses, reading the first path component as the **host** — so the address became `https://users/x.html`, a request to a machine called `users` carrying the rest of somebody's path to whatever the resolver makes of a bare name. 0.17.0's notes called that an empty host, which is what it looks like and not what a parser does with it, and now carry a correction. Clean run |
| [v0.17.0](https://github.com/bisand/denise/releases/tag/v0.17.0) | The keys stop depending on which fonts are installed, and holding a letter reaches its accents — the same complaint from both ends. `denise_render::icon` is filled polygons on a hundred-square box and deliberately nothing else: no curves, no path builder, and no strokes, because `draw_line` has no thickness and a one-pixel outline on a 48-pixel key would be invisible — so an outline is a fill with its middle knocked back out, which is how both the `×` inside `⌫` and the ring of the globe are made. It replaced asking the font, where DejaVu has `⌫ ⇥ ⏎` and both triangles but not `⎋`, a Mac's Arial has neither triangle, and the face that ships here has none of them. Holding a letter offers alternates from `Layout::alternates`, which are the layout's own: Norwegian does not offer `ø` from `o` because `ø` has a key of its own, and US does because it has not. Not a popup, and not as a shortcut — a pushed scene cancels the press it covers, and the press it would cancel is the one holding the key that opened it — so the strip is ordinary nodes on the shelf, and `Keyboard::handle` does the hit test the tree cannot, the choice being made by where the finger lifts rather than by what it pressed. The layout key wears a globe and keeps its name in the corner, a globe being unable to say which of three layouts is live; Escape becomes a keyboard going downwards, which it earns here because here its job is to put the keyboard away. Two bugs neither demo would have shown: a cancelled touch typed whatever its last reported position was over, and the strip's choices hung off the shelf rather than the strip, so five invisible nodes survived every gesture. One break — `Layout` gained a public field, and is still not `#[non_exhaustive]`, so the next one will break literals again the way this one did. Touch on glass is the one thing unverified, neither board having a digitiser. The product also gets a searchable name, DeniseUI, while the repository and the root crate stay `denise`. Clean run |
| [v0.16.0](https://github.com/bisand/denise/releases/tag/v0.16.0) | A panel with nothing plugged into it can be typed on. Two crates: `denise-layout`, the position-to-character tables moved out of `denise-evdev` because a table saying `Semicolon` types `ø` is no more about evdev than about Cocoa, and `denise-keyboard`, which emits what the hardware path emits from the same tables through the same composer. Fourteen columns and not the ISO/ANSI intersection, which sounds careful and could not type `å`; numbers carry what Shift would give; the named keys draw `⌫ ⇥ ⏎ ◀ ▶` where the font has them, asked rather than assumed, because DejaVu has every one and a Mac's Arial has neither triangle. Five toolkit features it needed and none keyboard-specific — the shelf, `Button::no_focus`, `Panel::backdrop`, `Ui::reveal_focused`, `TextEngine::font_contains` — plus two bugs older than any of it: a held press dropped without telling the widget, and a viewport left scrolled past content that shrank. Verified on a Pi 3 A+ configured Norwegian, which is where the flicker was found too: the keyboard repainted itself every frame because collecting held keys went through `Ui::widget_mut`, which damages what it hands out. The repaint tests could not see it — they ask whether the panel shows the right pixels, and repainting too much does. `LayoutSource` lost `Copy` and gained a variant; everything else is additive. The third release to create a crate, and the first to create two. Clean run |
| [v0.15.1](https://github.com/bisand/denise/releases/tag/v0.15.1) | Seven badges. 0.15.0 got the feature-gated items onto docs.rs at all; this labels each with the feature it needs, because a decoder or a text tier shown with no label reads as available in a default build. `doc_cfg` behind `cfg_attr(docsrs, ...)`, in the only two crates that gate public items on a feature — `denise-image`'s three decoders and `denise-text`'s two optional tiers. Note that `doc_auto_cfg` was removed in 1.92 and merged into `doc_cfg`, so the older name is the one that works. No code change. Clean run |
| [v0.15.0](https://github.com/bisand/denise/releases/tag/v0.15.0) | Nothing shipped behaves differently; what changed is where each crate is documented and which machines check it. No crate carried `[package.metadata.docs.rs]`, so all fourteen were documented on linux with default features — and `denise-macos` is `#![cfg(target_os = "macos")]` from its first line, so its page had been publishing three items against seven. Twenty-four dead intra-doc links mended, and twenty-five widget examples turned from `ignore` into doctests that compile, which immediately found a wrong import where the obvious one would look: 23 workspace doctests to 48. `denise_render::blit` is public and is the only API addition — a module that exports nothing, published because it is where the premultiplied source format is explained and a private module renders nowhere. CI grew a Mac, the Linux backends on arm, `aarch64-unknown-linux-musl`, and rustdoc warnings that fail; the first two found an ABI test that had never compiled on aarch64 at all. Clean run |
| [v0.14.0](https://github.com/bisand/denise/releases/tag/v0.14.0) | Scrolling stops tearing on a panel, and input survives a device that was not there at boot: `PresentMode::Immediate` becomes a preference rather than a promise about every frame — at or above a quarter of the surface's rows the flip waits for vblank, counted over the rows the damage actually covers rather than its bounding box — and `denise-evdev` watches `/dev/input` instead of scanning it once. The sidebar that tore was 14.7% of the pixels and 94% of the scanlines; the mouse asleep at boot turned up 775 seconds in. Both were found by putting a real panel on a wall, which is also where the release's examples came from. `devices_changed` is the only addition and nothing breaks. Clean run |
| [v0.13.0](https://github.com/bisand/denise/releases/tag/v0.13.0) | One knob for how fast animation runs: `Ui::set_motion` replaces four private `FRAME_MS` constants, and `Animation` stops spelling a frame rate and a deadline the same way — `Wake::Animating` is a rate the tree owns, `Wake::At` is a deadline it must not touch. On a Pi 3A+, 4.20% of a core at 16 ms, 2.06% at 33, and 0.00% with motion off. The first release to break `Widget::animate`. Clean run |
| [v0.12.0](https://github.com/bisand/denise/releases/tag/v0.12.0) | macOS stops copying the whole window sixty times a second: `denise-winit` presents through a pair of `IOSurface`s instead of softbuffer, whose CoreGraphics backend reallocates per frame and discards the damage. 48.8% of a core to 3.5%, at three times the frame rate. Clean run |
| [v0.11.1](https://github.com/bisand/denise/releases/tag/v0.11.1) | An idle window costs nothing: the desktop backend stopped presenting frames nobody asked for, and `DeniseApp::next_frame_in` lets an application set the cadence the way the kiosk loops always have. Hello 19% → 0.4% on a Retina Mac, the Pi identical to the hundredth of a second. Clean run, after re-running three jobs that failed to download their own actions |
| [v0.11.0](https://github.com/bisand/denise/releases/tag/v0.11.0) | The review pass: `cargo deny`, Miri and fuzzing in CI, plus what they and a reading found — an overflow panic in the caret blink reachable from any large clock, buffer validation that wrapped on 32-bit, a `Picture` that could lie about its size, and examples copying the whole window on every caret blink. Clean run, after two goes at teaching CI to install cargo-fuzz |
| [v0.10.2](https://github.com/bisand/denise/releases/tag/v0.10.2) | `Ui::popup_open`, and Escape quitting the windowed examples the way it always has on a kiosk — asking the tree first, since a drawer that advertises Escape should get it. Clean run |
| [v0.10.1](https://github.com/bisand/denise/releases/tag/v0.10.1) | The close button closes the window. `CloseRequested` reached every application and was read by none of them, since `exit_requested` defaults to false — dead on all three platforms, and only noticed on a Mac because Cmd+Q hid it. Clean run |
| [v0.10.0](https://github.com/bisand/denise/releases/tag/v0.10.0) | The desktop end of the DPI decision: `run_with` hands an application its display's scale factor, and a window's size becomes logical. The scaling path existed and was tested for three releases with nothing on a desktop able to reach it. Clean run |
| [v0.9.1](https://github.com/bisand/denise/releases/tag/v0.9.1) | The scrollable-stack `max_scroll` fix, found by the new gallery example within minutes of it existing. Clean run |
| [v0.9.0](https://github.com/bisand/denise/releases/tag/v0.9.0) | `denise-video`, the fourteenth crate: hardware decode onto a DRM plane, verified on a Pi 3A+ before it shipped. The second release to create a crate. Clean run |
| [v0.8.0](https://github.com/bisand/denise/releases/tag/v0.8.0) | Animated relayout — layout tweens and stacks — plus Collapse, Accordion and the drawer. The widget tracker closed with this one. Clean run |
| [v0.7.0](https://github.com/bisand/denise/releases/tag/v0.7.0) | Timeline and Carousel — twenty-three widgets, and every unblocked widget in the tracker done. Clean run |
| [v0.6.0](https://github.com/bisand/denise/releases/tag/v0.6.0) | Table, Rating, Avatar and the star primitive — twenty-one widgets, and the widget tracker's arcs and scrolling groups both closed. Clean run |
| [v0.5.0](https://github.com/bisand/denise/releases/tag/v0.5.0) | Pictures end to end: the blit, the `Image` widget, and `denise-image` — the first release to **create** a crate rather than update the ones that existed. Clean run |
| [v0.4.0](https://github.com/bisand/denise/releases/tag/v0.4.0) | What the foundations were for: RadialProgress, Spinner, Tooltip, Select and Toast — the widgets that were waiting on popups. Clean run, four minutes gate to upload |
| [v0.3.0](https://github.com/bisand/denise/releases/tag/v0.3.0) | The five foundations: arcs, requested animation, popups, DPI, scrolling. First real release through the tag-driven pipeline, clean on the first run |
| [v0.2.1-rc.1](https://github.com/bisand/denise/releases/tag/v0.2.1-rc.1) | A live test of the tag-driven pipeline, and says so in its notes. The workflow bumped from the tag, moved the tag onto the bump, waited for CI — releasing eight seconds after the last check went green — and published. Invisible to `"0.2"` requirements |
| [v0.2.0](https://github.com/bisand/denise/releases/tag/v0.2.0) | The widget wave: fourteen widgets, word wrapping, per-crate READMEs. First release through the Prepare release workflow — which is how its two bugs were found |
| [v0.1.0](https://github.com/bisand/denise/releases/tag/v0.1.0) | M4 and M5: the text engine and the embedding backends |
| [v0.0.1](https://github.com/bisand/denise/releases/tag/v0.0.1) | The first real release. All twelve, uploaded within ten seconds of each other |
| [v0.0.0](https://github.com/bisand/denise/releases/tag/v0.0.0) | Not a release — name reservations for three crates, over two days |

The two `0.0.x` rows were tagged after the fact, from crates.io upload timestamps against the commit history. `0.0.1` is exact: the commit `38568ae` is titled *"chore: bump to 0.0.1 so the published core carries M4 and M5"*, and the first upload followed it by nineteen seconds. `0.0.0` is a reconstruction and its release notes say so.
