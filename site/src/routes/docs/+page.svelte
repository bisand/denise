<script lang="ts">
	import { base } from '$app/paths';
	import Seo from '$lib/components/Seo.svelte';
	import Icon from '$lib/components/Icon.svelte';
	import { docPages, site, crates, blob } from '$lib/site';

	const groups = [...new Set(docPages.map((d) => d.group))];

	const repoDocs = [
		{ path: 'docs/design.md', title: 'Design notes', what: 'How it is built and why: architecture, rasteriser, text, keyboards, theming, and the milestone history.' },
		{ path: 'docs/raspberry-pi.md', title: 'Raspberry Pi', what: 'Getting a Pi to hand over a display at all, and what to check when it will not.' },
		{ path: 'docs/windows.md', title: 'Windows', what: 'The Win32 control and the ActiveX shim, including the toolchain traps.' },
		{ path: 'docs/forms.md', title: 'The .dform file', what: 'Why KDL, the version 1 schema, loading one from Rust, and what a form file will not do.' },
		{ path: 'docs/designer.md', title: 'The designer', what: 'From a drawing to a running screen, end to end.' },
		{ path: 'docs/arrange.md', title: 'denise-arrange', what: 'The design note for content-driven sizing in an optional crate. Nothing is built yet.' },
		{ path: 'docs/releasing.md', title: 'Releasing', what: 'How a version goes to crates.io, and what each guard is for.' }
	];
</script>

<Seo title="Documentation" description="Guides for DeniseUI: getting started, how damage tracking works, theming, form files and the designer, Raspberry Pi kiosks, and embedding the toolkit in other applications." />

<section class="mx-auto max-w-7xl px-4 pt-14 pb-8">
	<h1 class="text-4xl font-bold sm:text-5xl">Documentation</h1>
	<p class="mt-4 max-w-2xl text-lg text-base-content/70">
		Six guides that get you from an empty project to a screen on a board. The API reference lives on docs.rs; the design notes and every crate’s own README live in the repository.
	</p>
</section>

<section class="mx-auto max-w-7xl px-4 pb-8">
	{#each groups as group}
		<h2 class="mt-8 mb-3 font-display text-sm font-semibold tracking-wide text-base-content/50 uppercase">{group}</h2>
		<div class="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
			{#each docPages.filter((d) => d.group === group) as doc, i}
				<a href="{base}/docs/{doc.slug}/" class="group card card-border bg-base-200/60 transition hover:border-primary/50">
					<div class="card-body p-5">
						<div class="flex items-center gap-2">
							<span class="grid h-7 w-7 place-items-center rounded-lg bg-primary/15 font-mono text-xs text-primary">{i + 1}</span>
							<h3 class="font-display font-semibold group-hover:text-primary">{doc.title}</h3>
						</div>
					</div>
				</a>
			{/each}
		</div>
	{/each}
</section>

<section class="mx-auto max-w-7xl px-4 py-12">
	<div class="grid gap-8 lg:grid-cols-2">
		<div class="card card-border bg-base-200/50">
			<div class="card-body">
				<h2 class="card-title"><Icon name="book" class="h-5 w-5 text-primary" /> API reference</h2>
				<p class="text-sm text-base-content/70">Every crate documents itself, and CI compiles the examples in those docs.</p>
				<div class="mt-2 grid grid-cols-2 gap-1 text-sm sm:grid-cols-3">
					{#each crates as c}
						<a class="link link-hover font-mono text-xs" href="https://docs.rs/{c.name}" rel="noopener">{c.name}</a>
					{/each}
				</div>
			</div>
		</div>
		<div class="card card-border bg-base-200/50">
			<div class="card-body">
				<h2 class="card-title"><Icon name="github" class="h-5 w-5 text-primary" /> In the repository</h2>
				<ul class="mt-1 space-y-2 text-sm">
					{#each repoDocs as doc}
						<li>
							<a class="link font-medium" href={blob(doc.path)} rel="noopener">{doc.title}</a>
							<span class="text-base-content/60"> — {doc.what}</span>
						</li>
					{/each}
				</ul>
				<p class="mt-3 text-sm text-base-content/60">
					Questions, or something that does not work? <a class="link" href={site.issues} rel="noopener">Open an issue</a>.
				</p>
			</div>
		</div>
	</div>
</section>
