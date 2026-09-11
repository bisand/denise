import { highlight } from '$lib/server/highlight';
import { snippets, install } from '$lib/snippets';
import { minor } from '$lib/site';

export const prerender = true;

export async function load() {
	return {
		tabs: await Promise.all(
			snippets.map(async (s) => ({
				id: s.id,
				label: s.label,
				note: s.note,
				html: await highlight(s.code, s.lang)
			}))
		),
		install: await highlight(install.replaceAll('VERSION', minor), 'toml')
	};
}
