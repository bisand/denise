import adapter from '@sveltejs/adapter-static';
import { vitePreprocess } from '@sveltejs/vite-plugin-svelte';
import { mdsvex, escapeSvelte } from 'mdsvex';
import { createHighlighter } from 'shiki';

// One highlighter for every Markdown page, built once at startup. The theme
// pair matches the site's: Catppuccin, like the logo and the toolkit's own
// default seeds.
const highlighter = await createHighlighter({
	themes: ['catppuccin-mocha', 'catppuccin-latte'],
	langs: ['rust', 'toml', 'bash', 'c', 'kdl', 'text']
});

/** @type {import('@sveltejs/kit').Config} */
const config = {
	extensions: ['.svelte', '.md'],
	preprocess: [
		vitePreprocess(),
		mdsvex({
			extensions: ['.md'],
			highlight: {
				highlighter: async (code, lang = 'text') => {
					const known = highlighter.getLoadedLanguages().includes(lang) ? lang : 'text';
					const html = highlighter.codeToHtml(code, {
						lang: known,
						themes: { dark: 'catppuccin-mocha', light: 'catppuccin-latte' },
						defaultColor: false
					});
					return `{@html \`${escapeSvelte(html)}\`}`;
				}
			}
		})
	],
	kit: {
		adapter: adapter({ pages: 'build', assets: 'build', strict: true }),
		paths: { base: process.env.BASE_PATH ?? '' },
		prerender: { entries: ['*'] }
	}
};

export default config;
