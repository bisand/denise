# The DeniseUI website

The marketing site and guide at <https://bisand.github.io/denise/>, built with
SvelteKit, Tailwind CSS 4 and daisyUI, prerendered to static HTML by
`@sveltejs/adapter-static` so it can be hosted anywhere.

```bash
npm install
npm run wasm       # builds the live demo; needs `rustup target add wasm32-wasip1`
npm run dev        # http://localhost:5173
npm run build      # static output in build/
```

## The live demo is the toolkit

`wasm/` is a third backend beside the window and kiosk ones in `examples/`. It
pulls in `examples/gallery/src/app.rs` and `examples/table-editor/src/app.rs`
**by path, unchanged**, and exports a dozen C functions the page calls: input
in, damage rectangles out. `src/lib/denise/runtime.ts` is the other half — a
four-function WASI (a clock, stderr, no files), input translation, and a blit
that copies only the rectangles the tree reports.

It targets `wasm32-wasip1` rather than `wasm32-unknown-unknown` because those
applications use `std::time::Instant` and read `/etc/localtime`; on WASI they
compile as they are, and the browser supplies the clock. If either example
changes shape, this is where it will fail to build — which is the point.

`npm run wasm` writes `static/wasm/denise.wasm`, which is generated and not
committed. The Website workflow builds it before the site, so a deploy cannot
go out with a stale or missing demo.

## Layout

- `src/routes/` — home, `/demo/`, `/widgets/`, `/showcase/`, `/designer/`, `/docs/`
- `src/lib/docs/*.md` — the guide, one Markdown file per page (mdsvex). Register new pages in `src/lib/site.ts`
- `src/lib/site.ts`, `widgets.ts`, `showcase.ts`, `snippets.ts` — the content the pages read
- `src/lib/components/` — `LiveDemo` (the canvas and its toolbar), `Shot`, `CodeTabs`, `StackCompare`, `IdleMeter`
- `static/screenshots/` — WebP, converted by `npm run images` from PNG or JPEG dropped into that directory

The version shown in the install snippet and the download links is read from
the workspace `Cargo.toml` at build time (`vite.config.ts`), so a release keeps
the site current without anybody remembering to edit it.

## Screenshots

Most come from the applications' own snapshot modes, which need no display:

```bash
cargo run -p gallery -- --font <a .ttf> --snapshot gallery.ppm --size 2560x1600 --scale 2
cargo run -p table-editor -- --snapshot table.ppm
cargo run -p denise-designer -- forms/reference.dform --snapshot d.ppm --scale 2 --select volume --drag 40,0
ffmpeg -i gallery.ppm gallery.png     # sips will not read ppm
```

Put the PNG or JPEG in `static/screenshots/` and run `npm run images`, which
converts to WebP, caps the width at 2560, renders `static/logo-256.png` and
builds the social card from `scripts/og.svg`.

Deployed by [`.github/workflows/site.yml`](../.github/workflows/site.yml) on
every push to `main` that touches the site, the two demo examples or the
crates. Set `BASE_PATH` when serving from a sub-path.
