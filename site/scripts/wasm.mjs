// Builds the live demo — the gallery and the record editor, compiled to
// wasm32-wasip1 — and puts it where the page fetches it.
//
//   rustup target add wasm32-wasip1
//   node scripts/wasm.mjs
import { execFileSync } from 'node:child_process';
import { copyFileSync, mkdirSync, statSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const site = join(dirname(fileURLToPath(import.meta.url)), '..');
const manifest = join(site, 'wasm', 'Cargo.toml');

execFileSync(
	'cargo',
	['build', '--release', '--target', 'wasm32-wasip1', '--manifest-path', manifest],
	{ stdio: 'inherit' }
);

const built = join(site, 'wasm', 'target', 'wasm32-wasip1', 'release', 'denise_web.wasm');
const out = join(site, 'static', 'wasm', 'denise.wasm');
mkdirSync(dirname(out), { recursive: true });
copyFileSync(built, out);
console.log(`→ ${out} (${Math.round(statSync(out).size / 1024)} KB)`);
