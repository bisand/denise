<script lang="ts">
	import Icon from './Icon.svelte';
	import { onMount } from 'svelte';

	let dark = $state(true);

	onMount(() => {
		const set = document.documentElement.getAttribute('data-theme');
		dark = set ? set === 'denise-dark' : matchMedia('(prefers-color-scheme: dark)').matches;
	});

	function toggle() {
		dark = !dark;
		const theme = dark ? 'denise-dark' : 'denise-light';
		document.documentElement.setAttribute('data-theme', theme);
		try {
			localStorage.setItem('denise-theme', theme);
		} catch {}
		window.dispatchEvent(new CustomEvent('denise-theme', { detail: { dark } }));
	}
</script>

<button class="btn btn-ghost btn-sm btn-square" onclick={toggle} aria-label={dark ? 'Switch to light theme' : 'Switch to dark theme'} title="Toggle theme">
	<Icon name={dark ? 'sun' : 'moon'} class="h-[18px] w-[18px]" />
</button>
