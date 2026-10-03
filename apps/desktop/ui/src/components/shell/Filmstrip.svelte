<!--
  The filmstrip at the bottom of the window. It is shared by all modules.
  Placeholder thumbnails until the catalog backend is available.
-->
<script lang="ts">
  interface Props {
    visible: boolean;
  }

  let { visible }: Props = $props();

  const PLACEHOLDER_COUNT = 24;
  const items = Array.from({ length: PLACEHOLDER_COUNT }, (_, i) => i);
  let selected = $state(0);
</script>

<footer class="filmstrip" aria-label="Filmstrip" hidden={!visible} data-testid="filmstrip">
  <div class="strip">
    {#each items as i (i)}
      <button
        type="button"
        class="thumb"
        class:selected={i === selected}
        aria-label="Photo {i + 1}"
        aria-pressed={i === selected}
        onclick={() => (selected = i)}
      ></button>
    {/each}
  </div>
</footer>

<style>
  .filmstrip {
    height: var(--filmstrip-height);
    background: var(--color-bg-panel);
    border-top: 1px solid var(--color-border);
  }
  .strip {
    height: 100%;
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: 0 var(--space-3);
    overflow-x: auto;
  }
  .thumb {
    flex: none;
    height: calc(var(--filmstrip-height) - 2 * var(--space-4));
    aspect-ratio: 3 / 2;
    background: var(--color-bg-placeholder);
    border: 2px solid transparent;
    border-radius: var(--radius-sm);
    padding: 0;
    cursor: pointer;
  }
  .thumb:hover {
    border-color: var(--color-border-strong);
  }
  .thumb.selected {
    border-color: var(--color-text-muted);
  }
</style>
