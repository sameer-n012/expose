<!--
  The options of the clone stamp and the healing brush: size, hardness,
  opacity, flow, spacing, the pressure switches, "Aligned", and the layers to
  sample.

  - Both tools share one set of settings (`editor.clone`).
  - Alt+click on the canvas sets the source point.
  - Size is 1 to 5000 px (logarithmic slider). Hardness, opacity, and flow
    show as percent. Spacing shows as percent of the size.
-->
<script lang="ts">
  import { MAX_SIZE, sizeToSlider, sliderToSize } from '../../lib/editor/brush';
  import type { CloneTool } from '../../lib/editor/clone';
  import type { EditorStore } from '../../lib/stores/editor.svelte';

  interface Props {
    editor: EditorStore;
    tool: CloneTool;
  }

  let { editor, tool }: Props = $props();

  const settings = $derived(editor.clone);
  const label = $derived(tool === 'cloneStamp' ? 'Clone stamp' : 'Healing brush');

  /** Sets a 0..1 setting from a percent text. A text that is not a number is dropped. */
  function setPercent(key: 'hardness' | 'opacity' | 'flow' | 'spacing', event: Event & { currentTarget: HTMLInputElement }): void {
    const value = Number(event.currentTarget.value);
    if (Number.isFinite(value)) {
      editor.updateBrush(tool, { [key]: value / 100 });
    }
    event.currentTarget.value = String(Math.round(editor.clone[key] * 100));
  }

  function setSize(value: number, event: Event & { currentTarget: HTMLInputElement }): void {
    if (Number.isFinite(value)) {
      editor.updateBrush(tool, { size: Math.round(value) });
    }
    event.currentTarget.value = String(editor.clone.size);
  }
</script>

<div class="options" role="group" aria-label="Clone options" data-testid="clone-options">
  <span class="name" data-testid="clone-tool-name">{label}</span>
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
  <label title="On: the offset to the source point stays between strokes. Off: each stroke starts at the source point.">
    <input type="checkbox" checked={editor.cloneAligned} onchange={(e) => (editor.cloneAligned = e.currentTarget.checked)} />
    Aligned
  </label>
  <label title="The layers that the tool copies from">
    Sample
    <select aria-label="Sample" value={editor.cloneSampleAll ? 'all' : 'current'} onchange={(e) => (editor.cloneSampleAll = e.currentTarget.value === 'all')}>
      <option value="current">Current layer</option>
      <option value="all">All layers</option>
    </select>
  </label>
  <span class="hint">Alt+click sets the source point.</span>
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
  input[type='range'] {
    width: 8em;
  }
  .name {
    color: var(--color-text);
    font-weight: 600;
  }
  .hint {
    color: var(--color-text-muted);
  }
</style>
