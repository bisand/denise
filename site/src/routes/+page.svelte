<script lang="ts">
	import { base } from '$app/paths';
	import Seo from '$lib/components/Seo.svelte';
	import Icon from '$lib/components/Icon.svelte';
	import Shot from '$lib/components/Shot.svelte';
	import LiveDemo from '$lib/components/LiveDemo.svelte';
	import CodeTabs from '$lib/components/CodeTabs.svelte';
	import CopyLine from '$lib/components/CopyLine.svelte';
	import StackCompare from '$lib/components/StackCompare.svelte';
	import IdleMeter from '$lib/components/IdleMeter.svelte';
	import { site, crates, backends, blob, tree, minor } from '$lib/site';
	import { widgets } from '$lib/widgets';

	let { data } = $props();

	const features = [
		{
			icon: 'layers',
			title: 'Damage tracking, not dirty flags',
			body: 'Type into a field and the field repaints. There are no invalidate() calls, no repaint bookkeeping and nothing for you to remember: the tree knows what changed and paints only that.'
		},
		{
			icon: 'monitor',
			title: 'Straight to the display',
			body: 'DRM/KMS with async page flips and a hardware cursor plane, or fbdev where there is no /dev/dri. Input comes from evdev, the console is muted, and it is handed back on exit.'
		},
		{
			icon: 'chip',
			title: 'A no_std core',
			body: 'denise, denise-render, denise-text and denise-ui all build no_std + alloc, with zero allocation in the render hot path. CI asserts the core’s dependency tree contains no platform crates.'
		},
		{
			icon: 'shield',
			title: 'forbid(unsafe_code) in the core',
			body: 'unsafe lives in backend crates only, every block with a SAFETY comment. CI runs cargo deny, Miri over the C ABI, and fuzzes the image decoders and the ABI.'
		},
		{
			icon: 'palette',
			title: 'Themes from nine seeds',
			body: 'Widgets name roles, never colours. A theme is derived from nine seed colours with contrast aimed at WCAG AA by construction, and swapping one is a single call.'
		},
		{
			icon: 'type',
			title: 'Text in three tiers',
			body: 'The built-in 8×8 bitmap font costs nothing, TrueType through fontdue adds about 145 KB, and full shaping through cosmic-text adds 3.1 MB. You pay for what you draw.'
		},
		{
			icon: 'keyboard',
			title: 'A keyboard for panels with none',
			body: 'denise-keyboard emits exactly what evdev would, in the layout the machine is configured for — US, Norwegian or German — with dead keys and long-press alternates.'
		},
		{
			icon: 'plug',
			title: 'Embeddable in what you already ship',
			body: 'A stable C ABI, an NSView on macOS, a child HWND on Windows, and a registered, scriptable ActiveX control for the hosts that still want one.'
		}
	];

	const stats = [
		{ value: '80 ms', label: 'CPU for ten idle seconds on a Pi 3 A+' },
		{ value: '28', label: 'widgets, all in the live demo' },
		{ value: '19', label: 'crates on crates.io' },
		{ value: '0', label: 'compositors required' }
	];
</script>

<Seo />

<!-- Hero -->
<section class="relative overflow-hidden border-b border-base-300">
	<div class="glow pointer-events-none absolute inset-0 -z-10"></div>
	<div class="bg-grid pointer-events-none absolute inset-0 -z-10 opacity-[0.35] [mask-image:radial-gradient(ellipse_at_top,black,transparent_75%)]"></div>
	<div class="mx-auto grid max-w-7xl gap-10 px-4 pt-14 pb-12 lg:grid-cols-[minmax(0,1fr)_minmax(0,1.15fr)] lg:items-center lg:pt-20">
		<div class="min-w-0">
			<div class="flex flex-wrap items-center gap-2">
				<span class="badge badge-soft badge-primary font-mono">Rust</span>
				<span class="badge badge-soft badge-secondary">embedded Linux</span>
				<span class="badge badge-soft badge-accent">no desktop required</span>
			</div>
			<h1 class="mt-5 text-5xl leading-[1.05] font-bold tracking-tight sm:text-6xl">
				Pixels <span class="text-gradient">straight to the panel</span>.
			</h1>
			<p class="mt-5 max-w-xl text-lg text-base-content/75">
				DeniseUI is a direct-rendering UI toolkit for kiosks, digital signage, industrial HMIs, Raspberry Pi panels and in-vehicle displays.
				No X11. No Wayland. No browser engine. No managed runtime. One static binary that opens the display, draws, and reads input.
			</p>
			<div class="mt-7 flex flex-wrap gap-3">
				<a href="{base}/docs/getting-started/" class="btn btn-primary btn-lg">
					<Icon name="bolt" class="h-5 w-5" /> Get started
				</a>
				<a href="{base}/demo/" class="btn btn-lg">
					<Icon name="play" class="h-5 w-5" /> Try it in your browser
				</a>
			</div>
			<div class="mt-7 w-full max-w-md min-w-0">
				<div class="code-block overflow-hidden rounded-box border border-base-300 bg-base-200">
					{@html data.install}
				</div>
				<p class="mt-2 text-xs text-base-content/50">
					Rust {site.msrv}+ · MIT · aarch64, armv7 and x86-64 ·
					<a href={site.crates} class="link" rel="noopener">crates.io</a> ·
					<a href={site.docsrs} class="link" rel="noopener">docs.rs</a>
				</p>
			</div>
		</div>
		<div>
			<LiveDemo />
			<p class="mt-3 text-center text-sm text-base-content/60">
				Really running: the widget gallery, compiled to WebAssembly. Click it, type in it, drag a slider.
			</p>
		</div>
	</div>
</section>

<!-- Stats -->
<section class="border-b border-base-300 bg-base-200/50">
	<div class="mx-auto grid max-w-7xl grid-cols-2 gap-6 px-4 py-8 md:grid-cols-4">
		{#each stats as stat}
			<div class="text-center">
				<div class="font-display text-3xl font-bold text-primary sm:text-4xl">{stat.value}</div>
				<div class="mt-1 text-xs text-base-content/60 sm:text-sm">{stat.label}</div>
			</div>
		{/each}
	</div>
</section>

<!-- The stack -->
<section class="mx-auto max-w-7xl px-4 py-20">
	<div class="grid items-center gap-12 lg:grid-cols-2">
		<div>
			<span class="badge badge-soft badge-primary">The premise</span>
			<h2 class="mt-3 text-3xl font-bold sm:text-4xl">A panel shows one application. Why boot a desktop for it?</h2>
			<p class="mt-4 text-base-content/75">
				The usual way to put a screen on an embedded Linux board is to bring up a whole desktop session and then hide it: a compositor, a window manager, a GPU stack, often a browser engine, all to draw one form that never moves.
				DeniseUI deletes that. Your application links the toolkit, opens the display device itself and draws into the scanout buffer.
			</p>
			<ul class="mt-6 space-y-2.5 text-sm">
				<li class="flex gap-2.5"><Icon name="check" class="mt-0.5 h-4 w-4 shrink-0 text-success" /> Boots to your screen in one process, with no session to keep alive</li>
				<li class="flex gap-2.5"><Icon name="check" class="mt-0.5 h-4 w-4 shrink-0 text-success" /> Nothing between a changed pixel and the page flip</li>
				<li class="flex gap-2.5"><Icon name="check" class="mt-0.5 h-4 w-4 shrink-0 text-success" /> A far smaller thing to keep patched for ten years in the field</li>
				<li class="flex gap-2.5"><Icon name="check" class="mt-0.5 h-4 w-4 shrink-0 text-success" /> The same binary builds for a window while you develop</li>
			</ul>
		</div>
		<StackCompare />
	</div>
</section>

<!-- Code -->
<section class="border-y border-base-300 bg-base-200/40">
	<div class="mx-auto max-w-7xl px-4 py-20">
		<div class="grid gap-12 lg:grid-cols-[minmax(0,1fr)_minmax(0,1.1fr)] lg:items-center">
			<div class="min-w-0">
				<span class="badge badge-soft badge-secondary">The programming model</span>
				<h2 class="mt-3 text-3xl font-bold sm:text-4xl">Widgets do not run callbacks</h2>
				<p class="mt-4 text-base-content/75">
					A button holds a value of <em>your</em> type and emits it when pressed, so every state change happens in one <code class="rounded bg-base-300/60 px-1.5 font-mono text-[0.9em]">match</code> you wrote rather than in a closure somewhere else.
					There are no dirty flags, no repaint bookkeeping and nothing to invalidate. The way you use it is by not doing anything.
				</p>
				<p class="mt-4 text-base-content/75">
					The whole runnable version is <a class="link link-primary" href={blob('examples/hello/src/main.rs')} rel="noopener">examples/hello</a>: eighty lines, half of them comments. The same screen can be a file instead, drawn in the designer and loaded at run time.
				</p>
				<div class="mt-6 flex flex-wrap gap-3">
					<a href="{base}/docs/getting-started/" class="btn btn-sm btn-primary">Read the guide</a>
					<a href={tree('examples')} class="btn btn-sm" rel="noopener"><Icon name="github" class="h-4 w-4" /> All the examples</a>
				</div>
			</div>
			<CodeTabs tabs={data.tabs} class="min-w-0" />
		</div>
	</div>
</section>

<!-- Features -->
<section class="mx-auto max-w-7xl px-4 py-20">
	<div class="max-w-2xl">
		<span class="badge badge-soft badge-accent">What it gives you</span>
		<h2 class="mt-3 text-3xl font-bold sm:text-4xl">Built for machines that run for a year</h2>
		<p class="mt-4 text-base-content/70">Every one of these is in the repository today, measured on real hardware rather than asserted.</p>
	</div>
	<div class="mt-10 grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
		{#each features as f}
			<div class="card card-border bg-base-200/60 transition hover:border-primary/40">
				<div class="card-body p-5">
					<div class="mb-1 grid h-10 w-10 place-items-center rounded-lg bg-primary/15 text-primary"><Icon name={f.icon} /></div>
					<h3 class="font-display text-base font-semibold">{f.title}</h3>
					<p class="text-sm text-base-content/70">{f.body}</p>
				</div>
			</div>
		{/each}
	</div>
</section>

<!-- Idle -->
<section class="border-y border-base-300 bg-base-200/40">
	<div class="mx-auto max-w-7xl px-4 py-20">
		<div class="grid items-center gap-12 lg:grid-cols-2">
			<div>
				<span class="badge badge-soft badge-success">Measured</span>
				<h2 class="mt-3 text-3xl font-bold sm:text-4xl">What it costs when nothing happens</h2>
				<p class="mt-4 text-base-content/75">
					The number that matters for a panel left on for a year. On a Raspberry Pi 3 A+ at 1920×1080, the <code class="font-mono text-[0.9em]">panel</code> demo left untouched for ten seconds — with a text field focused, so its caret is blinking — draws twenty frames, wakes twenty times, and spends eighty milliseconds of CPU in total.
				</p>
				<p class="mt-4 text-base-content/75">
					Move the pointer and it repaints two cursor-sized rectangles, not a megapixel. Set <code class="font-mono text-[0.9em]">Motion::None</code> and animations land at once and the tree asks for no wake at all — which is both the reduced-motion answer and the tightest power budget.
				</p>
				<a href="{base}/docs/how-it-works/" class="btn btn-sm mt-6 btn-primary">How the damage tracker works</a>
			</div>
			<IdleMeter />
		</div>
	</div>
</section>

<!-- Widgets -->
<section class="mx-auto max-w-7xl px-4 py-20">
	<div class="grid gap-12 lg:grid-cols-[1fr_1.2fr] lg:items-center">
		<div>
			<span class="badge badge-soft badge-secondary">The widget set</span>
			<h2 class="mt-3 text-3xl font-bold sm:text-4xl">Twenty-eight widgets, and the tree owns the hard parts</h2>
			<p class="mt-4 text-base-content/75">
				Tooltips, toasts, drawers, modal scenes, popups, scrolling, focus order and the on-screen keyboard belong to the tree, so a widget stays a small thing that draws and answers events.
			</p>
			<div class="mt-6 flex flex-wrap gap-1.5">
				{#each widgets as w}
					<a href="{base}/widgets/#{w.id}" class="badge badge-outline badge-sm hover:badge-primary">{w.name}</a>
				{/each}
			</div>
			<a href="{base}/widgets/" class="btn btn-sm mt-7 btn-primary">See every widget <Icon name="arrow" class="h-4 w-4" /></a>
		</div>
		<Shot
			src="gallery-keyboard-2x.webp"
			alt="The gallery with the on-screen keyboard up: a theme editor on the left, role buttons, form controls, a progress ring, ratings and a spinner, and a full keyboard along the bottom"
			caption="examples/gallery — every widget live, with a theme editor driving Ui::set_theme"
		/>
	</div>
</section>

<!-- Designer -->
<section class="border-y border-base-300 bg-base-200/40">
	<div class="mx-auto max-w-7xl px-4 py-20">
		<div class="grid items-center gap-12 lg:grid-cols-[1.2fr_1fr]">
			<Shot
				src="designer-2x.webp"
				alt="The DeniseUI designer: a palette and outline on the left, the reference form on the canvas with a selected slider showing eight handles and an alignment guide, and the slider’s properties on the right"
				caption="The designer is itself a Denise application — not Tauri, not a web page"
			/>
			<div>
				<span class="badge badge-soft badge-primary">Draw it instead</span>
				<h2 class="mt-3 text-3xl font-bold sm:text-4xl">A form designer, like 1995 promised</h2>
				<p class="mt-4 text-base-content/75">
					Drag widgets out of a palette, snap them to guides, edit every property in the inspector, and press <kbd class="kbd kbd-sm">F5</kbd> to run the form. The canvas draws with the same code that will draw it on the panel, so what you see is what ships, to the pixel.
				</p>
				<p class="mt-4 text-base-content/75">
					It writes a <code class="font-mono text-[0.9em]">.dform</code> file: text, readable in a diff, hand-editable, comments preserved — and every edit undoes byte for byte, because the file is the document.
				</p>
				<div class="mt-6 flex flex-wrap gap-3">
					<a href="{base}/designer/" class="btn btn-primary btn-sm"><Icon name="download" class="h-4 w-4" /> Download the designer</a>
					<a href="{base}/docs/forms-and-designer/" class="btn btn-sm">The .dform file</a>
				</div>
			</div>
		</div>
	</div>
</section>

<!-- Real hardware -->
<section class="mx-auto max-w-7xl px-4 py-20">
	<div class="max-w-2xl">
		<span class="badge badge-soft badge-warning">Photographs, not renders</span>
		<h2 class="mt-3 text-3xl font-bold sm:text-4xl">A Raspberry Pi with no desktop at all</h2>
		<p class="mt-4 text-base-content/70">
			The third machine has no window system, so there is nothing to screenshot. These are photographs of a Pi 3 A+ driving a 1920×1080 display over DRM/KMS: same binaries, rebuilt with <code class="font-mono text-[0.9em]">--no-default-features --features kiosk</code>, which is the only thing that changes.
		</p>
	</div>
	<div class="mt-10 grid gap-5 md:grid-cols-3">
		<Shot src="pi-gallery-keyboard.webp" alt="The gallery filling a monitor attached to a Raspberry Pi, in the dark theme, with the on-screen keyboard up" caption="gallery, dark theme, keyboard up" />
		<Shot src="pi-table-editor-alternates.webp" alt="The record editor on the same monitor in the light theme, with a held key showing a framed strip of accented letters above it" caption="table-editor, mid-gesture: holding e offers é è ê ë" />
		<Shot src="pi-hello.webp" alt="The hello example centred on the same monitor, drawn in the built-in bitmap font" caption="hello, in the built-in 8×8 bitmap font" />
	</div>
	<p class="mx-auto mt-6 max-w-3xl text-center text-sm text-base-content/60">
		The moiré is the camera against the panel, not the renderer. The keyboard reads the layout off the board: this one is configured Norwegian, so the home row ends ø æ.
	</p>
</section>

<!-- Where it runs -->
<section class="border-y border-base-300 bg-base-200/40">
	<div class="mx-auto max-w-7xl px-4 py-20">
		<div class="grid gap-12 lg:grid-cols-[1fr_1.3fr]">
			<div>
				<span class="badge badge-soft badge-info">Backends</span>
				<h2 class="mt-3 text-3xl font-bold sm:text-4xl">Where it runs</h2>
				<p class="mt-4 text-base-content/75">
					The application picks its backend at compile time, with a cargo feature. The toolkit does not choose and offers no way to: <code class="font-mono text-[0.85em]">aarch64-unknown-linux-gnu</code> is the same target on a kiosk Pi and on a Pi running the desktop image, so a probe in a library would be wrong half the time.
				</p>
				<p class="mt-4 text-base-content/75">A kiosk build therefore never compiles winit at all.</p>
			</div>
			<div class="overflow-x-auto rounded-box border border-base-300 bg-base-100">
				<table class="table table-sm">
					<thead>
						<tr><th>Target</th><th>Crate</th><th>Status</th></tr>
					</thead>
					<tbody>
						{#each backends as b}
							<tr>
								<td>
									<div class="font-medium">{b.where}</div>
									<div class="text-xs text-base-content/60">{b.note}</div>
								</td>
								<td><a class="link font-mono text-xs" href="https://docs.rs/{b.crate}" rel="noopener">{b.crate}</a></td>
								<td>
									{#if b.status === 'ready'}
										<span class="badge badge-xs badge-soft badge-success">ready</span>
									{:else}
										<span class="badge badge-xs badge-soft badge-warning">building</span>
									{/if}
								</td>
							</tr>
						{/each}
					</tbody>
				</table>
			</div>
		</div>
	</div>
</section>

<!-- Crates -->
<section class="mx-auto max-w-7xl px-4 py-20">
	<div class="max-w-2xl">
		<span class="badge badge-soft badge-accent">On crates.io</span>
		<h2 class="mt-3 text-3xl font-bold sm:text-4xl">Nineteen crates, one version number</h2>
		<p class="mt-4 text-base-content/70">
			Take the core and a backend; leave the rest. Each crate has its own README — its API, its platform notes, and what it deliberately does not do — and every Rust example in them is compiled by <code class="font-mono text-[0.9em]">cargo test --doc</code>, so they cannot drift.
		</p>
	</div>
	<div class="mt-10 grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
		{#each crates as c}
			<a href="https://docs.rs/{c.name}" rel="noopener" class="group rounded-box border border-base-300 bg-base-200/50 p-4 transition hover:border-primary/50">
				<div class="flex items-center justify-between gap-2">
					<span class="font-mono text-sm font-medium group-hover:text-primary">{c.name}</span>
					<span class="badge badge-ghost badge-xs font-mono">{c.runs}</span>
				</div>
				<p class="mt-1.5 text-sm text-base-content/65">{c.what}</p>
			</a>
		{/each}
	</div>
	<div class="mt-8 flex flex-wrap justify-center gap-3">
		<CopyLine text={`cargo add denise@${minor} denise-ui@${minor} denise-winit@${minor}`} prompt="$" class="w-full max-w-2xl" />
	</div>
</section>

<!-- CTA -->
<section class="border-t border-base-300 bg-base-200/60">
	<div class="mx-auto max-w-3xl px-4 py-20 text-center">
		<h2 class="text-3xl font-bold sm:text-4xl">Draw your first frame</h2>
		<p class="mx-auto mt-4 max-w-xl text-base-content/70">
			Clone the repository and run the gallery in a window. When it looks right, rebuild it for the display itself and put it on a board.
		</p>
		<div class="mx-auto mt-8 max-w-xl space-y-2 text-left">
			<CopyLine prompt="$" text="git clone https://github.com/bisand/denise" />
			<CopyLine prompt="$" text="cargo run -p gallery" />
			<CopyLine prompt="$" text="cargo run -p gallery --no-default-features --features kiosk" />
		</div>
		<div class="mt-8 flex flex-wrap justify-center gap-3">
			<a href="{base}/docs/getting-started/" class="btn btn-primary btn-lg"><Icon name="book" class="h-5 w-5" /> Read the guide</a>
			<a href={site.github} class="btn btn-lg" rel="noopener"><Icon name="github" class="h-5 w-5" /> Star it on GitHub</a>
		</div>
	</div>
</section>
