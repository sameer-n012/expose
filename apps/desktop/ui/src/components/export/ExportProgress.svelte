<!--
  Progress panel of the export job: the count, a Cancel button, and at the
  end a summary. The messages of failed photos open on request.
-->
<script lang="ts">
  import type { ExportStore } from '../../lib/stores/export.svelte';

  interface Props {
    store: ExportStore;
  }

  let { store }: Props = $props();

  const finished = $derived(store.phase === 'finished');
  const percent = $derived(store.total === 0 ? 0 : Math.min(100, Math.round((store.done / store.total) * 100)));
  const failedCount = $derived(store.failures.length);
</script>

{#if store.phase !== 'idle'}
  <div class="panel" role="status" aria-label="Export progress" data-testid="export-progress">
    {#if !finished}
      <div class="line">
        <strong>Exporting</strong>
        <span data-testid="export-count-text">{store.done} / {store.total}</span>
        <button type="button" disabled={store.cancelling} onclick={() => void store.cancel()}>
          {store.cancelling ? 'Cancelling…' : 'Cancel'}
        </button>
      </div>
      <progress max="100" value={percent} aria-label="Export progress"></progress>
    {:else}
      <div class="line">
        <strong>{store.cancelled ? 'Export cancelled' : 'Export finished'}</strong>
        <button type="button" onclick={() => store.dismiss()}>Close</button>
      </div>
      <p class="summary" data-testid="export-summary">
        {store.written} written, {store.skipped} skipped, {failedCount} failed
      </p>
      {#if failedCount > 0}
        <details>
          <summary>Show failed photos</summary>
          <ul>
            {#each store.failures as failure (failure.photoId)}
              <li>Photo {failure.photoId}: {failure.message}</li>
            {/each}
          </ul>
        </details>
      {/if}
    {/if}
  </div>
{/if}

<style>
  .panel {
    position: fixed;
    left: var(--space-5);
    bottom: calc(var(--filmstrip-height) + var(--space-5));
    z-index: calc(var(--z-toast) - 1);
    width: 320px;
    padding: var(--space-4);
    background: var(--color-bg-panel-raised);
    border: 1px solid var(--color-border-strong);
    border-radius: var(--radius-md);
    font-size: var(--font-size-sm);
  }
  .line {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
  }
  progress {
    width: 100%;
    margin-top: var(--space-3);
  }
  .summary {
    margin: var(--space-3) 0 0;
  }
  ul {
    margin: var(--space-2) 0 0;
    padding-left: var(--space-5);
    max-height: 160px;
    overflow: auto;
    color: var(--color-danger);
  }
  button {
    background: var(--color-bg-panel);
    color: inherit;
    border: 1px solid var(--color-border-strong);
    border-radius: var(--radius-sm);
    padding: var(--space-2) var(--space-3);
    font: inherit;
    cursor: pointer;
  }
  button:disabled {
    opacity: 0.5;
    cursor: default;
  }
</style>
