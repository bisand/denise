<script lang="ts">
	import { base } from '$app/paths';
	import { site, nav, docPages, blob, version } from '$lib/site';
</script>

<footer class="mt-24 border-t border-base-300 bg-base-200">
	<div class="footer mx-auto max-w-7xl px-4 py-14 sm:footer-horizontal">
		<aside class="max-w-xs">
			<div class="flex items-center gap-2.5 font-display text-lg font-semibold">
				<img src="{base}/logo.svg" alt="" width="34" height="34" />
				{site.name}
			</div>
			<p class="text-base-content/70">
				A direct-rendering UI toolkit in Rust, for embedded Linux and machines with no desktop environment.
			</p>
			<p class="text-base-content/60">MIT licensed. Made in Norway by <a href={site.authorUrl} class="link" rel="noopener">{site.author}</a>.</p>
		</aside>
		<nav>
			<h6 class="footer-title">Site</h6>
			{#each nav as item}
				<a href="{base}{item.href}" class="link link-hover">{item.label}</a>
			{/each}
		</nav>
		<nav>
			<h6 class="footer-title">Guide</h6>
			{#each docPages as doc}
				<a href="{base}/docs/{doc.slug}/" class="link link-hover">{doc.title}</a>
			{/each}
		</nav>
		<nav>
			<h6 class="footer-title">Project</h6>
			<a href={site.github} class="link link-hover" rel="noopener">Source code</a>
			<a href={site.crates} class="link link-hover" rel="noopener">crates.io</a>
			<a href={site.docsrs} class="link link-hover" rel="noopener">API docs on docs.rs</a>
			<a href={site.releases} class="link link-hover" rel="noopener">Releases</a>
			<a href={site.issues} class="link link-hover" rel="noopener">Issues</a>
			<a href={blob('docs/design.md')} class="link link-hover" rel="noopener">Design notes</a>
		</nav>
	</div>
	<div class="mx-auto flex max-w-7xl flex-wrap justify-between gap-2 px-4 pb-8 text-xs text-base-content/50">
		<span>© 2026 {site.author}. DeniseUI v{version}.</span>
		<span>The live demos on this site are the toolkit itself, compiled to WebAssembly.</span>
	</div>
</footer>
