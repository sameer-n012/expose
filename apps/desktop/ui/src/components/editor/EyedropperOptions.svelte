<!--
  The options of the eyedropper tool: the sample size (1x1, 3x3, 5x5) and the
  source (all layers, or the selected layer). The values live in the editor
  store (`sampleSize`, `sampleSource`) and are shared by all tabs.
-->
<script lang="ts">
  import type { SampleSource } from '../../lib/editor/api';
  import type { EditorStore } from '../../lib/stores/editor.svelte';

  interface Props {
    editor: EditorStore;
  }

  let { editor }: Props = $props();

  const SIZES: (1 | 3 | 5)[] = [1, 3, 5];

  function setSize(value: string): void {
    const size = Number(value);
    if (size === 1 || size === 3 || size === 5) {
      editor.sampleSize = size;
    }
  }

  function setSource(value: string): void {
    if (value === 'composite' || value === 'layer') {
      editor.sampleSource = value satisfies SampleSource;
    }
  }
</script>

<div class="options" role="group" aria-label="Eyedropper options" data-testid="eyedropper-options">
  <label>
    Sample size
    <select aria-label="Sample size" value={String(editor.sampleSize)} onchange={(e) => setSize(e.currentTarget.value)}>
      {#each SIZES as size (size)}
        <option value={String(size)}>{size} × {size}</option>
      {/each}
    </select>
  </label>
  <label>
    Sample
    <select aria-label="Sample source" value={editor.sampleSource} onchange={(e) => setSource(e.currentTarget.value)}>
      <option value="composite">All layers</option>
      <option value="layer">Current layer</option>
    </select>
  </label>
  <span class="hint">Click sets the foreground color. Alt+click sets the background color.</span>
</div>

<style>
  .options {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-3);
    align-items: center;
    flex-basis: 100%;
  }
  label {
    display: flex;
    gap: var(--space-1);
    align-items: center;
    color: var(--color-text-muted);
  }
  select {
    background: var(--color-bg-panel-raised);
    color: var(--color-text);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
    padding: var(--space-1) var(--space-2);
    font-size: var(--font-size-sm);
  }
  .hint {
    color: var(--color-text-muted);
  }
</style>
