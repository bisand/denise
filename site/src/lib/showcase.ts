// Programs built with DeniseUI: the applications in the repository, and the
// ones outside it. Every command here is one that works from a clone.

export type Program = {
	id: string;
	name: string;
	what: string;
	body: string;
	shot?: string;
	alt?: string;
	run?: string[];
	source: string;
	external?: string;
	tags: string[];
};

export const programs: Program[] = [
	{
		id: 'designer',
		name: 'The form designer',
		what: 'A visual designer for .dform files — and a Denise application itself',
		body:
			'Palette, canvas, inspector and outline, with snapping guides, alignment, grouping, undo that is exact to the byte, and F5 to run the form. It is not Tauri and not a web page: the canvas draws the form with the same code that will draw it on the panel, so what is on screen is what ships. It is also the first real application written on the toolkit, which is how the toolkit finds out what it lacks.',
		shot: 'designer-2x.webp',
		alt: 'The designer with the reference form on its canvas, a slider selected with eight handles and an alignment guide, and the slider’s properties on the right',
		run: ['cargo run -p denise-designer -- forms/reference.dform'],
		source: 'tools/designer',
		tags: ['application', 'desktop', 'ships as a download']
	},
	{
		id: 'squint',
		name: 'squint',
		what: 'A text editor for files too big for text editors',
		body:
			'Opens a file of any size instantly and holds almost none of it in memory: a piece table over the file on disk, a sparse line index built in the background, syntax highlighting through syntect. Its whole window is DeniseUI — the TextArea edits the engine’s document through the toolkit’s TextDocument trait, so the widget never learns how big the file is, and the tab strip that closes, drags and renames is the toolkit’s Tabs. It is where several of the toolkit’s newer features were found.',
		shot: 'squint.webp',
		alt: 'squint showing a Rust source file with syntax highlighting, a tab at the top, line numbers, and a find field at the bottom',
		source: 'https://github.com/bisand/squint',
		external: 'https://github.com/bisand/squint',
		tags: ['application', 'desktop', 'separate project']
	},
	{
		id: 'gallery',
		name: 'gallery',
		what: 'Every widget live, with a theme editor beside them',
		body:
			'Nine seed colours, a light and dark switch, radius and depth, and a Surprise button. Move a slider and the whole surface follows, because widgets name theme roles and never colours. The badge in the corner is the worst surface-to-content contrast in whatever you have built. Its overlays section is where a modal, a drawer and a shelf are compared side by side.',
		shot: 'gallery-2x.webp',
		alt: 'The gallery: a theme editor sidebar beside live widgets — role buttons, form controls, sliders driving a progress ring, ratings and a spinner',
		run: ['cargo run -p gallery', 'cargo run -p gallery -- --keyboard', 'cargo run -p gallery --no-default-features --features kiosk'],
		source: 'examples/gallery',
		tags: ['example', 'window or panel']
	},
	{
		id: 'table-editor',
		name: 'table-editor',
		what: 'A record editor: a grid, a form, validation, a modal and CSV',
		body:
			'About as much user interface as a small internal tool has. Nine row nodes exist however many records there are; the rules live in table.rs, which knows no widget exists and is unit tested without a display. The same app.rs drives a window and a bare panel — only main.rs differs, by about fifty lines.',
		shot: 'table-editor-win.webp',
		alt: 'The record editor on Windows 11: a five-row grid with a selected row, an edit form and a status line',
		run: ['cargo run -p table-editor', 'cargo run -p table-editor --no-default-features --features kiosk'],
		source: 'examples/table-editor',
		tags: ['example', 'window or panel']
	},
	{
		id: 'browser',
		name: 'browser',
		what: 'A small web browser in which every visible thing is a widget',
		body:
			'Real pages — Hacker News, Wikipedia, DuckDuckGo Lite — fetched over rustls, parsed with html5ever, laid out by the example’s own block-and-inline engine and drawn entirely through the toolkit. A page’s form controls are the actual TextInput, Checkbox, RadioGroup and Select, and they submit for real. No JavaScript, on purpose. It is the composability proof.',
		shot: 'browser-form.webp',
		alt: 'The browser rendering an HTML form with headings, styled text and real Denise form controls',
		run: ['cargo run -p browser -- https://news.ycombinator.com'],
		source: 'examples/browser',
		tags: ['example', 'window or panel']
	},
	{
		id: 'hello',
		name: 'hello',
		what: 'Eighty lines: a message enum, a tree, an event loop',
		body: 'Start here. Half of it is comments, and it builds for a window or for a bare display with one feature flag. The same screen exists as a form file in examples/designed, so the two can be read side by side.',
		shot: 'hello-mac.webp',
		alt: 'The hello example running in a macOS window: a heading, a prompt, a text field and a Greet button',
		run: ['cargo run -p hello', 'cargo run -p hello -- --snapshot hello.ppm'],
		source: 'examples/hello',
		tags: ['example', 'start here']
	}
];

/** The rest of the examples, as a list rather than as cards. */
export const more = [
	{ name: 'designed', what: 'hello again, built from hello.dform instead of from Rust. Read the two side by side.', path: 'examples/designed' },
	{ name: 'runtime', what: 'Two forms read from files at run time: swap a screen by copying a file over it.', path: 'examples/runtime' },
	{ name: 'forms', what: 'Secondary windows on the desktop: a modeless settings form, a modal, and the state they share.', path: 'examples/forms' },
	{ name: 'hello-rect', what: 'The damage proof: a bouncing rectangle that repaints two rectangles, not a window.', path: 'examples/hello-rect' },
	{ name: 'panel', what: 'The widget tree, a modal and a cursor sprite, on bare Linux with no X.', path: 'examples/panel' },
	{ name: 'kiosk', what: 'The instrumented loop: input latency and frame-time percentiles.', path: 'examples/kiosk' },
	{ name: 'launcher', what: 'A menu of the other demos, handing the display to another program and taking it back.', path: 'examples/launcher' },
	{ name: 'splash', what: 'A boot splash on fbdev that survives the framebuffer being replaced underneath it.', path: 'examples/splash' },
	{ name: 'arranged', what: 'The sizing experiment behind denise-arrange.', path: 'examples/arranged' },
	{ name: 'typed', what: 'Text input and layout, on its own.', path: 'examples/typed' }
];
