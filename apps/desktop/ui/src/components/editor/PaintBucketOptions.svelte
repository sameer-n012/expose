<!--
  The options of the paint bucket: tolerance (0 to 255), contiguous, sample all
  layers, anti-alias, opacity (0 to 100 %), and the blend mode. The color is
  the foreground color. The values live in the editor store (`bucket`) and are
  shared by all tabs. The bucket fills a pixel layer, not a mask.
-->
<script lang="ts">
  import { clampUnit } from '../../lib/editor/brush';
  import { MAX_FLOOD_TOLERANCE, clampTolerance } from '../../lib/editor/flood';
  import { BLEND_GROUPS } from '../../lib/editor/layers';
  import type { EditorStore } from '../../lib/stores/editor.svelte';

  interface Props {
    editor: EditorStore;
  }

  let { editor }: Props = $props();

  /** The layer blend modes without dissolve (the fill does not support it). */
  const MODE_GROUPS = BLEND_GROUPS.map((group) => group.filter((mode) => mode.id !== 'dissolve')).filter((group) => group.length > 0);

  function setMode(value: string): void {
    const mode = MODE_GROUPS.flat().find((m) => m.id === value);
    if (mode !== undefined) {
      editor.bucket.blendMode = mode.id;
    }
  }
</script>

<div class="options" role="group" aria-label="Paint bucket options" data-testid="bucket-options">
  <label>
    Tolerance
    <input
      type="number"
      min="0"
      max={MAX_FLOOD_TOLERANCE}
      step="1"
      value={editor.bucket.tolerance}
      aria-label="Tolerance"
      onchange={(e) => {
        editor.bucket.tolerance = clampTolerance(Number(e.currentTarget.value));
        e.currentTarget.value = String(editor.bucket.tolerance);
      }}
    />
  </label>
  <label>
    <input type="checkbox" checked={editor.bucket.contiguous} onchange={(e) => (editor.bucket.contiguous = e.currentTarget.checked)} />
    Contiguous
  </label>
  <label>
    <input type="checkbox" checked={editor.bucket.sampleAllLayers} onchange={(e) => (editor.bucket.sampleAllLayers = e.currentTarget.checked)} />
    Sample all layers
  </label>
  <label>
    <input type="checkbox" checked={editor.bucket.antiAlias} onchange={(e) => (editor.bucket.antiAlias = e.currentTarget.checked)} />
    Anti-alias
  </label>
  <label>
    Opacity (%)
    <input
      type="number"
      min="0"
      max="100"
      step="1"
      value={Math.round(editor.bucket.opacity * 100)}
      aria-label="Opacity (%)"
      onchange={(e) => {
        const percent = Number(e.currentTarget.value);
        editor.bucket.opacity = Number.isFinite(percent) ? clampUnit(percent / 100) : editor.bucket.opacity;
        e.currentTarget.value = String(Math.round(editor.bucket.opacity * 100));
      }}
    />
  </label>
  <label>
    Mode
    <select aria-label="Blend mode" value={editor.bucket.blendMode} onchange={(e) => setMode(e.currentTarget.value)}>
      {#each MODE_GROUPS as group, index (index)}
        {#each group as mode (mode.id)}
          <option value={mode.id}>{mode.label}</option>
        {/each}
      {/each}
    </select>
  </label>
  <span class="hint">Click fills with the foreground color.</span>
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
  input[type='number'],
  select {
    background: var(--color-bg-panel-raised);
    color: var(--color-text);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
    padding: var(--space-1) var(--space-2);
    font-size: var(--font-size-sm);
  }
  input[type='number'] {
    width: 4.5em;
  }
  .hint {
    color: var(--color-text-muted);
  }
</style>
