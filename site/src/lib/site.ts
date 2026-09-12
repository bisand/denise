export const version = __DENISE_VERSION__;
/** `0.23` from `0.23.0`: what a Cargo dependency line says. */
export const minor = version.split('.').slice(0, 2).join('.');

const github = 'https://github.com/bisand/denise';

export const site = {
	name: 'DeniseUI',
	tagline: 'Pixels straight to the panel.',
	description:
		'DeniseUI is a direct-rendering UI toolkit in Rust for embedded Linux and machines with no desktop: straight to DRM/KMS or the framebuffer, damage-tracked repaints, a no_std core, 28 widgets and a visual form designer.',
	github,
	issues: `${github}/issues`,
	releases: `${github}/releases`,
	latestRelease: `${github}/releases/latest`,
	license: `${github}/blob/main/LICENSE`,
	crates: 'https://crates.io/crates/denise',
	docsrs: 'https://docs.rs/denise',
	author: 'André Biseth',
	authorUrl: 'https://github.com/bisand',
	msrv: '1.95'
};

export const blob = (path: string) => `${github}/blob/main/${path}`;
export const tree = (path: string) => `${github}/tree/main/${path}`;

export const nav = [
	{ href: '/demo/', label: 'Live demo' },
	{ href: '/widgets/', label: 'Widgets' },
	{ href: '/showcase/', label: 'Showcase' },
	{ href: '/designer/', label: 'Designer' },
	{ href: '/docs/', label: 'Docs' }
];

export type Download = { os: string; detail: string; file: string; icon: string };

export const downloads: Download[] = [
	{ os: 'macOS', detail: 'Universal: Intel and Apple silicon, .dmg', file: `denise-${version}-universal-apple-darwin.dmg`, icon: 'apple' },
	{ os: 'Windows', detail: 'x86-64, .zip', file: `denise-${version}-x86_64-pc-windows-msvc.zip`, icon: 'windows' },
	{ os: 'Linux', detail: 'x86-64, .tar.gz', file: `denise-${version}-x86_64-unknown-linux-gnu.tar.gz`, icon: 'linux' },
	{ os: 'Linux ARM', detail: 'aarch64, .tar.gz', file: `denise-${version}-aarch64-unknown-linux-gnu.tar.gz`, icon: 'linux' }
];

export const downloadUrl = (file: string) => `${github}/releases/download/v${version}/${file}`;

export type DocPage = { slug: string; title: string; group: string };

export const docPages: DocPage[] = [
	{ slug: 'getting-started', title: 'Getting started', group: 'Guide' },
	{ slug: 'how-it-works', title: 'How it works', group: 'Guide' },
	{ slug: 'theming', title: 'Theming', group: 'Guide' },
	{ slug: 'forms-and-designer', title: 'Forms and the designer', group: 'Guide' },
	{ slug: 'raspberry-pi', title: 'On a Raspberry Pi', group: 'Platforms' },
	{ slug: 'embedding', title: 'Embedding in other apps', group: 'Platforms' }
];

export type Crate = { name: string; what: string; runs: string };

export const crates: Crate[] = [
	{ name: 'denise', what: 'Core types, the surface and painting traits, damage tracking, theming', runs: 'no_std + alloc' },
	{ name: 'denise-render', what: 'Software rasteriser and the built-in font', runs: 'no_std + alloc' },
	{ name: 'denise-text', what: 'Glyph sources, atlas, line layout, word wrapping', runs: 'no_std + alloc' },
	{ name: 'denise-ui', what: 'Scene graph, scene stack, widgets, cursor sprite', runs: 'no_std + alloc' },
	{ name: 'denise-arrange', what: 'Optional content-driven layout: rows, columns and layers over the tree', runs: 'no_std + alloc' },
	{ name: 'denise-wgpu', what: 'The same painting trait on wgpu, for the desktop', runs: 'std' },
	{ name: 'denise-image', what: 'PNG, JPEG, GIF and BMP decoding into premultiplied pixels', runs: 'std' },
	{ name: 'denise-layout', what: 'Keyboard layouts, dead keys, the system’s configured layout', runs: 'std' },
	{ name: 'denise-forms', what: 'Loads a .dform file into a widget tree at run time', runs: 'std' },
	{ name: 'denise-keyboard', what: 'On-screen keyboard: a shelf of keys that emits what hardware emits', runs: 'std' },
	{ name: 'denise-video', what: 'V4L2 hardware decode onto a DRM plane, zero-copy', runs: 'Linux' },
	{ name: 'denise-drm', what: 'Linux DRM/KMS, the primary target', runs: 'Linux' },
	{ name: 'denise-fbdev', what: 'Linux fbdev fallback', runs: 'Linux' },
	{ name: 'denise-evdev', what: 'Input devices, console muting', runs: 'Linux' },
	{ name: 'denise-winit', what: 'Desktop development and preview', runs: 'any' },
	{ name: 'denise-macos', what: 'Embeddable NSView', runs: 'macOS' },
	{ name: 'denise-win32', what: 'Child-HWND control', runs: 'Windows' },
	{ name: 'denise-activex', what: 'COM/ActiveX shim, scriptable', runs: 'Windows' },
	{ name: 'denise-ffi', what: 'Stable C ABI, cdylib', runs: 'any' }
];

export type Backend = { where: string; crate: string; status: 'ready' | 'building'; note: string };

export const backends: Backend[] = [
	{ where: 'Bare Linux, DRM/KMS', crate: 'denise-drm', status: 'ready', note: 'Pi 3 A+ at 1920×1080, async page flips, hardware cursor plane, console restored on exit' },
	{ where: 'Bare Linux, fbdev', crate: 'denise-fbdev', status: 'ready', note: 'The fallback when there is no /dev/dri' },
	{ where: 'macOS, Windows, Linux desktops', crate: 'denise-winit', status: 'ready', note: 'Development and preview, one tree per window' },
	{ where: 'Desktop, on the GPU', crate: 'denise-wgpu', status: 'building', note: 'A painter on wgpu, parity-tested against the rasteriser' },
	{ where: 'Inside a macOS app', crate: 'denise-macos', status: 'ready', note: 'An NSView over a CoreGraphics bitmap context' },
	{ where: 'Inside a Windows app', crate: 'denise-win32', status: 'ready', note: 'A child HWND over a DIB section' },
	{ where: 'COM and ActiveX hosts', crate: 'denise-activex', status: 'ready', note: 'Registered, sited, scriptable, with a type library' },
	{ where: 'Anything with a C FFI', crate: 'denise-ffi', status: 'ready', note: 'A stable C ABI and a hand-written header' }
];
