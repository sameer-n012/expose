<!--
  The Feather dialog (modal): the radius of the feather in pixels, 0 to 250.
  OK feathers the selection. Cancel and Esc close the dialog.

  Keyboard: Tab and Shift+Tab stay inside the dialog. Key events do not
  leave the dialog, so shortcuts of the shell do not run behind it.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { MAX_FEATHER } from '../../lib/editor/selection';
  import type { EditorStore } from '../../lib/stores/editor.svelte';

  interface Props {
    editor: EditorStore;
  }

  let { editor }: Props = $props();

  let dialog: HTMLDivElement | undefined = $state();
  let input: HTMLInputElement | undefined = $state();
  let text = $state('5');

  const radius = $derived(text.trim() === '' ? Number.NaN : Number(text));
  const problem = $derived(
    Number.isFinite(radius) && radius >= 0 && radius <= MAX_FEATHER ? null : `Enter a radius from 0 to ${MAX_FEATHER} pixels.`,
  );

  onMount(() => {
    const previous = document.activeElement;
    input?.focus();
    input?.select();
    return () => {
      if (previous instanceof HTMLElement) {
        previous.focus();
      }
    };
  });

  function accept(): void {
    if (problem !== null) {
      return;
    }
    editor.closeFeatherDialog();
    editor.featherSelection(radius);
  }

  function onkeydown(event: KeyboardEvent): void {
    // Shell shortcuts must not run while the dialog is open.
    event.stopPropagation();
    if (event.key === 'Escape') {
      event.preventDefault();
      editor.closeFeatherDialog();
      return;
    }
    if (event.key === 'Enter' && event.target instanceof HTMLInputElement) {
      event.preventDefault();
      accept();
      return;
    }
    if (event.key !== 'Tab' || dialog === undefined) {
      return;
    }
    const items = [...dialog.querySelectorAll<HTMLElement>('button:not(:disabled), input')];
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
<div class="backdrop" onclick={(e) => e.target === e.currentTarget && editor.closeFeatherDialog()}>
  <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="feather-title" tabindex="-1" bind:this={dialog} {onkeydown}>
    <h2 id="feather-title">Feather selection</h2>
    <div class="grid">
      <label for="feather-radius">Radius (px)</label>
      <input id="feather-radius" type="number" min="0" max={MAX_FEATHER} step="1" bind:this={input} value={text} oninput={(e) => (text = e.currentTarget.value)} />
    </div>
    <p class="problem" role="alert" data-testid="feather-problem">{problem ?? ''}</p>
    <div class="actions">
      <button type="button" onclick={() => editor.closeFeatherDialog()}>Cancel</button>
      <button type="button" class="primary" disabled={problem !== null} onclick={accept}>OK</button>
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
    width: min(340px, 92vw);
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
  .grid {
    display: grid;
    grid-template-columns: max-content 1fr;
    align-items: center;
    gap: var(--space-3);
  }
  input,
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
