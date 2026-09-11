<script lang="ts">
	type Tab = { id: string; label: string; note: string; html: string };
	let { tabs, class: cls = '' }: { tabs: Tab[]; class?: string } = $props();
	let active = $state('');
	const current = $derived(tabs.find((t) => t.id === active) ?? tabs[0]);
</script>

<div class="overflow-hidden rounded-box border border-base-300 bg-base-200 shadow-xl {cls}">
	<div role="tablist" class="flex flex-wrap gap-1 border-b border-base-300 bg-base-300/40 px-2 py-1.5">
		{#each tabs as tab}
			<button
				role="tab"
				aria-selected={active === tab.id}
				class="rounded-md px-3 py-1 text-sm transition"
				class:bg-base-100={active === tab.id}
				class:font-medium={active === tab.id}
				class:text-base-content={active === tab.id}
				class:text-base-content-60={active !== tab.id}
				onclick={() => (active = tab.id)}
			>
				{tab.label}
			</button>
		{/each}
	</div>
	<div class="code-block">
		{@html current.html}
	</div>
	<p class="border-t border-base-300 px-5 py-3 text-sm text-base-content/70">{current.note}</p>
</div>
