<!--
  The New document dialog of the Editor (modal): name, size, resolution,
  bit depth, and background. The size presets fill the size and resolution.

  The Create button is disabled while the values are not valid, and the
  first problem shows as a message. On Create, the editor store makes the
  document (it asks about unsaved changes of the old document first).

  Keyboard: Tab and Shift+Tab stay inside the dialog. Esc is Cancel. Enter
  in a field is Create. Key events do not leave the dialog, so shortcuts of
  the shell do not run behind it.
-->
<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import type { BackgroundId, NewDocumentSpec } from '../../lib/editor/api';
  import { DOCUMENT_PRESETS, defaultSpec, validateSpec, type DocumentPreset } from '../../lib/editor/newDocument';
  import type { EditorStore } from '../../lib/stores/editor.svelte';

  interface Props {
    editor: EditorStore;
  }

  let { editor }: Props = $props();

  let spec = $state<NewDocumentSpec>(defaultSpec(untrack(() => editor.suggestedName)));
  let creating = $state(false);
  let dialog: HTMLDivElement | undefined = $state();
  let nameInput: HTMLInputElement | undefined = $state();

  const problem = $derived(validateSpec(spec));

  const BACKGROUNDS: ReadonlyArray<{ id: BackgroundId; label: string }> = [
    { id: 'white', label: 'White' },
    { id: 'black', label: 'Black' },
    { id: 'transparent', label: 'Transparent' },
  ];
  const BIT_DEPTHS: ReadonlyArray<NewDocumentSpec['bitDepth']> = [8, 16, 32];

  onMount(() => {
    const previous = document.activeElement;
    nameInput?.focus();
    nameInput?.select();
    return () => {
      if (previous instanceof HTMLElement) {
        previous.focus();
      }
    };
  });

  function applyPreset(preset: DocumentPreset): void {
    spec.width = preset.width;
    spec.height = preset.height;
    spec.resolutionPpi = preset.resolutionPpi;
  }

  /** Reads a number input; an empty or invalid field is NaN, which the validation refuses. */
  function numberOf(event: Event & { currentTarget: HTMLInputElement }): number {
    return event.currentTarget.valueAsNumber;
  }

  async function create(): Promise<void> {
    if (problem !== null || creating) {
      return;
    }
    creating = true;
    try {
      editor.closeNewDocument();
      await editor.newDocument({ ...spec, name: spec.name.trim() });
    } finally {
      creating = false;
    }
  }

  function onkeydown(event: KeyboardEvent): void {
    // Shell shortcuts must not run while the dialog is open.
    event.stopPropagation();
    if (event.key === 'Escape') {
      event.preventDefault();
      editor.closeNewDocument();
      return;
    }
    if (event.key === 'Enter' && event.target instanceof HTMLInputElement) {
      event.preventDefault();
      void create();
      return;
    }
    if (event.key !== 'Tab' || dialog === undefined) {
      return;
    }
    const items = [...dialog.querySelectorAll<HTMLElement>('button:not(:disabled), input, select')];
    const first = items[0];
    const last = items[items.length - 1];
    if (first === undefined || last === undefined) {
      return;
    }
    const active = document.activeElement;
    if (event.shiftKey && (active === first || !dialog.contains(active))) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && (active === last || !dialog.contains(active))) {
      event.preventDefault();
      first.focus();
    }
  }
</script>

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div class="backdrop" onclick={(e) => e.target === e.currentTarget && editor.closeNewDocument()}>
  <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="newdoc-title" tabindex="-1" bind:this={dialog} {onkeydown}>
    <h2 id="newdoc-title">New document</h2>

    <div class="presets" role="group" aria-label="Presets">
      {#each DOCUMENT_PRESETS as preset (preset.id)}
        <button type="button" onclick={() => applyPreset(preset)}>{preset.label}</button>
      {/each}
    </div>

    <div class="grid">
      <label for="newdoc-name">Name</label>
      <input id="newdoc-name" type="text" bind:this={nameInput} bind:value={spec.name} />

      <label for="newdoc-width">Width (px)</label>
      <input id="newdoc-width" type="number" min="1" step="1" value={spec.width} oninput={(e) => (spec.width = numberOf(e))} />

      <label for="newdoc-height">Height (px)</label>
      <input id="newdoc-height" type="number" min="1" step="1" value={spec.height} oninput={(e) => (spec.height = numberOf(e))} />

      <label for="newdoc-ppi">Resolution (ppi)</label>
      <input id="newdoc-ppi" type="number" min="1" step="1" value={spec.resolutionPpi} oninput={(e) => (spec.resolutionPpi = numberOf(e))} />

      <label for="newdoc-depth">Bit depth</label>
      <select id="newdoc-depth" value={String(spec.bitDepth)} onchange={(e) => (spec.bitDepth = Number(e.currentTarget.value) as NewDocumentSpec['bitDepth'])}>
        {#each BIT_DEPTHS as depth (depth)}
          <option value={String(depth)}>{depth} bit</option>
        {/each}
      </select>

      <label for="newdoc-background">Background</label>
      <select id="newdoc-background" bind:value={spec.background}>
        {#each BACKGROUNDS as background (background.id)}
          <option value={background.id}>{background.label}</option>
        {/each}
      </select>
    </div>

    <p class="problem" role="alert" data-testid="newdoc-problem">{problem ?? ''}</p>

    <div class="actions">
      <button type="button" onclick={() => editor.closeNewDocument()}>Cancel</button>
      <button type="button" class="primary" disabled={problem !== null || creating} onclick={() => void create()}>Create</button>
    </div>
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: var(--z-toast);
    display: flex;
    align-items: center;
    justify-content: center;
    background: rgb(0 0 0 / 55%);
  }
  .dialog {
    width: min(460px, 92vw);
    padding: var(--space-5);
    background: var(--color-bg-panel);
    border: 1px solid var(--color-border-strong);
    border-radius: var(--radius-md);
    font-size: var(--font-size-md);
  }
  h2 {
    margin: 0 0 var(--space-3);
    font-size: var(--font-size-xl);
  }
  .presets {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
    margin-bottom: var(--space-4);
  }
  .grid {
    display: grid;
    grid-template-columns: max-content 1fr;
    align-items: center;
    gap: var(--space-3);
  }
  input,
  select,
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
  .problem {
    min-height: 1.4em;
    margin: var(--space-3) 0;
    font-size: var(--font-size-sm);
    color: var(--color-text-muted);
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-3);
  }
  .primary:not(:disabled) {
    background: var(--color-accent);
    color: var(--color-text-on-accent);
  }
</style>
