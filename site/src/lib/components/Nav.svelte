<script lang="ts">
	import { page } from '$app/state';
	import { base } from '$app/paths';
	import { nav, site, version } from '$lib/site';
	import ThemeToggle from './ThemeToggle.svelte';
	import Icon from './Icon.svelte';

	const isActive = (href: string) => page.url.pathname.startsWith(base + href.replace(/\/$/, ''));
	let open = $state(false);
</script>

<header class="sticky top-0 z-40 border-b border-base-300/70 bg-base-100/75 backdrop-blur-lg">
	<div class="navbar mx-auto max-w-7xl gap-2 px-4">
		<div class="navbar-start gap-3">
			<a href="{base}/" class="flex items-center gap-2.5 font-display text-lg font-semibold tracking-tight">
				<img src="{base}/logo.svg" alt="" width="30" height="30" class="h-[30px] w-[30px]" />
				{site.name}
			</a>
			<a href={site.latestRelease} class="badge badge-sm badge-soft badge-primary hidden font-mono sm:inline-flex" rel="noopener">v{version}</a>
		</div>
		<nav class="navbar-center hidden lg:flex" aria-label="Main">
			<ul class="menu menu-horizontal gap-1 px-1 text-[15px]">
				{#each nav as item}
					<li><a href="{base}{item.href}" class:menu-active={isActive(item.href)}>{item.label}</a></li>
				{/each}
			</ul>
		</nav>
		<div class="navbar-end gap-1">
			<ThemeToggle />
			<a href={site.github} class="btn btn-ghost btn-sm" rel="noopener" target="_blank" aria-label="DeniseUI on GitHub">
				<Icon name="github" class="h-[18px] w-[18px]" />
				<span class="hidden sm:inline">GitHub</span>
			</a>
			<a href="{base}/docs/getting-started/" class="btn btn-primary btn-sm hidden sm:inline-flex">Get started</a>
			<button class="btn btn-ghost btn-square btn-sm lg:hidden" aria-label="Menu" aria-expanded={open} onclick={() => (open = !open)}>
				<Icon name="menu" />
			</button>
		</div>
	</div>
	{#if open}
		<nav class="border-t border-base-300 bg-base-100 px-4 py-2 lg:hidden" aria-label="Main">
			<ul class="menu w-full">
				{#each nav as item}
					<li><a href="{base}{item.href}" onclick={() => (open = false)} class:menu-active={isActive(item.href)}>{item.label}</a></li>
				{/each}
				<li><a href="{base}/docs/getting-started/" onclick={() => (open = false)}>Get started</a></li>
			</ul>
		</nav>
	{/if}
</header>
