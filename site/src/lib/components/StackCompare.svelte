<script lang="ts">
	// What sits between an application and the glass: a kiosk built on a desktop
	// stack against a DeniseUI binary. Illustrative layers, not a benchmark.
	type Layer = { name: string; detail: string; tone: string; keep?: boolean };

	const desktop: Layer[] = [
		{ name: 'Your application', detail: 'the part you actually wrote', tone: 'primary', keep: true },
		{ name: 'Browser engine or managed runtime', detail: 'Chromium, Electron, a VM', tone: 'neutral' },
		{ name: 'UI framework', detail: 'and its event loop and repaint model', tone: 'neutral' },
		{ name: 'Compositor', detail: 'Wayland or X11', tone: 'neutral' },
		{ name: 'Desktop session', detail: 'login manager, services, a window manager', tone: 'neutral' },
		{ name: 'GPU userspace', detail: 'Mesa, EGL, drivers', tone: 'neutral' },
		{ name: 'Linux kernel', detail: 'DRM/KMS, input', tone: 'accent', keep: true }
	];

	let lean = $state(true);
</script>

<div class="card card-border bg-base-200/70 shadow-xl">
	<div class="card-body gap-4">
		<div role="tablist" class="tabs tabs-box self-start">
			<button role="tab" class="tab" class:tab-active={!lean} aria-selected={!lean} onclick={() => (lean = false)}>On a desktop stack</button>
			<button role="tab" class="tab" class:tab-active={lean} aria-selected={lean} onclick={() => (lean = true)}>With DeniseUI</button>
		</div>
		<div class="flex flex-col gap-2">
			{#each desktop as layer}
				{@const gone = lean && !layer.keep}
				<div
					class="layer overflow-hidden rounded-lg border px-4 transition-all duration-500"
					class:gone
					class:border-primary={layer.tone === 'primary'}
					class:bg-primary={layer.tone === 'primary'}
					class:text-primary-content={layer.tone === 'primary'}
					class:border-accent={layer.tone === 'accent'}
					class:border-base-300={layer.tone === 'neutral'}
					class:bg-base-100={layer.tone !== 'primary'}
				>
					<div class="flex items-baseline justify-between gap-3 py-2.5">
						<span class="font-medium">
							{layer.name}{#if lean && layer.tone === 'primary'}<span class="font-semibold"> + DeniseUI</span>{/if}
						</span>
						<span class="text-right text-xs opacity-70">
							{#if lean && layer.tone === 'primary'}one static binary{:else}{layer.detail}{/if}
						</span>
					</div>
				</div>
			{/each}
		</div>
		<p class="text-sm text-base-content/70">
			{#if lean}
				The binary opens <code class="font-mono">/dev/dri</code> (or <code class="font-mono">/dev/fb0</code>), reads <code class="font-mono">/dev/input</code>, and draws. Nothing to boot into, nothing to keep patched, nothing between a changed pixel and the page flip.
			{:else}
				Every layer here is a process to start, a thing to update, and a place where a frame can be lost. On a panel that only ever shows one application, most of them do nothing useful.
			{/if}
		</p>
	</div>
</div>

<style>
	.layer {
		max-height: 4rem;
	}
	.layer.gone {
		max-height: 0;
		opacity: 0;
		border-width: 0;
		margin-top: -0.5rem;
	}
	@media (prefers-reduced-motion: reduce) {
		.layer {
			transition: none;
		}
	}
</style>
