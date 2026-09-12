import tailwindcss from '@tailwindcss/vite';
import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig, type Plugin } from 'vite';
import { readFileSync } from 'node:fs';

// What version the site advertises.
//
// **The latest published release**, asked for at build time, because that is
// what a visitor can actually `cargo add` — and because it is one number in one
// place rather than a dependency snippet, a download link and a badge that each
// go stale on their own. The Website workflow rebuilds on `release: published`,
// so a release updates the site without anybody touching the site.
//
// The workspace manifest is the fallback, for a clone with no network and for
// the case the API is having a day. It is the right answer a moment before a
// release and a moment after one, and never wrong by more than that.
const cargo = readFileSync(new URL('../Cargo.toml', import.meta.url), 'utf8');
const inManifest = /\[workspace\.package\][^[]*?version\s*=\s*"([^"]+)"/.exec(cargo)?.[1] ?? '0.0.0';

async function published(): Promise<string> {
	const token = process.env.GITHUB_TOKEN;
	try {
		const response = await fetch('https://api.github.com/repos/bisand/denise/releases/latest', {
			headers: {
				accept: 'application/vnd.github+json',
				// Authenticated in CI: sixty requests an hour is shared by every
				// project on a runner's address, and a rate-limited build would
				// quietly publish the manifest's number instead.
				...(token ? { authorization: `Bearer ${token}` } : {})
			},
			signal: AbortSignal.timeout(10_000)
		});
		if (!response.ok) throw new Error(`HTTP ${response.status}`);
		const tag = String((await response.json()).tag_name ?? '');
		const match = /^v?(\d+\.\d+\.\d+.*)$/.exec(tag);
		if (!match) throw new Error(`tag ${tag} is not a version`);
		return match[1];
	} catch (error) {
		console.warn(
			`[denise] using the workspace version ${inManifest}: could not read the latest release (${error instanceof Error ? error.message : error})`
		);
		return inManifest;
	}
}

/** Puts the version into the guide's Markdown, where a `{}` cannot go. */
function versionTokens(version: string, minor: string): Plugin {
	return {
		name: 'denise-version-tokens',
		enforce: 'pre',
		transform(code, id) {
			if (!id.endsWith('.md')) return null;
			if (!code.includes('__DENISE_')) return null;
			return {
				code: code.replaceAll('__DENISE_MINOR__', minor).replaceAll('__DENISE_VERSION__', version),
				map: null
			};
		}
	};
}

export default defineConfig(async () => {
	const version = await published();
	const minor = version.split('.').slice(0, 2).join('.');
	return {
		plugins: [versionTokens(version, minor), tailwindcss(), sveltekit()],
		define: {
			__DENISE_VERSION__: JSON.stringify(version)
		},
		server: {
			fs: { allow: ['..'] }
		}
	};
});
