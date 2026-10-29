<!--
  The History panel of the Editor: the undo steps of the active document.
  Row 0 is the start state. A click on a row undoes or redoes to that state.
  Rows after the current row are steps that can be redone.
-->
<script lang="ts">
  import { tick } from 'svelte';
  import type { EditorStore } from '../../lib/stores/editor.svelte';

  interface Props {
    editor: EditorStore;
  }

  let { editor }: Props = $props();

  const history = $derived(editor.info?.history ?? null);
  const rows = $derived(history === null ? [] : ['Start', ...history.steps]);
  const cursor = $derived(history?.cursor ?? 0);

  let list: HTMLElement | undefined = $state();

  // Keep the current row visible when the cursor or the step list changes.
  $effect(() => {
    void cursor;
    void rows.length;
    void tick().then(() => {
      list?.querySelector<HTMLElement>('[aria-selected="true"]')?.scrollIntoView?.({ block: 'nearest' });
    });
  });

  function onkeydown(event: KeyboardEvent, index: number): void {
    if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault();
      editor.historyJump(index);
    }
  }
</script>

<details class="section" open>
  <summary>History</summary>
  {#if history === null}
    <p class="note">No document</p>
  {:else}
    <div class="list" role="listbox" aria-label="History" bind:this={list}>
      {#each rows as name, index (index)}
        <div
          class="row"
          class:dim={index > cursor}
          role="option"
          tabindex="0"
          aria-selected={index === cursor}
          onclick={() => editor.historyJump(index)}
          onkeydown={(event) => onkeydown(event, index)}
        >
          {name}
        </div>
      {/each}
    </div>
  {/if}
</details>

<style>
  .section {
    border-bottom: 1px solid var(--color-border);
  }
  summary {
    padding: var(--space-3) var(--space-4);
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-strong);
    color: var(--color-text-muted);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    cursor: default;
  }
  .note {
    margin: 0;
    padding: 0 var(--space-4) var(--space-4);
    font-size: var(--font-size-sm);
    color: var(--color-text-muted);
  }
  .list {
    max-height: 14rem;
    overflow-y: auto;
    padding-bottom: var(--space-2);
  }
  .row {
    padding: var(--space-1) var(--space-4);
    font-size: var(--font-size-sm);
    cursor: default;
  }
  .row:hover {
    background: rgb(255 255 255 / 6%);
  }
  .row[aria-selected='true'] {
    background: rgb(255 255 255 / 12%);
    font-weight: var(--font-weight-strong);
  }
  .row.dim {
    color: var(--color-text-muted);
    opacity: 0.6;
  }
</style>
