<script lang="ts">
	import { onMount } from 'svelte';

	// Ten seconds of the `panel` demo on a Raspberry Pi 3 A+ at 1920×1080, left
	// untouched with a text field focused: twenty frames, one per caret blink.
	// The numbers are the README's measurement; the animation replays its shape.
	const BLINKS = 20;
	let played = $state(0);
	let root: HTMLDivElement;

	onMount(() => {
		if (matchMedia('(prefers-reduced-motion: reduce)').matches) {
			played = BLINKS;
			return;
		}
		let timer: ReturnType<typeof setInterval> | undefined;
		const io = new IntersectionObserver((entries) => {
			if (!entries[0].isIntersecting || timer) return;
			played = 0;
			timer = setInterval(() => {
				played += 1;
				if (played >= BLINKS) clearInterval(timer);
			}, 500);
		});
		io.observe(root);
		return () => {
			io.disconnect();
			clearInterval(timer);
		};
	});
</script>

<div bind:this={root} class="card card-border bg-base-200/70 shadow-xl">
	<div class="card-body">
		<div class="flex items-center justify-between text-xs text-base-content/60">
			<span class="font-mono">panel · Raspberry Pi 3 A+ · 1920×1080</span>
			<span class="font-mono">{(played / 2).toFixed(1)} s / 10 s</span>
		</div>
		<div class="relative mt-3 h-16 rounded-lg border border-base-300 bg-base-100">
			{#each Array(BLINKS) as _, i}
				<div
					class="absolute top-2 bottom-2 w-[3px] rounded-full transition-all duration-300"
					class:bg-secondary={i < played}
					class:bg-base-300={i >= played}
					style="left: calc({(i + 0.5) * 5}% - 1px)"
				></div>
			{/each}
		</div>
		<div class="mt-1 flex justify-between font-mono text-[11px] text-base-content/40">
			<span>0 s</span><span>caret blinks every 500 ms; nothing else wakes it</span><span>10 s</span>
		</div>
		<div class="mt-5 grid grid-cols-3 gap-3 text-center">
			<div class="rounded-lg bg-base-100 p-3">
				<div class="font-display text-3xl font-bold text-secondary">20</div>
				<div class="text-xs text-base-content/60">frames drawn</div>
			</div>
			<div class="rounded-lg bg-base-100 p-3">
				<div class="font-display text-3xl font-bold text-primary">20</div>
				<div class="text-xs text-base-content/60">wake-ups</div>
			</div>
			<div class="rounded-lg bg-base-100 p-3">
				<div class="font-display text-3xl font-bold text-accent">80 ms</div>
				<div class="text-xs text-base-content/60">CPU in total</div>
			</div>
		</div>
		<p class="mt-3 text-xs text-base-content/60">
			Most of those 80 ms are the two full repaints every double-buffered swapchain owes at start-up. The loop blocks in <code class="font-mono">poll</code> on the input descriptors and the caret deadline; with nothing focused there is no deadline, and it blocks indefinitely.
		</p>
	</div>
</div>
