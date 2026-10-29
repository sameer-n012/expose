<!--
  The options of the brush and the eraser tool: colors, size, hardness,
  opacity, flow, spacing, and the pressure switches.

  - The brush and the eraser have their own settings (`editor.brush` and
    `editor.eraser`).
  - Size is 1 to 5000 px. The slider is logarithmic, so small sizes are easy
    to set. Hardness, opacity, and flow show as percent (0 to 100). Spacing
    shows as percent of the size (1 to 1000).
  - The paint target (pixels or mask of the selected layer) shows as text.
    A click on the mask chip in the Layers panel chooses the mask.
-->
<script lang="ts">
  import { MAX_SIZE, hexToRgb, rgbToHex, sizeToSlider, sliderToSize, type PaintTool } from '../../lib/editor/brush';
  import { BLEND_GROUPS } from '../../lib/editor/layers';
  import type { EditorStore } from '../../lib/stores/editor.svelte';

  interface Props {
    editor: EditorStore;
    tool: PaintTool;
  }

  let { editor, tool }: Props = $props();

  const settings = $derived(editor.brushOf(tool));
  const onMask = $derived(editor.paintTarget === 'mask');

  /** The layer blend modes without dissolve (strokes do not support it). */
  const BRUSH_MODE_GROUPS = BLEND_GROUPS.map((group) => group.filter((mode) => mode.id !== 'dissolve')).filter((group) => group.length > 0);

  function setMode(value: string): void {
    const mode = BRUSH_MODE_GROUPS.flat().find((m) => m.id === value);
    if (mode !== undefined) {
      editor.updateBrush(tool, { blendMode: mode.id });
    }
  }

  /** Sets a 0..1 setting from a percent text. A text that is not a number is dropped. */
  function setPercent(key: 'hardness' | 'opacity' | 'flow' | 'spacing', event: Event & { currentTarget: HTMLInputElement }): void {
    const value = Number(event.currentTarget.value);
    if (Number.isFinite(value)) {
      editor.updateBrush(tool, { [key]: value / 100 });
    }
    event.currentTarget.value = String(Math.round(editor.brushOf(tool)[key] * 100));
  }

  function setSize(value: number, event: Event & { currentTarget: HTMLInputElement }): void {
    if (Number.isFinite(value)) {
      editor.updateBrush(tool, { size: Math.round(value) });
    }
    event.currentTarget.value = String(editor.brushOf(tool).size);
  }

  function setColor(which: 'foreground' | 'background', hex: string): void {
    // The color input always gives "#rrggbb". The check keeps a bad value out of the store.
    if (hexToRgb(hex) !== null) {
      editor[which] = rgbToHex(hexToRgb(hex) ?? [0, 0, 0]);
    }
  }
</script>

<div class="options" role="group" aria-label="Brush options" data-testid="brush-options">
  <div class="group">
    <input
      type="color"
      aria-label="Foreground color"
      title="Foreground color"
      value={editor.foreground}
      oninput={(e) => setColor('foreground', e.currentTarget.value)}
    />
    <input
      type="color"
      aria-label="Background color"
      title="Background color"
      value={editor.background}
      oninput={(e) => setColor('background', e.currentTarget.value)}
    />
    <button type="button" title="Swap the colors (X)" onclick={() => editor.swapColors()}>Swap</button>
    <button type="button" title="Black and white (D)" onclick={() => editor.resetColors()}>Default</button>
  </div>
  <div class="group">
    <label>
      Size
      <input
        type="range"
        min="0"
        max="1"
        step="0.001"
        aria-label="Size slider"
        value={sizeToSlider(settings.size)}
        oninput={(e) => setSize(sliderToSize(Number(e.currentTarget.value)), e)}
      />
    </label>
    <input
      type="number"
      min="1"
      max={MAX_SIZE}
      step="1"
      aria-label="Size (px)"
      value={settings.size}
      onchange={(e) => setSize(Number(e.currentTarget.value), e)}
    />
  </div>
  {#if tool === 'brush'}
    <label title={onMask ? 'Mask paint always uses Normal.' : 'The blend mode of the paint'}>
      Mode
      <select aria-label="Blend mode" disabled={onMask} value={onMask ? 'normal' : settings.blendMode} onchange={(e) => setMode(e.currentTarget.value)}>
        {#each BRUSH_MODE_GROUPS as group, index (index)}
          {#each group as mode (mode.id)}
            <option value={mode.id}>{mode.label}</option>
          {/each}
        {/each}
      </select>
    </label>
  {/if}
  <label>
    Hardness
    <input
      type="number"
      min="0"
      max="100"
      step="5"
      aria-label="Hardness (%)"
      value={Math.round(settings.hardness * 100)}
      onchange={(e) => setPercent('hardness', e)}
    />
  </label>
  <label>
    Opacity
    <input
      type="number"
      min="0"
      max="100"
      step="5"
      aria-label="Opacity (%)"
      value={Math.round(settings.opacity * 100)}
      onchange={(e) => setPercent('opacity', e)}
    />
  </label>
  <label>
    Flow
    <input type="number" min="0" max="100" step="5" aria-label="Flow (%)" value={Math.round(settings.flow * 100)} onchange={(e) => setPercent('flow', e)} />
  </label>
  <label>
    Spacing
    <input
      type="number"
      min="1"
      max="1000"
      step="5"
      aria-label="Spacing (%)"
      value={Math.round(settings.spacing * 100)}
      onchange={(e) => setPercent('spacing', e)}
    />
  </label>
  <label title="A pen presses the size">
    <input type="checkbox" checked={settings.pressureSize} onchange={(e) => editor.updateBrush(tool, { pressureSize: e.currentTarget.checked })} />
    Pressure: size
  </label>
  <label title="A pen presses the opacity">
    <input
      type="checkbox"
      checked={settings.pressureOpacity}
      onchange={(e) => editor.updateBrush(tool, { pressureOpacity: e.currentTarget.checked })}
    />
    Pressure: opacity
  </label>
  <span class="target" data-testid="paint-target">Target: {editor.paintTarget === 'mask' ? 'layer mask' : 'pixels'}</span>
  {#if editor.paintBlockReason !== null}
    <span class="reason" role="status">{editor.paintBlockReason}</span>
  {/if}
</div>

<style>
  .options {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-3);
    align-items: center;
    flex-basis: 100%;
  }
  .group {
    display: flex;
    gap: var(--space-1);
    align-items: center;
  }
  label {
    display: flex;
    gap: var(--space-1);
    align-items: center;
    color: var(--color-text-muted);
  }
  button,
  select,
  input[type='number'] {
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
  input[type='color'] {
    width: 2em;
    height: 1.6em;
    padding: 0;
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
    background: none;
  }
  input[type='range'] {
    width: 8em;
  }
  .target {
    color: var(--color-text-muted);
  }
  .reason {
    color: var(--color-danger);
  }
</style>
