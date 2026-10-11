<!--
  The unsaved-changes question of the Editor (modal): Save, Don't save, or
  Cancel. The editor store opens it before it replaces a document that has
  changes that are not saved.

  Keyboard: Tab and Shift+Tab stay inside the dialog. Esc is Cancel. Key
  events do not leave the dialog, so shortcuts of the shell do not run
  behind it.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import type { EditorStore } from '../../lib/stores/editor.svelte';

  interface Props {
    editor: EditorStore;
  }

  let { editor }: Props = $props();

  let dialog: HTMLDivElement | undefined = $state();
  let saveButton: HTMLButtonElement | undefined = $state();

  onMount(() => {
    const previous = document.activeElement;
    saveButton?.focus();
    return () => {
      // Give the focus back to the control that had it.
      if (previous instanceof HTMLElement) {
        previous.focus();
      }
    };
  });

  function onkeydown(event: KeyboardEvent): void {
    // Shell shortcuts must not run while the dialog is open.
    event.stopPropagation();
    if (event.key === 'Escape') {
      event.preventDefault();
      editor.answerLeave('cancel');
      return;
    }
    if (event.key !== 'Tab' || dialog === undefined) {
      return;
    }
    const items = [...dialog.querySelectorAll<HTMLElement>('button:not(:disabled)')];
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
<div class="backdrop" onclick={(e) => e.target === e.currentTarget && editor.answerLeave('cancel')}>
  <div
    class="dialog"
    role="alertdialog"
    aria-modal="true"
    aria-labelledby="leave-title"
    aria-describedby="leave-message"
    tabindex="-1"
    bind:this={dialog}
    {onkeydown}
  >
    <h2 id="leave-title">Save the changes?</h2>
    <p id="leave-message">{editor.leavePrompt?.message ?? ''} If you do not save them, they are lost.</p>
    <div class="actions">
      <button type="button" onclick={() => editor.answerLeave('discard')}>Don't save</button>
      <button type="button" onclick={() => editor.answerLeave('cancel')}>Cancel</button>
      <button type="button" class="primary" bind:this={saveButton} onclick={() => editor.answerLeave('save')}>Save</button>
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
    width: min(420px, 92vw);
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
  p {
    margin: 0 0 var(--space-5);
    color: var(--color-text-muted);
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-3);
  }
  .primary {
    background: var(--color-accent);
    color: var(--color-text-on-accent);
  }
</style>
