import tailwindcss from '@tailwindcss/vite';
import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vite';
import { readFileSync } from 'node:fs';

// The version the site advertises is the workspace's, read at build time, so a
// release that bumps `Cargo.toml` on main updates the install snippet and the
// download links with it rather than leaving them for somebody to remember.
const cargo = readFileSync(new URL('../Cargo.toml', import.meta.url), 'utf8');
const version = /\[workspace\.package\][^[]*?version\s*=\s*"([^"]+)"/.exec(cargo)?.[1] ?? '0.0.0';

export default defineConfig({
	plugins: [tailwindcss(), sveltekit()],
	define: {
		__DENISE_VERSION__: JSON.stringify(version)
	},
	server: {
		fs: { allow: ['..'] }
	}
});
