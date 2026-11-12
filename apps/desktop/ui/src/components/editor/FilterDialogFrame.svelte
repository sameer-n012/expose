<!--
  The frame of a filter dialog (Gaussian blur, unsharp mask).

  - It is not modal. It sits at the lower right of the stage, so the canvas
    centre stays free. Zoom and pan keep working.
  - The "Preview" box (on at the start) shows the filter on the layer. A new
    value asks for a new preview; the client keeps one call in flight.
  - OK keeps the filter as one history step. Cancel and Escape restore the
    layer. Enter in a field is OK.
  - The dialog closes with a cancel when the selected layer or the paint
    target changes.
-->
<script lang="ts">
  import { onMount, untrack, type Snippet } from 'svelte';
  import type { FilterDto } from '../../lib/editor/api';
  import type { EditorStore, FilterSession } from '../../lib/stores/editor.svelte';

  interface Props {
    editor: EditorStore;
    session: FilterSession;
    title: string;
    /** The filter for the values now. */
    filter: FilterDto;
    children: Snippet;
  }

  let { editor, session, title, filter, children }: Props = $props();

  let dialog: HTMLDivElement | undefined = $state();
  let preview = $state(true);
  let busy = $state(false);

  // Every new value, and the box, ask for a preview or cancel it.
  $effect(() => {
    const current = filter;
    const on = preview;
    untrack(() => {
      if (on) {
        session.client.preview(current);
      } else {
        void session.client.cancelSession();
      }
    });
  });

  // Another layer or another paint target: the filter does not match any more.
  $effect(() => {
    if (editor.selectedId !== session.layerId || editor.filterTarget !== session.target) {
      untrack(() => editor.closeFilterDialog());
    }
  });

  onMount(() => {
    dialog?.querySelector<HTMLInputElement>('input[type="number"]')?.focus();
  });

  async function accept(): Promise<void> {
    if (busy) {
      return;
    }
    busy = true;
    const done = await session.client.commit(filter);
    busy = false;
    if (done && editor.filterDialog === session) {
      editor.closeFilterDialog();
    }
  }

  function cancel(): void {
    editor.closeFilterDialog();
  }

  function onkeydown(event: KeyboardEvent): void {
    // Shell shortcuts must not run while the dialog has the focus.
    event.stopPropagation();
    if (event.key === 'Escape') {
      event.preventDefault();
      cancel();
    } else if (event.key === 'Enter' && !(event.target instanceof HTMLButtonElement)) {
      event.preventDefault();
      void accept();
    }
  }
</script>

<div class="dialog" role="dialog" aria-labelledby="filter-title" tabindex="-1" bind:this={dialog} {onkeydown} data-testid="filter-dialog">
  <h2 id="filter-title">{title}</h2>
  {@render children()}
  <label class="preview">
    <input type="checkbox" bind:checked={preview} />
    Preview
  </label>
  <div class="actions">
    <button type="button" onclick={cancel}>Cancel</button>
    <button type="button" class="primary" disabled={busy} onclick={() => void accept()}>OK</button>
  </div>
</div>

<style>
  .dialog {
    position: absolute;
    right: var(--space-3);
    bottom: var(--space-3);
    z-index: 2;
    width: min(300px, 90%);
    padding: var(--space-4);
    background: var(--color-bg-panel);
    border: 1px solid var(--color-border-strong);
    border-radius: var(--radius-md);
    font-size: var(--font-size-md);
  }
  h2 {
    margin: 0 0 var(--space-2);
    font-size: var(--font-size-lg);
  }
  .preview {
    display: flex;
    gap: var(--space-2);
    align-items: center;
    margin: var(--space-2) 0;
    font-size: var(--font-size-sm);
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-3);
  }
  button {
    background: var(--color-bg-panel-raised);
    color: var(--color-text);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
    padding: var(--space-1) var(--space-3);
    font-size: var(--font-size-sm);
  }
  button:disabled {
    opacity: 0.5;
  }
  .primary:not(:disabled) {
    background: var(--color-accent);
    color: var(--color-text-on-accent);
  }
</style>
