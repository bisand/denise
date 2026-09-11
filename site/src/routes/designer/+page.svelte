<script lang="ts">
	import { base } from '$app/paths';
	import Seo from '$lib/components/Seo.svelte';
	import Icon from '$lib/components/Icon.svelte';
	import Shot from '$lib/components/Shot.svelte';
	import CopyLine from '$lib/components/CopyLine.svelte';
	import { downloads, downloadUrl, version, site, blob, tree } from '$lib/site';

	const gestures = [
		{ title: 'Drag out of the palette', body: 'Every widget the file format knows, filtered by name, dropped where you let go — and the drop lands in whatever panel is under it.' },
		{ title: 'Move, resize, align', body: 'Eight handles, snapping guides against siblings and the form’s own edges, align and distribute, same-size, and grouping.' },
		{ title: 'Edit every property', body: 'The inspector holds the rectangle, the role, the text, visibility, tab order, the event names — and dims the ones the file does not write.' },
		{ title: 'Press F5', body: 'Preview mode hides the scrim that keeps the form from behaving while it is being designed. That is the whole of preview mode.' },
		{ title: 'Undo, exactly', body: 'Every edit lands in the file as a targeted change and undoes byte for byte, because the model is the document rather than a struct serialised over it.' },
		{ title: 'Keep your editor open', body: 'The designer watches the file it has open and reads it again when your editor saves, keeping the selection by name.' }
	];
</script>

<Seo title="The form designer" description="Draw a DeniseUI screen instead of writing it: a visual designer for .dform files, itself a Denise application, with builds for macOS, Windows and Linux." />

<section class="relative overflow-hidden border-b border-base-300">
	<div class="glow pointer-events-none absolute inset-0 -z-10 opacity-60"></div>
	<div class="mx-auto max-w-7xl px-4 pt-14 pb-12">
		<div class="grid items-center gap-10 lg:grid-cols-[1fr_1.25fr]">
			<div>
				<span class="badge badge-soft badge-primary">Delphi had one in 1995</span>
				<h1 class="mt-3 text-4xl font-bold sm:text-5xl">Draw the screen. Ship the file.</h1>
				<p class="mt-4 text-lg text-base-content/75">
					A visual form designer for DeniseUI, and a Denise application itself — not Tauri, not egui, not a web page. The canvas draws your form with the same code that will draw it on the panel, so what is on screen is what ships, to the pixel.
				</p>
				<div class="mt-7 flex flex-wrap gap-3">
					<a href="#downloads" class="btn btn-primary btn-lg"><Icon name="download" class="h-5 w-5" /> Download v{version}</a>
					<a href="{base}/docs/forms-and-designer/" class="btn btn-lg">How a form becomes a screen</a>
				</div>
				<p class="mt-3 text-sm text-base-content/55">macOS, Windows and Linux. No Rust toolchain needed. MIT licensed.</p>
			</div>
			<Shot
				src="designer-2x.webp"
				alt="The designer: palette and outline on the left, the reference form on the canvas with a selected slider showing eight handles and an alignment guide, and the slider’s properties on the right"
				eager
			/>
		</div>
	</div>
</section>

<section class="mx-auto max-w-7xl px-4 py-16">
	<div class="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
		{#each gestures as g}
			<div class="card card-border bg-base-200/60">
				<div class="card-body p-5">
					<h2 class="font-display text-base font-semibold">{g.title}</h2>
					<p class="text-sm text-base-content/70">{g.body}</p>
				</div>
			</div>
		{/each}
	</div>
</section>

<section class="border-y border-base-300 bg-base-200/40">
	<div class="mx-auto max-w-7xl px-4 py-16">
		<div class="grid items-center gap-10 lg:grid-cols-2">
			<div>
				<span class="badge badge-soft badge-accent">The file</span>
				<h2 class="mt-3 text-3xl font-bold">A form is text, and stays text</h2>
				<p class="mt-4 text-base-content/75">
					What the designer writes is a <code class="font-mono text-[0.9em]">.dform</code> file: KDL, readable in a diff, hand-editable, and your comments survive a save. The same file is loaded at run time by <a class="link" href="https://docs.rs/denise-forms" rel="noopener">denise-forms</a> in about five lines, or generated into Rust the compiler checks.
				</p>
				<p class="mt-4 text-base-content/75">
					That is what makes a screen a deployable thing: copy a file over the one on the panel and the application draws the new screen, with every event still reaching its function — or fails at load naming the one that does not.
				</p>
				<div class="mt-6 flex flex-wrap gap-3">
					<a href={blob('docs/forms.md')} class="btn btn-sm" rel="noopener">The .dform schema</a>
					<a href={blob('forms/reference.dform')} class="btn btn-sm" rel="noopener">The reference form</a>
					<a href={tree('tools/designer')} class="btn btn-sm" rel="noopener"><Icon name="code" class="h-4 w-4" /> Designer source</a>
				</div>
			</div>
			<div class="grid gap-4 sm:grid-cols-2">
				<Shot src="designer-carry-2x.webp" alt="A slider being carried out of the designer’s palette over the form" caption="Dragging out of the palette" />
				<Shot src="designer-tab-order-2x.webp" alt="The designer numbering every tab stop on the form" caption="Tab order, numbered on the form" />
				<Shot src="designer-preview-2x.webp" alt="The designer in preview mode, running the form on its canvas" caption="F5: the form runs on the canvas" />
				<Shot src="designer-clash.webp" alt="The designer’s file-changed sheet, offering to reload a form edited elsewhere" caption="The file changed underneath: it asks" />
			</div>
		</div>
	</div>
</section>

<section id="downloads" class="mx-auto max-w-5xl scroll-mt-20 px-4 py-16">
	<h2 class="text-3xl font-bold">Download</h2>
	<p class="mt-3 text-base-content/70">
		Each archive carries the designer, the <code class="font-mono text-[0.9em]">denise-forms</code> command line tool, and the reference form to open. Every release builds all four.
	</p>
	<div class="mt-8 grid gap-3 sm:grid-cols-2">
		{#each downloads as d}
			<a href={downloadUrl(d.file)} rel="noopener" class="group flex items-center gap-4 rounded-box border border-base-300 bg-base-200/60 p-4 transition hover:border-primary/60">
				<Icon name={d.icon} class="h-8 w-8 text-base-content/70 group-hover:text-primary" />
				<span class="min-w-0 flex-1">
					<span class="block font-display font-semibold">{d.os}</span>
					<span class="block text-sm text-base-content/60">{d.detail}</span>
					<span class="block truncate font-mono text-xs text-base-content/40">{d.file}</span>
				</span>
				<Icon name="download" class="h-5 w-5 shrink-0 text-base-content/40 group-hover:text-primary" />
			</a>
		{/each}
	</div>
	<p class="mt-4 text-sm text-base-content/60">
		Each has a <code class="font-mono">.sha256</code> beside it on the <a class="link" href={site.latestRelease} rel="noopener">release page</a>.
	</p>

	<div class="alert mt-8 items-start">
		<Icon name="shield" class="h-5 w-5 shrink-0 text-warning" />
		<div>
			<h3 class="font-semibold">These are not signed</h3>
			<p class="mt-1 text-sm text-base-content/75">
				There is no Apple Developer account behind this project and no Windows code-signing certificate, and saying so is cheaper than pretending otherwise.
			</p>
			<ul class="mt-2 space-y-1 text-sm text-base-content/75">
				<li><strong>macOS</strong> — right-click <em>Denise Designer.app</em> and choose <strong>Open</strong>, then <strong>Open</strong> again. macOS remembers; every launch after that is a double-click.</li>
				<li><strong>Windows</strong> — SmartScreen offers <strong>More info</strong>, then <strong>Run anyway</strong>.</li>
				<li><strong>Linux</strong> — <code class="font-mono">tar xzf</code> and run it. Linux does not care.</li>
			</ul>
		</div>
	</div>

	<div class="mt-10">
		<h3 class="font-display text-lg font-semibold">Or build it, which needs no explanation</h3>
		<div class="mt-3 space-y-2">
			<CopyLine prompt="$" text="cargo run -p denise-designer" />
			<CopyLine prompt="$" text="cargo run -p denise-designer -- forms/reference.dform" />
		</div>
	</div>
</section>
