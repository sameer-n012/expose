<!--
  The Properties panel of the Editor: the parameters of the selected
  adjustment layer. Brightness/Contrast and Hue/Saturation have sliders.
  Levels and Curves have no editor yet.
-->
<script lang="ts">
  import type { AdjustmentDto } from '../../lib/editor/api';
  import type { EditorStore } from '../../lib/stores/editor.svelte';
  import EditorSlider from './EditorSlider.svelte';

  interface Props {
    editor: EditorStore;
  }

  let { editor }: Props = $props();

  const layer = $derived(editor.selected);
  const adjustment = $derived(layer !== null && layer.kind === 'adjustment' ? layer.adjustment : null);

  /** Dragged values by `layerId:field`; they stay until all edits are done. */
  let drafts = $state<Record<string, number>>({});

  $effect(() => {
    if (editor.idle) {
      drafts = {};
    }
  });

  /** Fraction fields (-1..1) show as percent; `hue` shows in degrees. */
  function shown(field: string, stored: number): number {
    const draft = drafts[`${layer?.id}:${field}`];
    const value = draft ?? stored;
    return field === 'hue' ? Math.round(value) : Math.round(value * 100);
  }

  function setField(field: string, shownValue: number): void {
    if (layer === null || adjustment === null) {
      return;
    }
    const stored = field === 'hue' ? shownValue : shownValue / 100;
    drafts[`${layer.id}:${field}`] = stored;
    // Build the full adjustment from the current values and the dragged values.
    const next: Record<string, unknown> = { ...adjustment };
    for (const key of Object.keys(next)) {
      const draft = drafts[`${layer.id}:${key}`];
      if (draft !== undefined) {
        next[key] = draft;
      }
    }
    editor.liveEdit(`adjustment:${layer.id}`, { type: 'setAdjustment', layerId: layer.id, adjustment: next as AdjustmentDto });
  }

  function reset(field: string): void {
    setField(field, 0);
    editor.endGesture();
  }
</script>

<details class="section" open>
  <summary>Properties</summary>
  <div class="body" role="group" aria-label="Adjustment properties">
    {#if layer === null || adjustment === null}
      <p class="note">Select an adjustment layer to edit it.</p>
    {:else if adjustment.type === 'brightnessContrast'}
      <EditorSlider
        label="Brightness"
        value={shown('brightness', adjustment.brightness)}
        min={-100}
        max={100}
        onchange={(v) => setField('brightness', v)}
        onend={() => editor.endGesture()}
        onreset={() => reset('brightness')}
      />
      <EditorSlider
        label="Contrast"
        value={shown('contrast', adjustment.contrast)}
        min={-100}
        max={100}
        onchange={(v) => setField('contrast', v)}
        onend={() => editor.endGesture()}
        onreset={() => reset('contrast')}
      />
    {:else if adjustment.type === 'hueSaturation'}
      <EditorSlider
        label="Hue"
        value={shown('hue', adjustment.hue)}
        min={-180}
        max={180}
        unit="°"
        onchange={(v) => setField('hue', v)}
        onend={() => editor.endGesture()}
        onreset={() => reset('hue')}
      />
      <EditorSlider
        label="Saturation"
        value={shown('saturation', adjustment.saturation)}
        min={-100}
        max={100}
        onchange={(v) => setField('saturation', v)}
        onend={() => editor.endGesture()}
        onreset={() => reset('saturation')}
      />
      <EditorSlider
        label="Lightness"
        value={shown('lightness', adjustment.lightness)}
        min={-100}
        max={100}
        onchange={(v) => setField('lightness', v)}
        onend={() => editor.endGesture()}
        onreset={() => reset('lightness')}
      />
    {:else}
      <p class="note">No editor yet.</p>
    {/if}
  </div>
</details>

<style>
  .section {
    border-bottom: 1px solid var(--color-border);
  }
  summary {
    padding: var(--space-3) var(--space-4);
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-strong);
    color: var(--color-text-muted);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    cursor: default;
  }
  .note {
    margin: 0;
    padding: 0 var(--space-4) var(--space-4);
    font-size: var(--font-size-sm);
    color: var(--color-text-muted);
  }
</style>
