<script lang="ts">
	import { base } from '$app/paths';

	/** A screenshot that opens full size in a dialog when clicked. */
	let {
		src,
		alt,
		caption = '',
		class: cls = '',
		imgClass = '',
		eager = false
	}: { src: string; alt: string; caption?: string; class?: string; imgClass?: string; eager?: boolean } = $props();

	let dialog: HTMLDialogElement;
	const url = $derived(`${base}/screenshots/${src}`);
</script>

<figure class={cls}>
	<button type="button" class="group block w-full cursor-zoom-in overflow-hidden rounded-box border border-base-300 bg-base-300 shadow-xl" onclick={() => dialog.showModal()} aria-label="Enlarge: {alt}">
		<img src={url} {alt} loading={eager ? 'eager' : 'lazy'} decoding="async" class="block w-full transition duration-500 group-hover:scale-[1.015] {imgClass}" />
	</button>
	{#if caption}
		<figcaption class="mt-2.5 text-center text-sm text-base-content/60">{caption}</figcaption>
	{/if}
</figure>

<dialog bind:this={dialog} class="modal" onclick={(e) => e.target === dialog && dialog.close()}>
	<div class="modal-box max-h-[95vh] w-auto max-w-[95vw] bg-base-200 p-2">
		<img src={url} {alt} class="max-h-[88vh] w-auto rounded-lg" />
		{#if caption}<p class="px-2 pt-2 pb-1 text-sm text-base-content/70">{caption}</p>{/if}
	</div>
	<form method="dialog" class="modal-backdrop"><button>close</button></form>
</dialog>
