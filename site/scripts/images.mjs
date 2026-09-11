// Converts static/screenshots/*.{png,jpg} to WebP, capped at 2560 px wide, and
// removes the originals. Renders the logo to PNG for touch icons and builds the
// social card. Run: node scripts/images.mjs
import sharp from 'sharp';
import { readdir, unlink, readFile } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { join, extname } from 'node:path';

const dir = 'static/screenshots';
for (const name of await readdir(dir)) {
	const path = join(dir, name);
	if (!['.png', '.jpg', '.jpeg'].includes(extname(name))) continue;
	const out = path.replace(/\.(png|jpe?g)$/, '.webp');
	await sharp(path)
		.resize({ width: 2560, withoutEnlargement: true })
		.webp({ quality: 86, effort: 6 })
		.toFile(out);
	await unlink(path);
	console.log('→', out);
}

const logo = await readFile('static/logo.svg');
await sharp(logo, { density: 384 }).resize(256, 256).png().toFile('static/logo-256.png');
console.log('→ static/logo-256.png');

if (existsSync('scripts/og.svg') && existsSync(join(dir, 'gallery-2x.webp'))) {
	const shot = await sharp(join(dir, 'gallery-2x.webp')).resize(760).png().toBuffer();
	await sharp(await readFile('scripts/og.svg'))
		.composite([{ input: shot, left: 520, top: 150 }])
		.png()
		.toFile('static/og.png');
	console.log('→ static/og.png');
}
