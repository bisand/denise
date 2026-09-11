import { error } from '@sveltejs/kit';
import { docPages } from '$lib/site';
import type { Component } from 'svelte';
import type { EntryGenerator } from './$types';

export const prerender = true;

export const entries: EntryGenerator = () => docPages.map((doc) => ({ slug: doc.slug }));

export async function load({ params }) {
	const pages = import.meta.glob('$lib/docs/*.md');
	const match = pages[`/src/lib/docs/${params.slug}.md`];
	if (!match) error(404, `No guide called ${params.slug}`);
	const page = (await match()) as { default: Component; metadata: { title: string; description: string } };
	return { content: page.default, meta: page.metadata, slug: params.slug };
}
