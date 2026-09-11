// See https://svelte.dev/docs/kit/types#app.d.ts
declare global {
	/** The workspace version from `Cargo.toml`, injected by `vite.config.ts`. */
	const __DENISE_VERSION__: string;
	namespace App {}
}

export {};
