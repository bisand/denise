<script lang="ts">
	import { base } from '$app/paths';
	import Seo from '$lib/components/Seo.svelte';
	import Icon from '$lib/components/Icon.svelte';
	import Shot from '$lib/components/Shot.svelte';
	import CopyLine from '$lib/components/CopyLine.svelte';
	import { programs, more } from '$lib/showcase';
	import { blob, tree, site } from '$lib/site';
</script>

<Seo title="Showcase" description="Programs built with DeniseUI: the form designer, the squint text editor, a widget gallery, a record editor and a small web browser — with the commands that run them." />

<section class="mx-auto max-w-7xl px-4 pt-14 pb-10">
	<span class="badge badge-soft badge-primary">Built with it</span>
	<h1 class="mt-3 text-4xl font-bold sm:text-5xl">Showcase</h1>
	<p class="mt-4 max-w-3xl text-lg text-base-content/70">
		Real programs, not mock-ups. Two of them ship as downloads, one is a separate project, and the rest are in the repository with a command each. Every screenshot here came out of the program itself.
	</p>
</section>

<section class="mx-auto max-w-7xl space-y-16 px-4 pb-10">
	{#each programs as p, i}
		<article id={p.id} class="grid scroll-mt-24 items-center gap-10 lg:grid-cols-2" class:lg:grid-flow-dense={i % 2 === 1}>
			<div class:lg:col-start-2={i % 2 === 1}>
				<div class="flex flex-wrap items-center gap-2">
					<h2 class="font-display text-2xl font-bold sm:text-3xl">{p.name}</h2>
					{#each p.tags as tag}
						<span class="badge badge-sm badge-ghost">{tag}</span>
					{/each}
				</div>
				<p class="mt-2 text-lg text-base-content/80">{p.what}</p>
				<p class="mt-4 text-base-content/70">{p.body}</p>
				{#if p.run}
					<div class="mt-5 space-y-2">
						{#each p.run as cmd}
							<CopyLine prompt="$" text={cmd} />
						{/each}
					</div>
				{/if}
				<div class="mt-5 flex flex-wrap gap-3">
					{#if p.external}
						<a href={p.external} class="btn btn-sm btn-primary" rel="noopener"><Icon name="github" class="h-4 w-4" /> The project</a>
					{:else}
						<a href={tree(p.source)} class="btn btn-sm" rel="noopener"><Icon name="code" class="h-4 w-4" /> Source</a>
					{/if}
					{#if p.id === 'designer'}
						<a href="{base}/designer/" class="btn btn-sm btn-primary"><Icon name="download" class="h-4 w-4" /> Download</a>
					{/if}
					{#if p.id === 'gallery'}
						<a href="{base}/demo/" class="btn btn-sm btn-primary"><Icon name="play" class="h-4 w-4" /> Run it here</a>
					{/if}
				</div>
			</div>
			{#if p.shot}
				<Shot src={p.shot} alt={p.alt ?? p.name} class={i % 2 === 1 ? 'lg:col-start-1' : ''} />
			{/if}
		</article>
	{/each}
</section>

<section class="border-t border-base-300 bg-base-200/40">
	<div class="mx-auto max-w-7xl px-4 py-16">
		<h2 class="text-2xl font-bold">The rest of the examples</h2>
		<p class="mt-2 max-w-2xl text-base-content/70">
			Each one exists to demonstrate a single thing, and several of them are the proof behind a claim made elsewhere on this site.
		</p>
		<div class="mt-8 grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
			{#each more as m}
				<a href={tree(m.path)} rel="noopener" class="group rounded-box border border-base-300 bg-base-100 p-4 transition hover:border-primary/50">
					<div class="font-mono text-sm font-medium group-hover:text-primary">{m.name}</div>
					<p class="mt-1 text-sm text-base-content/65">{m.what}</p>
				</a>
			{/each}
		</div>
		<div class="mt-10 rounded-box border border-base-300 bg-base-100 p-6">
			<h3 class="font-display text-lg font-semibold">Built something with DeniseUI?</h3>
			<p class="mt-2 text-sm text-base-content/70">
				Open an issue and it can go on this page. A photograph of it running on a panel is worth more than a screenshot of it in a window.
			</p>
			<div class="mt-4 flex flex-wrap gap-3">
				<a href={site.issues} class="btn btn-sm btn-primary" rel="noopener">Tell us about it</a>
				<a href={blob('README.md')} class="btn btn-sm" rel="noopener">The repository README</a>
			</div>
		</div>
	</div>
</section>
