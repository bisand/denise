<script lang="ts">
	import { base } from '$app/paths';
	import Seo from '$lib/components/Seo.svelte';
	import LiveDemo from '$lib/components/LiveDemo.svelte';
	import Icon from '$lib/components/Icon.svelte';
	import { blob, tree } from '$lib/site';
	import type { DemoName } from '$lib/denise/runtime';

	let demo = $state<DemoName>('gallery');

	const tries: Record<DemoName, { title: string; body: string }[]> = {
		gallery: [
			{ title: 'Click the Name field and type', body: 'The on-screen keyboard comes up, because that is what a panel with nothing plugged into it does; the key at its bottom right puts it away. Your real keyboard types too — and only the field and its caret repaint.' },
			{ title: 'Drag the R, G or B slider', body: 'A seed colour changes, the theme is derived again, and everything repaints, because every widget names a role rather than a colour.' },
			{ title: 'Turn off “Awake”', body: 'The spinner stops asking for frames. What is left is the wall clock, which asks to be woken exactly when the next second starts.' },
			{ title: 'Open “Choose a mode”', body: 'A popup is a scene above the tree, and Escape or a click outside closes it. Tab and the arrow keys work everywhere.' },
			{ title: 'Press F2', body: 'The built-in themes, cycled — the same key the example binds on a desktop. The Light, Dark and Contrast buttons above do it from the page instead.' },
			{ title: 'Scroll the right-hand column', body: 'It is a viewport, and scrolling is the tree’s job: wheel, page keys, touch-drag, clipping and hit testing all agree because one reflow computes them.' }
		],
		editor: [
			{ title: 'Click a row, then edit the form', body: 'A row is a full-width Button with Labels on top. There was no table widget when this example was written.' },
			{ title: 'Press ↑ and ↓', body: 'Nine row nodes exist however many records there are. Scrolling changes what they show, not how many there are.' },
			{ title: 'Delete a record', body: 'The confirmation is a dimmed scene pushed over the tree. Nothing underneath is disabled; it just stops receiving input.' },
			{ title: 'Press Save', body: 'It reports that it cannot write people.csv. That is true: a browser tab has no file system, and the application says so rather than pretending.' }
		]
	};
</script>

<Seo title="Live demo" description="The DeniseUI widget gallery and record editor, compiled unchanged to WebAssembly and running in your browser, with every repainted rectangle outlined." />

<section class="relative overflow-hidden">
	<div class="glow pointer-events-none absolute inset-0 -z-10 opacity-70"></div>
	<div class="mx-auto max-w-7xl px-4 pt-14 pb-8">
		<span class="badge badge-soft badge-secondary">Live, in your browser</span>
		<h1 class="mt-3 max-w-3xl text-4xl font-bold sm:text-5xl">This is not a video. It is the toolkit.</h1>
		<p class="mt-4 max-w-3xl text-lg text-base-content/70">
			The <code class="font-mono text-[0.9em]">app.rs</code> from two of the repository’s examples, compiled byte for byte to WebAssembly.
			The same widgets, the same software rasteriser, the same damage tracker that drives a Raspberry Pi’s display, drawing into a canvas instead of a scanout buffer.
			Pink outlines are the rectangles it repaints.
		</p>
	</div>
</section>

<section class="mx-auto max-w-7xl px-4">
	<LiveDemo bind:demo />
</section>

<section class="mx-auto max-w-7xl px-4 pt-12">
	<div class="grid gap-8 lg:grid-cols-[minmax(0,1.4fr)_minmax(0,1fr)]">
		<div class="min-w-0">
			<h2 class="text-2xl font-semibold">Things to try</h2>
			<div class="mt-5 grid gap-3 sm:grid-cols-2">
				{#each tries[demo] as item, i}
					<div class="card card-border bg-base-200/60">
						<div class="card-body p-5">
							<div class="flex items-center gap-2 font-semibold">
								<span class="grid h-6 w-6 place-items-center rounded-full bg-primary/15 font-mono text-xs text-primary">{i + 1}</span>
								{item.title}
							</div>
							<p class="text-sm text-base-content/70">{item.body}</p>
						</div>
					</div>
				{/each}
			</div>
		</div>
		<div class="card card-border bg-base-200/60">
			<div class="card-body">
				<h2 class="card-title">How this runs</h2>
				<ul class="mt-1 space-y-3 text-sm text-base-content/75">
					<li class="flex gap-2"><Icon name="package" class="mt-0.5 h-4 w-4 shrink-0 text-primary" /> <span>A third backend beside the window and the kiosk ones: <a class="link" href={tree('site/wasm')}>site/wasm</a>, about three hundred lines, pulls in <a class="link" href={blob('examples/gallery/src/app.rs')}>gallery/app.rs</a> and <a class="link" href={blob('examples/table-editor/src/app.rs')}>table-editor/app.rs</a> by path.</span></li>
					<li class="flex gap-2"><Icon name="chip" class="mt-0.5 h-4 w-4 shrink-0 text-primary" /> <span>Built for <code class="font-mono">wasm32-wasip1</code>, so the applications keep using <code class="font-mono">std::time</code>. The page supplies a clock and nothing else. No wasm-bindgen, no JavaScript UI.</span></li>
					<li class="flex gap-2"><Icon name="layers" class="mt-0.5 h-4 w-4 shrink-0 text-primary" /> <span>Each frame, the page copies only the damaged rectangles onto the canvas, the way a DRM backend presents only what changed.</span></li>
					<li class="flex gap-2"><Icon name="gauge" class="mt-0.5 h-4 w-4 shrink-0 text-primary" /> <span>Between frames nothing runs. The page sleeps until the tree’s next deadline, or until you touch it, and stops entirely when the demo scrolls out of view.</span></li>
					<li class="flex gap-2"><Icon name="type" class="mt-0.5 h-4 w-4 shrink-0 text-primary" /> <span>Text is Noto Sans through the TrueType tier. Prefers reduced motion? The toolkit gets <code class="font-mono">Motion::None</code>.</span></li>
				</ul>
				<div class="card-actions mt-3">
					<a href="{base}/docs/how-it-works/" class="btn btn-sm btn-primary">How damage tracking works</a>
				</div>
			</div>
		</div>
	</div>
</section>
