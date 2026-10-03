<!--
  Shows the toasts of a toast store. It does not block input to the app.
  Error toasts use role="alert" so that screen readers announce them at once.
-->
<script lang="ts">
  import type { ToastStore } from '../../lib/stores/toasts.svelte';

  interface Props {
    store: ToastStore;
  }

  let { store }: Props = $props();
</script>

<div class="host" aria-live="polite" data-testid="toasts">
  {#each store.items as toast (toast.id)}
    <div class="toast {toast.kind}" role={toast.kind === 'error' ? 'alert' : 'status'}>
      <div class="text">
        <div class="message">{toast.message}</div>
        {#if toast.detail !== undefined}
          <div class="detail">{toast.detail}</div>
        {/if}
      </div>
      <button type="button" class="close" aria-label="Close notification" onclick={() => store.dismiss(toast.id)}>
        &times;
      </button>
    </div>
  {/each}
</div>

<style>
  .host {
    position: fixed;
    right: var(--space-5);
    /* Above the filmstrip, so that toasts do not cover the thumbnails. */
    bottom: calc(var(--filmstrip-height) + var(--space-5));
    z-index: var(--z-toast);
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    max-width: 360px;
    /* Clicks between toasts go to the app below. */
    pointer-events: none;
  }
  .toast {
    pointer-events: auto;
    display: flex;
    align-items: flex-start;
    gap: var(--space-3);
    padding: var(--space-3) var(--space-4);
    background: var(--color-bg-panel-raised);
    border: 1px solid var(--color-border-strong);
    border-left: 3px solid var(--color-info);
    border-radius: var(--radius-md);
    font-size: var(--font-size-md);
    user-select: text;
    -webkit-user-select: text;
  }
  .success {
    border-left-color: var(--color-success);
  }
  .warning {
    border-left-color: var(--color-warning);
  }
  .error {
    border-left-color: var(--color-danger);
  }
  .text {
    flex: 1;
    min-width: 0;
  }
  .detail {
    margin-top: var(--space-1);
    font-size: var(--font-size-sm);
    color: var(--color-text-muted);
    overflow-wrap: anywhere;
  }
  .close {
    background: none;
    border: none;
    padding: 0 var(--space-1);
    font-size: var(--font-size-lg);
    line-height: 1;
    color: var(--color-text-muted);
    cursor: pointer;
  }
  .close:hover {
    color: var(--color-text);
  }
</style>
