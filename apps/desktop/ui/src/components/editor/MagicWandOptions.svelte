<!--
  The options of the magic wand: tolerance (0 to 255), contiguous, sample all
  layers, and anti-alias. The values live in the editor store (`wand`) and are
  shared by all tabs. The selection mode comes from the modifier keys of the
  click, as for the marquee.
-->
<script lang="ts">
  import { MAX_FLOOD_TOLERANCE, clampTolerance } from '../../lib/editor/flood';
  import type { EditorStore } from '../../lib/stores/editor.svelte';

  interface Props {
    editor: EditorStore;
  }

  let { editor }: Props = $props();
</script>

<div class="options" role="group" aria-label="Magic wand options" data-testid="wand-options">
  <label>
    Tolerance
    <input
      type="number"
      min="0"
      max={MAX_FLOOD_TOLERANCE}
      step="1"
      value={editor.wand.tolerance}
      aria-label="Tolerance"
      onchange={(e) => {
        editor.wand.tolerance = clampTolerance(Number(e.currentTarget.value));
        e.currentTarget.value = String(editor.wand.tolerance);
      }}
    />
  </label>
  <label>
    <input type="checkbox" checked={editor.wand.contiguous} onchange={(e) => (editor.wand.contiguous = e.currentTarget.checked)} />
    Contiguous
  </label>
  <label>
    <input type="checkbox" checked={editor.wand.sampleAllLayers} onchange={(e) => (editor.wand.sampleAllLayers = e.currentTarget.checked)} />
    Sample all layers
  </label>
  <label>
    <input type="checkbox" checked={editor.wand.antiAlias} onchange={(e) => (editor.wand.antiAlias = e.currentTarget.checked)} />
    Anti-alias
  </label>
  <span class="hint">Click selects. Shift adds, Alt subtracts, Shift+Alt intersects.</span>
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
  input[type='number'] {
    width: 4.5em;
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
