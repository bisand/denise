<script lang="ts">
	import { base } from '$app/paths';
	import Seo from '$lib/components/Seo.svelte';
	import Icon from '$lib/components/Icon.svelte';
	import { docPages } from '$lib/site';

	let { data } = $props();

	const index = $derived(docPages.findIndex((d) => d.slug === data.slug));
	const previous = $derived(index > 0 ? docPages[index - 1] : null);
	const next = $derived(index >= 0 && index < docPages.length - 1 ? docPages[index + 1] : null);
	const groups = [...new Set(docPages.map((d) => d.group))];
</script>

<Seo title={data.meta.title} description={data.meta.description} />

<div class="mx-auto grid max-w-7xl gap-10 px-4 pt-10 pb-10 lg:grid-cols-[220px_minmax(0,1fr)]">
	<aside class="lg:sticky lg:top-20 lg:self-start">
		<a href="{base}/docs/" class="mb-3 inline-flex items-center gap-1.5 text-sm text-base-content/60 hover:text-base-content">
			<Icon name="book" class="h-4 w-4" /> All guides
		</a>
		{#each groups as group}
			<div class="mt-4">
				<div class="mb-1 text-xs font-semibold tracking-wide text-base-content/40 uppercase">{group}</div>
				<ul class="menu menu-sm w-full px-0">
					{#each docPages.filter((d) => d.group === group) as doc}
						<li><a href="{base}/docs/{doc.slug}/" class:menu-active={doc.slug === data.slug}>{doc.title}</a></li>
					{/each}
				</ul>
			</div>
		{/each}
	</aside>

	<article class="min-w-0">
		<h1 class="font-display text-4xl font-bold">{data.meta.title}</h1>
		<p class="mt-3 text-lg text-base-content/70">{data.meta.description}</p>
		<div class="divider"></div>
		<div class="prose-docs max-w-none">
			<data.content />
		</div>

		<nav class="mt-14 flex flex-wrap justify-between gap-3 border-t border-base-300 pt-6">
			{#if previous}
				<a href="{base}/docs/{previous.slug}/" class="btn btn-ghost btn-sm">← {previous.title}</a>
			{:else}
				<span></span>
			{/if}
			{#if next}
				<a href="{base}/docs/{next.slug}/" class="btn btn-ghost btn-sm">{next.title} →</a>
			{/if}
		</nav>
	</article>
</div>
