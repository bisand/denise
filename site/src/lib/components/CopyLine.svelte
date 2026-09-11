<script lang="ts">
	/** A one-line command or snippet with a copy button. */
	let { text, prompt = '', class: cls = '' }: { text: string; prompt?: string; class?: string } = $props();
	let copied = $state(false);

	async function copy() {
		try {
			await navigator.clipboard.writeText(text);
			copied = true;
			setTimeout(() => (copied = false), 1600);
		} catch {}
	}
</script>

<div class="flex items-center gap-3 rounded-box border border-base-300 bg-base-200/80 py-2 pr-2 pl-4 font-mono text-sm {cls}">
	{#if prompt}<span class="text-base-content/40 select-none">{prompt}</span>{/if}
	<code class="flex-1 overflow-x-auto whitespace-nowrap">{text}</code>
	<button class="btn btn-ghost btn-xs" onclick={copy} aria-label="Copy to clipboard">{copied ? 'Copied' : 'Copy'}</button>
</div>
