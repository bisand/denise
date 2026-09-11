<script lang="ts">
	import { base } from '$app/paths';
	import { onMount, tick } from 'svelte';
	import { DeniseHost, loadModule, type DemoName, type FrameInfo } from '$lib/denise/runtime';
	import Icon from './Icon.svelte';

	let {
		demo = $bindable<DemoName>('gallery'),
		switcher = true,
		flashing = $bindable(true),
		class: cls = ''
	}: { demo?: DemoName; switcher?: boolean; flashing?: boolean; class?: string } = $props();

	// The size each application's layout is written for, in logical pixels.
	const layouts = {
		gallery: { w: 1280, h: 800, title: 'examples/gallery', poster: 'gallery-2x.webp' },
		editor: { w: 1000, h: 470, title: 'examples/table-editor', poster: 'table-editor.webp' }
	} as const;

	let frame: HTMLDivElement;
	let canvas: HTMLCanvasElement;
	let overlay: HTMLCanvasElement;
	let host: DeniseHost | null = null;

	let status = $state<'idle' | 'loading' | 'running' | 'error'>('idle');
	let error = $state('');
	let theme = $state(1);
	let frames = $state(0);
	let last = $state<{ rects: number; pixels: number; pct: number } | null>(null);
	let wake = $state(-1);
	let fullscreen = $state(false);

	const layout = $derived(layouts[demo]);

	const siteIsLight = () => {
		const set = document.documentElement.getAttribute('data-theme');
		return set ? set === 'denise-light' : !matchMedia('(prefers-color-scheme: dark)').matches;
	};

	// Paint flashing: every rectangle the tree reported, fading out, drawn on a
	// canvas above the application's. The overlay is the page's; the rectangles
	// are the toolkit's own damage.
	type Flash = { x: number; y: number; w: number; h: number; at: number };
	let flashes: Flash[] = [];
	let flashRaf = 0;
	const FADE = 650;

	function onFrame(info: FrameInfo) {
		frames += 1;
		last = { rects: info.rects.length, pixels: info.pixels, pct: (info.pixels / info.surface) * 100 };
		if (!flashing) return;
		if (overlay.width !== info.width || overlay.height !== info.height) {
			overlay.width = info.width;
			overlay.height = info.height;
		}
		const at = performance.now();
		// A spinner repaints the same rectangle every frame; restarting its fade
		// rather than stacking another keeps what is underneath visible.
		const key = (r: { x: number; y: number; w: number; h: number }) => `${r.x},${r.y},${r.w},${r.h}`;
		const fresh = new Set(info.rects.map(key));
		flashes = flashes.filter((f) => !fresh.has(key(f)));
		for (const r of info.rects) flashes.push({ ...r, at });
		if (!flashRaf) flashRaf = requestAnimationFrame(drawFlashes);
	}

	function drawFlashes() {
		flashRaf = 0;
		const ctx = overlay.getContext('2d');
		if (!ctx) return;
		const now = performance.now();
		ctx.clearRect(0, 0, overlay.width, overlay.height);
		flashes = flashes.filter((f) => now - f.at < FADE);
		const line = Math.max(1, Math.round(overlay.width / 700));
		for (const f of flashes) {
			const a = 1 - (now - f.at) / FADE;
			ctx.fillStyle = `rgba(243, 139, 168, ${0.22 * a})`;
			ctx.fillRect(f.x, f.y, f.w, f.h);
			ctx.strokeStyle = `rgba(243, 139, 168, ${0.95 * a})`;
			ctx.lineWidth = line;
			ctx.strokeRect(f.x + line / 2, f.y + line / 2, f.w - line, f.h - line);
		}
		if (flashes.length) flashRaf = requestAnimationFrame(drawFlashes);
	}

	$effect(() => {
		if (!flashing && overlay) {
			flashes = [];
			overlay.getContext('2d')?.clearRect(0, 0, overlay.width, overlay.height);
		}
	});

	async function start() {
		if (status === 'loading' || status === 'running') return;
		status = 'loading';
		try {
			const module = await loadModule(`${base}/wasm/denise.wasm`);
			const light = siteIsLight();
			theme = light ? 0 : 1;
			host = await DeniseHost.create(canvas, module, {
				demo,
				light,
				logicalWidth: layout.w,
				onFrame,
				onSleep: (ms) => (wake = ms)
			});
			status = 'running';
		} catch (e) {
			status = 'error';
			error = e instanceof Error ? e.message : String(e);
		}
	}

	async function choose(next: DemoName) {
		if (next === demo) return;
		demo = next;
		frames = 0;
		last = null;
		await tick();
		host?.restart({ demo: next, logicalWidth: layouts[next].w, light: theme === 0 });
		if (theme === 2) host?.setTheme(2);
		canvas.focus({ preventScroll: true });
	}

	function setTheme(index: number) {
		theme = index;
		host?.setTheme(index);
	}

	function toggleFullscreen() {
		if (document.fullscreenElement) document.exitFullscreen();
		else frame.requestFullscreen?.();
	}

	onMount(() => {
		const seen = new IntersectionObserver(
			(entries) => {
				if (entries.some((e) => e.isIntersecting)) {
					seen.disconnect();
					start();
				}
			},
			{ rootMargin: '300px' }
		);
		seen.observe(frame);

		const followSite = (e: Event) => {
			const dark = (e as CustomEvent<{ dark: boolean }>).detail.dark;
			if (theme !== 2) setTheme(dark ? 1 : 0);
		};
		window.addEventListener('denise-theme', followSite);
		const onFullscreen = () => (fullscreen = document.fullscreenElement === frame);
		document.addEventListener('fullscreenchange', onFullscreen);

		return () => {
			seen.disconnect();
			window.removeEventListener('denise-theme', followSite);
			document.removeEventListener('fullscreenchange', onFullscreen);
			cancelAnimationFrame(flashRaf);
			host?.destroy();
		};
	});

	const fmt = new Intl.NumberFormat('en');
</script>

<div bind:this={frame} class="live overflow-hidden rounded-2xl border border-base-300 bg-base-200 shadow-2xl shadow-primary/10 {cls}" class:is-fullscreen={fullscreen}>
	<div class="flex flex-wrap items-center gap-x-3 gap-y-2 border-b border-base-300 bg-base-300/40 px-3 py-2 text-sm">
		<div class="flex items-center gap-1.5" aria-hidden="true">
			<span class="h-3 w-3 rounded-full bg-error/80"></span>
			<span class="h-3 w-3 rounded-full bg-warning/80"></span>
			<span class="h-3 w-3 rounded-full bg-success/80"></span>
		</div>
		{#if switcher}
			<div role="tablist" aria-label="Application" class="tabs tabs-box tabs-xs bg-base-100/60">
				<button role="tab" class="tab" class:tab-active={demo === 'gallery'} aria-selected={demo === 'gallery'} onclick={() => choose('gallery')}>Gallery</button>
				<button role="tab" class="tab" class:tab-active={demo === 'editor'} aria-selected={demo === 'editor'} onclick={() => choose('editor')}>Record editor</button>
			</div>
		{:else}
			<span class="font-mono text-xs text-base-content/60">{layout.title}</span>
		{/if}
		<span class="badge badge-xs badge-soft badge-accent hidden font-mono md:inline-flex">wasm32 · the real toolkit</span>
		<div class="ml-auto flex items-center gap-2">
			<div class="join hidden sm:inline-flex" role="group" aria-label="Theme">
				{#each ['Light', 'Dark', 'Contrast'] as label, i}
					<button class="btn join-item btn-xs" class:btn-active={theme === i} aria-pressed={theme === i} onclick={() => setTheme(i)}>{label}</button>
				{/each}
			</div>
			<label class="label cursor-pointer gap-1.5 text-xs" title="Outline every rectangle the toolkit repaints">
				<input type="checkbox" class="toggle toggle-xs toggle-secondary" bind:checked={flashing} />
				Paint flashing
			</label>
			<button class="btn btn-ghost btn-xs btn-square" onclick={toggleFullscreen} aria-label="Full screen" title="Full screen">
				<Icon name="expand" class="h-4 w-4" />
			</button>
		</div>
	</div>

	<div class="stage relative mx-auto w-full" style="aspect-ratio: {layout.w} / {layout.h}">
		{#if status !== 'running'}
			<img src="{base}/screenshots/{layout.poster}" alt="" class="absolute inset-0 h-full w-full object-cover opacity-60 blur-[1px]" />
		{/if}
		<canvas
			bind:this={canvas}
			tabindex="0"
			aria-label="The DeniseUI {demo === 'gallery' ? 'widget gallery' : 'record editor'}, running live. Click to interact; keyboard input goes to it while focused."
			class="absolute inset-0 h-full w-full outline-none focus-visible:ring-2 focus-visible:ring-primary"
			class:invisible={status !== 'running'}
		></canvas>
		<canvas bind:this={overlay} class="pointer-events-none absolute inset-0 h-full w-full" aria-hidden="true"></canvas>
		{#if status === 'loading' || status === 'idle'}
			<div class="absolute inset-0 grid place-items-center">
				<div class="flex items-center gap-3 rounded-full bg-base-100/90 px-5 py-2.5 text-sm shadow-lg">
					<span class="loading loading-spinner loading-sm text-primary"></span>
					Loading the toolkit (about 860 KB)…
				</div>
			</div>
		{:else if status === 'error'}
			<div class="absolute inset-0 grid place-items-center p-6">
				<div class="alert alert-error max-w-md">
					<span>The live demo could not start: {error}. The screenshot behind this is what it draws.</span>
				</div>
			</div>
		{/if}
	</div>

	<div class="flex flex-wrap items-center gap-x-5 gap-y-1 border-t border-base-300 px-4 py-2 font-mono text-xs text-base-content/70" aria-live="off">
		<span><span class="text-base-content">{fmt.format(frames)}</span> frames drawn</span>
		{#if last}
			<span>
				last frame: <span class="text-secondary">{last.rects} {last.rects === 1 ? 'rectangle' : 'rectangles'}</span>,
				{fmt.format(last.pixels)} px
				<span class="text-base-content">({last.pct < 0.1 ? last.pct.toFixed(3) : last.pct.toFixed(1)}% of the surface)</span>
			</span>
		{/if}
		<span class="ml-auto">
			{#if status !== 'running'}
				—
			{:else if wake < 0}
				<span class="text-success">● asleep</span>: no timer, waiting for input
			{:else}
				next wake in {wake} ms
			{/if}
		</span>
	</div>
</div>

<style>
	.is-fullscreen {
		display: flex;
		flex-direction: column;
		border-radius: 0;
	}
	.is-fullscreen .stage {
		flex: 1 1 auto;
		width: auto;
		height: 100%;
		max-width: 100%;
		max-height: calc(100vh - 6rem);
	}
</style>
