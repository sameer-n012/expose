<!-- A button that opens the current photo in the Editor. -->
<script lang="ts">
  interface Props {
    onedit: () => void;
    disabled?: boolean;
    /** The shortcut text, for the tooltip. */
    shortcut?: string | null;
  }

  let { onedit, disabled = false, shortcut = null }: Props = $props();
</script>

<button type="button" {disabled} title={shortcut === null ? 'Edit in Editor' : `Edit in Editor (${shortcut})`} onclick={onedit}>
  Edit in Editor
</button>

<style>
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
</style>
