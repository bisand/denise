import { docPages, nav } from '$lib/site';

export const prerender = true;

const ORIGIN = 'https://bisand.github.io/denise';

export function GET() {
	const paths = ['/', ...nav.map((n) => n.href), ...docPages.map((d) => `/docs/${d.slug}/`)];
	const body = `<?xml version="1.0" encoding="UTF-8"?>
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
${paths.map((path) => `\t<url><loc>${ORIGIN}${path}</loc></url>`).join('\n')}
</urlset>
`;
	return new Response(body, { headers: { 'content-type': 'application/xml' } });
}
