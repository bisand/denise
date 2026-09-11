<script lang="ts">
	import { base } from '$app/paths';
	import Seo from '$lib/components/Seo.svelte';
	import Icon from '$lib/components/Icon.svelte';
	import Shot from '$lib/components/Shot.svelte';
	import LiveDemo from '$lib/components/LiveDemo.svelte';
	import { widgets, groups, treeOwned } from '$lib/widgets';
	import { blob } from '$lib/site';

	let filter = $state('All');
	const shown = $derived(filter === 'All' ? widgets : widgets.filter((w) => w.group === filter));
</script>

<Seo title="Widgets" description="The twenty-eight DeniseUI widgets, what each one is for, and the parts the widget tree owns: modal scenes, drawers, shelves, tooltips, toasts, popups, scrolling and focus." />

<section class="mx-auto max-w-7xl px-4 pt-14 pb-10">
	<span class="badge badge-soft badge-secondary">Twenty-eight, and counting</span>
	<h1 class="mt-3 text-4xl font-bold sm:text-5xl">Widgets</h1>
	<p class="mt-4 max-w-3xl text-lg text-base-content/70">
		Each is a small thing that draws itself and answers events. The hard parts — modality, overlays, scrolling, focus order, the pointer — belong to the tree, which is why a widget stays small and why anything assembled from them behaves like the rest.
	</p>
	<p class="mt-3 max-w-3xl text-base-content/60">
		Every widget below is on this page, live. New ones arrive one at a time; the descriptions are the modules’ own.
	</p>
</section>

<section class="mx-auto max-w-7xl px-4">
	<LiveDemo switcher={false} />
</section>

<section class="mx-auto max-w-7xl px-4 pt-14">
	<div role="tablist" class="tabs tabs-box w-fit">
		{#each ['All', ...groups] as group}
			<button role="tab" class="tab" class:tab-active={filter === group} aria-selected={filter === group} onclick={() => (filter = group)}>{group}</button>
		{/each}
	</div>

	<div class="mt-6 grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
		{#each shown as w (w.id)}
			<div id={w.id} class="card card-border scroll-mt-24 bg-base-200/60 transition hover:border-primary/40">
				<div class="card-body p-5">
					<div class="flex items-start justify-between gap-2">
						<h2 class="font-display text-lg font-semibold">{w.name}</h2>
						<a href={blob(`denise-ui/src/widgets/${w.file}`)} rel="noopener" class="text-base-content/40 hover:text-primary" aria-label="{w.name} source on GitHub" title="Source">
							<Icon name="code" class="h-4 w-4" />
						</a>
					</div>
					<p class="text-sm text-base-content/80">{w.blurb}</p>
					{#if w.more}
						<p class="text-sm text-base-content/60">{w.more}</p>
					{/if}
					<div class="mt-1 text-xs text-base-content/40">{w.group}</div>
				</div>
			</div>
		{/each}
	</div>
</section>

<section class="mx-auto max-w-7xl px-4 py-20">
	<div class="grid gap-10 lg:grid-cols-[1fr_1.1fr]">
		<div>
			<span class="badge badge-soft badge-accent">Not widgets</span>
			<h2 class="mt-3 text-3xl font-bold">What the tree owns</h2>
			<p class="mt-4 text-base-content/70">
				These are properties of the scene graph rather than things you place. That is what keeps a modal from needing flags, a drawer from needing a second surface, and an on-screen keyboard from stealing the caret out of the field it is typing into.
			</p>
			<div class="mt-6 space-y-3">
				{#each treeOwned as item}
					<div class="rounded-box border border-base-300 bg-base-200/50 p-4">
						<div class="font-medium">{item.name}</div>
						<p class="mt-1 text-sm text-base-content/65">{item.what}</p>
					</div>
				{/each}
			</div>
		</div>
		<div class="space-y-6">
			<Shot
				src="browser-form.webp"
				alt="A small web browser rendering an HTML form, with its inputs, checkbox, radio group, dropdown and buttons all visibly Denise widgets"
				caption="examples/browser — a page’s form controls are the actual TextInput, Checkbox, RadioGroup and Select, submitting for real"
			/>
			<div class="card card-border bg-base-200/50">
				<div class="card-body">
					<h3 class="card-title text-base">Missing one? Assemble it.</h3>
					<p class="text-sm text-base-content/70">
						There was no table widget when the record editor was written, so a row became a full-width <code class="font-mono text-[0.9em]">Button</code> with <code class="font-mono text-[0.9em]">Label</code>s on top: labels are not interactive, so a click falls through them and arrives as a selection. That is how most of the widgets you will miss get built — and some of them, like <code class="font-mono text-[0.9em]">Table</code>, later became real ones.
					</p>
					<div class="card-actions">
						<a href="{base}/showcase/" class="btn btn-sm">See what has been built</a>
					</div>
				</div>
			</div>
		</div>
	</div>
</section>
