<script lang="ts">
	import { page } from '$app/state';
	import { site } from '$lib/site';

	let { title = '', description = site.description }: { title?: string; description?: string } = $props();
	const full = $derived(title ? `${title} · ${site.name}` : `${site.name} · ${site.tagline}`);
	// Social cards need an absolute URL; the site's origin is only known once built for Pages.
	const origin = 'https://bisand.github.io';
</script>

<svelte:head>
	<title>{full}</title>
	<meta name="description" content={description} />
	<meta property="og:type" content="website" />
	<meta property="og:title" content={full} />
	<meta property="og:description" content={description} />
	<meta property="og:url" content="{origin}{page.url.pathname}" />
	<meta property="og:image" content="{origin}/denise/og.png" />
	<meta name="twitter:card" content="summary_large_image" />
</svelte:head>
