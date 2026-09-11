<script lang="ts">
	import '../app.css';
	import Nav from '$lib/components/Nav.svelte';
	import Footer from '$lib/components/Footer.svelte';
	let { children } = $props();
</script>

<div class="flex min-h-screen flex-col">
	<a href="#main" class="sr-only focus:not-sr-only focus:absolute focus:z-50 focus:m-2 focus:rounded focus:bg-base-100 focus:p-2">Skip to content</a>
	<Nav />
	<main id="main" class="flex-1">
		{@render children()}
	</main>
	<Footer />
</div>
