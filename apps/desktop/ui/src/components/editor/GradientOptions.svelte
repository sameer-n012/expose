<!--
  The options of the gradient tool: shape, the gradient swatch (opens the
  gradient editor), a preset, opacity (0 to 100 %), the blend mode, and
  reverse. The values live in the editor store (`gradient`) and are shared by
  all tabs. The foreground and background presets follow the current colors.
  On a mask the mode is always Normal.
-->
<script lang="ts">
  import { clampUnit, hexToRgb } from '../../lib/editor/brush';
  import { GRADIENT_PRESETS, GRADIENT_SHAPES, cssGradient, presetStops } from '../../lib/editor/gradient';
  import { BLEND_GROUPS } from '../../lib/editor/layers';
  import type { EditorStore } from '../../lib/stores/editor.svelte';

  interface Props {
    editor: EditorStore;
  }

  let { editor }: Props = $props();

  /** The layer blend modes without dissolve (the gradient does not support it). */
  const MODE_GROUPS = BLEND_GROUPS.map((group) => group.filter((mode) => mode.id !== 'dissolve')).filter((group) => group.length > 0);

  const onMask = $derived(editor.paintTarget === 'mask');
  const stops = $derived(presetStops(editor.gradient, hexToRgb(editor.foreground) ?? [0, 0, 0], hexToRgb(editor.background) ?? [1, 1, 1]));
  const swatch = $derived(cssGradient(stops, editor.gradient.reverse));

  function setPreset(value: string): void {
    const preset = GRADIENT_PRESETS.find((p) => p.id === value);
    if (preset === undefined) {
      return;
    }
    if (preset.id === 'custom') {
      // Custom keeps the stops that the swatch shows now.
      editor.gradient.stops = stops;
    }
    editor.gradient.preset = preset.id;
  }

  function setMode(value: string): void {
    const mode = MODE_GROUPS.flat().find((m) => m.id === value);
    if (mode !== undefined) {
      editor.gradient.blendMode = mode.id;
    }
  }
</script>

<div class="options" role="group" aria-label="Gradient options" data-testid="gradient-options">
  <div class="shapes" role="group" aria-label="Gradient shape">
    {#each GRADIENT_SHAPES as shape (shape.id)}
      <button type="button" aria-pressed={editor.gradient.shape === shape.id} title={shape.title} onclick={() => (editor.gradient.shape = shape.id)}>
        {shape.label}
      </button>
    {/each}
  </div>
  <button type="button" class="swatch" aria-label="Edit the gradient" title="Edit the gradient" style:background-image={swatch} onclick={() => editor.openGradientDialog()}></button>
  <label>
    Preset
    <select aria-label="Gradient preset" value={editor.gradient.preset} onchange={(e) => setPreset(e.currentTarget.value)}>
      {#each GRADIENT_PRESETS as preset (preset.id)}
        <option value={preset.id}>{preset.label}</option>
      {/each}
    </select>
  </label>
  <label>
    Opacity (%)
    <input
      type="number"
      min="0"
      max="100"
      step="1"
      value={Math.round(editor.gradient.opacity * 100)}
      aria-label="Opacity (%)"
      onchange={(e) => {
        const percent = Number(e.currentTarget.value);
        editor.gradient.opacity = Number.isFinite(percent) ? clampUnit(percent / 100) : editor.gradient.opacity;
        e.currentTarget.value = String(Math.round(editor.gradient.opacity * 100));
      }}
    />
  </label>
  <label title={onMask ? 'Mask paint always uses Normal.' : 'The blend mode of the gradient'}>
    Mode
    <select aria-label="Blend mode" disabled={onMask} value={onMask ? 'normal' : editor.gradient.blendMode} onchange={(e) => setMode(e.currentTarget.value)}>
      {#each MODE_GROUPS as group, index (index)}
        {#each group as mode (mode.id)}
          <option value={mode.id}>{mode.label}</option>
        {/each}
      {/each}
    </select>
  </label>
  <label>
    <input type="checkbox" checked={editor.gradient.reverse} onchange={(e) => (editor.gradient.reverse = e.currentTarget.checked)} />
    Reverse
  </label>
</div>

<style>
  .options {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-3);
    align-items: center;
    flex-basis: 100%;
  }
  .shapes {
    display: flex;
    gap: var(--space-1);
  }
  label {
    display: flex;
    gap: var(--space-1);
    align-items: center;
    color: var(--color-text-muted);
  }
  input[type='number'],
  select,
  button {
    background: var(--color-bg-panel-raised);
    color: var(--color-text);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
    padding: var(--space-1) var(--space-2);
    font-size: var(--font-size-sm);
  }
  button[aria-pressed='true'] {
    background: var(--color-accent);
    color: var(--color-text-on-accent);
  }
  input[type='number'] {
    width: 4.5em;
  }
  .swatch {
    width: 8em;
    height: 1.6em;
    padding: 0;
    /* A checkerboard shows through the transparent parts. */
    background-color: #fff;
  }
</style>
