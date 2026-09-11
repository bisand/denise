import { createHighlighter, type Highlighter } from 'shiki';

let highlighter: Promise<Highlighter> | null = null;

/** Highlights `code` at build time, emitting both Catppuccin themes as CSS variables. */
export async function highlight(code: string, lang: string): Promise<string> {
	highlighter ??= createHighlighter({
		themes: ['catppuccin-mocha', 'catppuccin-latte'],
		langs: ['rust', 'toml', 'bash', 'c', 'kdl', 'text']
	});
	return (await highlighter).codeToHtml(code.trim(), {
		lang,
		themes: { dark: 'catppuccin-mocha', light: 'catppuccin-latte' },
		defaultColor: false
	});
}
