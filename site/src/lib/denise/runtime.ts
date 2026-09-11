// A DeniseUI application, compiled to wasm32-wasip1, drawing into a canvas.
//
// The Rust side is `site/wasm`: the gallery and the record editor from
// `examples/`, unchanged, behind a dozen exported functions. This file is the
// other half of that backend — a four-function WASI (a clock, stderr, no
// files), input translation, and a blit that copies only the rectangles the
// tree reports as damaged.

export type DemoName = 'gallery' | 'editor';
export type DamageRect = { x: number; y: number; w: number; h: number };
export type FrameInfo = {
	rects: DamageRect[];
	/** Pixels painted by this frame. */
	pixels: number;
	/** Pixels on the whole surface. */
	surface: number;
	/** Canvas size in physical pixels, for mapping `rects` onto an overlay. */
	width: number;
	height: number;
};

interface Exports {
	memory: WebAssembly.Memory;
	denise_start(demo: number, w: number, h: number, scale: number, light: number, reduced: number): number;
	denise_frame(): number;
	denise_next_wake(): number;
	denise_rgba(): number;
	denise_rects(): number;
	denise_pointer_move(x: number, y: number): void;
	denise_pointer_button(button: number, down: number, x: number, y: number, mods: number): void;
	denise_scroll(dx: number, dy: number, x: number, y: number): void;
	denise_pointer_left(): void;
	denise_touch(phase: number, id: number, x: number, y: number): void;
	denise_key_buffer(): number;
	denise_key(len: number, down: number, repeat: number, mods: number): void;
	denise_text(ch: number): void;
	denise_theme(index: number): void;
}

const EBADF = 8;
const ENOSYS = 52;

let compiled: Promise<WebAssembly.Module> | null = null;

/** Fetches and compiles the module once per page, however many canvases use it. */
export function loadModule(url: string): Promise<WebAssembly.Module> {
	if (!compiled) {
		compiled = (async () => {
			const response = await fetch(url);
			if (!response.ok) throw new Error(`${url}: HTTP ${response.status}`);
			return WebAssembly.compile(await response.arrayBuffer());
		})();
		compiled.catch(() => (compiled = null));
	}
	return compiled;
}

/** Just enough `wasi_snapshot_preview1` for Rust's `std`: time, stderr, no filesystem. */
function wasiImports(module: WebAssembly.Module, memory: () => WebAssembly.Memory) {
	const view = () => new DataView(memory().buffer);
	const decoder = new TextDecoder();
	let line = '';

	const zeroSizes = (count: number, size: number) => {
		const v = view();
		v.setUint32(count, 0, true);
		v.setUint32(size, 0, true);
		return 0;
	};

	const impl: Record<string, (...args: any[]) => number> = {
		args_sizes_get: zeroSizes,
		args_get: () => 0,
		environ_sizes_get: zeroSizes,
		environ_get: () => 0,
		clock_time_get: (id: number, _precision: bigint, out: number) => {
			const ns =
				id === 0
					? BigInt(Date.now()) * 1_000_000n
					: BigInt(Math.round(performance.now() * 1000)) * 1000n;
			view().setBigUint64(out, ns, true);
			return 0;
		},
		fd_write: (fd: number, iovs: number, count: number, written: number) => {
			const v = view();
			let total = 0;
			for (let i = 0; i < count; i++) {
				const ptr = v.getUint32(iovs + i * 8, true);
				const len = v.getUint32(iovs + i * 8 + 4, true);
				if (fd === 1 || fd === 2) {
					line += decoder.decode(new Uint8Array(memory().buffer, ptr, len), { stream: true });
				}
				total += len;
			}
			let newline: number;
			while ((newline = line.indexOf('\n')) >= 0) {
				console.debug('[denise]', line.slice(0, newline));
				line = line.slice(newline + 1);
			}
			v.setUint32(written, total, true);
			return 0;
		},
		fd_fdstat_get: (fd: number, out: number) => {
			if (fd > 2) return EBADF;
			const v = view();
			for (let i = 0; i < 24; i++) v.setUint8(out + i, 0);
			v.setUint8(out, 2); // a character device
			return 0;
		},
		fd_close: () => 0,
		random_get: (ptr: number, len: number) => {
			for (let at = 0; at < len; at += 65536) {
				crypto.getRandomValues(new Uint8Array(memory().buffer, ptr + at, Math.min(65536, len - at)));
			}
			return 0;
		},
		sched_yield: () => 0,
		proc_exit: (code: number) => {
			throw new Error(`the application exited with ${code}`);
		}
	};

	const functions: Record<string, (...args: any[]) => number> = {};
	for (const entry of WebAssembly.Module.imports(module)) {
		if (entry.module !== 'wasi_snapshot_preview1' || entry.kind !== 'function') continue;
		// Anything else std asks for is a file, and there are none: no preopened
		// directories means `std::fs` fails cleanly rather than calling these.
		functions[entry.name] =
			impl[entry.name] ?? (() => (/^(fd|path)_/.test(entry.name) ? EBADF : ENOSYS));
	}
	return { wasi_snapshot_preview1: functions };
}

export type HostOptions = {
	demo: DemoName;
	light: boolean;
	/** The width the application's layout is written for, in logical pixels. */
	logicalWidth: number;
	onFrame?: (info: FrameInfo) => void;
	/** After each pass: milliseconds until the tree's next deadline, or -1 for none. */
	onSleep?: (wake: number) => void;
};

const modifiers = (e: KeyboardEvent | MouseEvent) =>
	(e.shiftKey ? 1 : 0) | (e.ctrlKey ? 2 : 0) | (e.altKey ? 4 : 0) | (e.metaKey ? 8 : 0);

/** Keys the page leaves to the browser: reload, developer tools, full screen. */
const BROWSER_KEYS = new Set(['F5', 'F11', 'F12']);

export class DeniseHost {
	private ex: Exports;
	private ctx: CanvasRenderingContext2D;
	private image: ImageData | null = null;
	private width = 0;
	private height = 0;
	private raf = 0;
	private timer: ReturnType<typeof setTimeout> | undefined;
	private resizeTimer: ReturnType<typeof setTimeout> | undefined;
	private visible = true;
	private destroyed = false;
	private cleanup: (() => void)[] = [];
	private encoder = new TextEncoder();

	private constructor(
		private canvas: HTMLCanvasElement,
		instance: WebAssembly.Instance,
		private options: HostOptions
	) {
		this.ex = instance.exports as unknown as Exports;
		const ctx = canvas.getContext('2d', { alpha: false });
		if (!ctx) throw new Error('no 2D canvas context');
		this.ctx = ctx;
	}

	static async create(canvas: HTMLCanvasElement, module: WebAssembly.Module, options: HostOptions) {
		let memory: WebAssembly.Memory | null = null;
		const instance = await WebAssembly.instantiate(
			module,
			wasiImports(module, () => memory!)
		);
		memory = (instance.exports as unknown as Exports).memory;
		const host = new DeniseHost(canvas, instance, options);
		host.attach();
		host.start();
		return host;
	}

	/** Rebuilds the application, in a different demo or theme. Its state starts over. */
	restart(changes: Partial<Pick<HostOptions, 'demo' | 'light' | 'logicalWidth'>>) {
		Object.assign(this.options, changes);
		this.start();
	}

	/** 0 light, 1 dark, 2 high contrast. */
	setTheme(index: number) {
		this.ex.denise_theme(index);
		this.request();
	}

	destroy() {
		this.destroyed = true;
		cancelAnimationFrame(this.raf);
		clearTimeout(this.timer);
		clearTimeout(this.resizeTimer);
		for (const undo of this.cleanup) undo();
		this.cleanup = [];
	}

	private start() {
		const rect = this.canvas.getBoundingClientRect();
		const dpr = Math.min(window.devicePixelRatio || 1, 3);
		const width = Math.max(1, Math.round(rect.width * dpr));
		const height = Math.max(1, Math.round(rect.height * dpr));
		this.width = width;
		this.height = height;
		this.canvas.width = width;
		this.canvas.height = height;
		this.image = null;
		const scale = (rect.width / this.options.logicalWidth) * dpr;
		const reduced = matchMedia('(prefers-reduced-motion: reduce)').matches;
		this.ex.denise_start(
			this.options.demo === 'editor' ? 1 : 0,
			width,
			height,
			scale,
			this.options.light ? 1 : 0,
			reduced ? 1 : 0
		);
		this.request();
	}

	private request = () => {
		if (!this.raf && !this.destroyed) this.raf = requestAnimationFrame(this.frame);
	};

	private frame = () => {
		this.raf = 0;
		if (this.destroyed) return;
		const count = this.ex.denise_frame();
		if (count > 0) this.blit(count);
		this.plan();
	};

	/** Sleeps until the tree's next deadline, or indefinitely when it has none. */
	private plan() {
		clearTimeout(this.timer);
		if (!this.visible) return;
		const wake = this.ex.denise_next_wake();
		this.options.onSleep?.(wake);
		if (wake < 0) return;
		if (wake <= 4) this.request();
		else this.timer = setTimeout(this.request, wake);
	}

	private blit(count: number) {
		const buffer = this.ex.memory.buffer;
		const ptr = this.ex.denise_rgba();
		const bytes = this.width * this.height * 4;
		if (!this.image || this.image.data.buffer !== buffer || this.image.data.byteOffset !== ptr) {
			this.image = new ImageData(new Uint8ClampedArray(buffer, ptr, bytes), this.width, this.height);
		}
		const words = new Int32Array(buffer, this.ex.denise_rects(), count * 4);
		const rects: DamageRect[] = [];
		let pixels = 0;
		for (let i = 0; i < count; i++) {
			const [x, y, w, h] = [words[i * 4], words[i * 4 + 1], words[i * 4 + 2], words[i * 4 + 3]];
			// Only the damaged rectangle is copied: the dirty-rect form of
			// putImageData, which is the browser's version of a partial present.
			this.ctx.putImageData(this.image, 0, 0, x, y, w, h);
			rects.push({ x, y, w, h });
			pixels += w * h;
		}
		this.options.onFrame?.({
			rects,
			pixels,
			surface: this.width * this.height,
			width: this.width,
			height: this.height
		});
	}

	private position(e: MouseEvent): [number, number] {
		const r = this.canvas.getBoundingClientRect();
		return [
			Math.floor(((e.clientX - r.left) * this.width) / r.width),
			Math.floor(((e.clientY - r.top) * this.height) / r.height)
		];
	}

	private on<K extends keyof HTMLElementEventMap>(
		target: HTMLElement,
		type: K,
		handler: (e: HTMLElementEventMap[K]) => void,
		options?: AddEventListenerOptions
	) {
		target.addEventListener(type, handler as EventListener, options);
		this.cleanup.push(() => target.removeEventListener(type, handler as EventListener, options));
	}

	private attach() {
		const c = this.canvas;
		const touches = new Set<number>();

		this.on(c, 'pointerdown', (e) => {
			e.preventDefault();
			c.focus({ preventScroll: true });
			// A drag that leaves the canvas still belongs to the widget that
			// started it. Synthetic events have no pointer to capture, and
			// losing the capture must not lose the press.
			try {
				c.setPointerCapture(e.pointerId);
			} catch {}
			const [x, y] = this.position(e);
			if (e.pointerType === 'touch') {
				touches.add(e.pointerId);
				this.ex.denise_touch(0, e.pointerId, x, y);
			} else {
				this.ex.denise_pointer_move(x, y);
				this.ex.denise_pointer_button(e.button, 1, x, y, modifiers(e));
			}
			this.request();
		});
		this.on(c, 'pointermove', (e) => {
			const [x, y] = this.position(e);
			if (e.pointerType === 'touch') {
				if (!touches.has(e.pointerId)) return;
				this.ex.denise_touch(1, e.pointerId, x, y);
			} else {
				this.ex.denise_pointer_move(x, y);
			}
			this.request();
		});
		const release = (e: PointerEvent) => {
			const [x, y] = this.position(e);
			if (e.pointerType === 'touch') {
				if (!touches.delete(e.pointerId)) return;
				this.ex.denise_touch(e.type === 'pointercancel' ? 3 : 2, e.pointerId, x, y);
			} else {
				this.ex.denise_pointer_button(e.button, 0, x, y, modifiers(e));
			}
			this.request();
		};
		this.on(c, 'pointerup', release);
		this.on(c, 'pointercancel', release);
		this.on(c, 'pointerleave', (e) => {
			if (e.pointerType === 'touch') return;
			this.ex.denise_pointer_left();
			this.request();
		});
		this.on(c, 'contextmenu', (e) => e.preventDefault());

		// The wheel belongs to the page until the canvas has been clicked, so a
		// visitor scrolling past the demo is not caught by it.
		this.on(
			c,
			'wheel',
			(e) => {
				if (document.activeElement !== c) return;
				e.preventDefault();
				const r = c.getBoundingClientRect();
				const unit = e.deltaMode === 1 ? 16 : e.deltaMode === 2 ? r.height : 1;
				const k = (unit * this.width) / r.width;
				const [x, y] = this.position(e);
				this.ex.denise_scroll(e.deltaX * k, e.deltaY * k, x, y);
				this.request();
			},
			{ passive: false }
		);

		const key = (e: KeyboardEvent, down: boolean) => {
			if (BROWSER_KEYS.has(e.code)) return;
			const shortcut = e.metaKey || (e.ctrlKey && !e.altKey);
			const bytes = this.encoder.encode(e.code).subarray(0, 32);
			new Uint8Array(this.ex.memory.buffer, this.ex.denise_key_buffer(), 32).set(bytes);
			this.ex.denise_key(bytes.length, down ? 1 : 0, e.repeat ? 1 : 0, modifiers(e));
			if (down && !shortcut && [...e.key].length === 1) {
				this.ex.denise_text(e.key.codePointAt(0)!);
			}
			if (!shortcut) e.preventDefault();
			this.request();
		};
		this.on(c, 'keydown', (e) => key(e, true));
		this.on(c, 'keyup', (e) => key(e, false));

		const visibility = new IntersectionObserver((entries) => {
			this.visible = entries.some((entry) => entry.isIntersecting);
			if (this.visible) this.request();
			else clearTimeout(this.timer);
		});
		visibility.observe(c);
		this.cleanup.push(() => visibility.disconnect());

		const size = new ResizeObserver(() => {
			clearTimeout(this.resizeTimer);
			this.resizeTimer = setTimeout(() => {
				const r = c.getBoundingClientRect();
				const dpr = Math.min(window.devicePixelRatio || 1, 3);
				if (Math.abs(Math.round(r.width * dpr) - this.width) > 2) this.start();
			}, 200);
		});
		size.observe(c);
		this.cleanup.push(() => size.disconnect());
	}
}
