<!--
  Center view of the Develop module: the live preview of the active photo.

  The view follows `develop.target` (the focused photo of the library while
  the module is active). When the module is hidden or the view is
  destroyed, the store saves the edits and frees the session.
-->
<script lang="ts">
  import { onDestroy, untrack } from 'svelte';
  import { HistogramFeed } from '../../lib/develop/histogram';
  import type { DevelopStore } from '../../lib/stores/develop.svelte';
  import PreviewCanvas from '../develop/PreviewCanvas.svelte';

  interface Props {
    develop: DevelopStore;
  }

  let { develop }: Props = $props();

  $effect(() => {
    const target = develop.target;
    untrack(() => void develop.activate(target));
  });

  // The histogram follows the displayed frames, at most every 100 ms.
  const feed = new HistogramFeed({ onHistogram: (histogram) => (develop.histogram = histogram) });

  $effect(() => {
    // A new or closed session: drop the frame that waits for a histogram.
    void develop.session;
    feed.cancel();
  });

  onDestroy(() => {
    feed.cancel();
    void develop.dispose();
  });
</script>

<div class="develop">
  {#if develop.status === 'empty'}
    <p class="message" role="status">Select a photo in the Library to develop it.</p>
  {:else if develop.status === 'loading'}
    <p class="message" role="status">Loading the photo…</p>
  {:else if develop.status === 'error'}
    <p class="message error" role="alert" data-testid="develop-error">
      The develop settings of this photo could not be read: {develop.loadError}
    </p>
  {:else if develop.settings !== null}
    <PreviewCanvas
      session={develop.session}
      settings={develop.settings}
      onframe={(event) => feed.push(event.frame)}
    />
  {/if}
</div>

<style>
  .develop {
    width: 100%;
    height: 100%;
    background: var(--color-bg-center);
  }
  .message {
    display: flex;
    align-items: center;
    justify-content: center;
    height: 100%;
    margin: 0;
    padding: var(--space-5);
    color: var(--color-text-muted);
    font-size: var(--font-size-md);
  }
  .error {
    color: var(--color-danger);
  }
</style>
