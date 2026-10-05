<!--
  The Crop panel of the Develop module: crop tool start and end, angle,
  flips, aspect, overlay, and reset.

  The controls work also when the crop tool is not active (the preview
  then shows the result at once). They need a preview session, because
  the session size gives the aspect of the image.
-->
<script lang="ts">
  import { ASPECT_PRESETS, CROP_ANGLE_RANGE, aspectFromId, aspectId } from '../../lib/develop/crop';
  import { CROP_OVERLAYS, CROP_OVERLAY_LABELS, type CropOverlayKind } from '../../lib/develop/cropView';
  import type { DevelopStore } from '../../lib/stores/develop.svelte';
  import SliderRow from './SliderRow.svelte';

  interface Props {
    develop: DevelopStore;
  }

  let { develop }: Props = $props();

  const uid = $props.id();
  const crop = $derived(develop.crop);
  const usable = $derived(crop !== null && develop.frameSize !== null);
  const currentAspect = $derived(crop === null ? 'original' : aspectId(crop.aspect));
  const locked = $derived(crop !== null && crop.aspect !== 'free');
  /** The presets, plus the current ratio when it is not a preset. */
  const aspectOptions = $derived(
    ASPECT_PRESETS.some((p) => p.id === currentAspect)
      ? ASPECT_PRESETS
      : [...ASPECT_PRESETS, { id: currentAspect, label: currentAspect.replace(':', ' : '), aspect: 'free' as const }],
  );

  function onAspect(event: Event & { currentTarget: HTMLSelectElement }): void {
    const aspect = aspectFromId(event.currentTarget.value);
    if (aspect !== null) {
      develop.setCropAspect(aspect);
    }
  }

  function onOverlay(event: Event & { currentTarget: HTMLSelectElement }): void {
    const value = event.currentTarget.value;
    const kind = CROP_OVERLAYS.find((k) => k === value);
    if (kind !== undefined) {
      develop.cropOverlay = kind satisfies CropOverlayKind;
    }
  }

  /** The angle drag shows the fine grid until the pointer goes up anywhere. */
  function endStraighten(): void {
    if (develop.straightening) {
      develop.straightening = false;
    }
  }
</script>

<svelte:window onpointerup={endStraighten} onpointercancel={endStraighten} />

<details class="section" open>
  <summary>Crop</summary>
  <div class="body" role="group" aria-label="Crop and straighten">
    <div class="actions">
      {#if develop.cropActive}
        <button type="button" class="primary" onclick={() => develop.commitCrop()} title="Keep the crop (Enter)">
          Done
        </button>
        <button type="button" onclick={() => develop.cancelCrop()} title="Restore the crop (Esc)">Cancel</button>
      {:else}
        <button type="button" disabled={!usable} onclick={() => develop.enterCrop()} title="Crop tool (R)">
          Crop
        </button>
      {/if}
    </div>

    <!-- The pointer-down starts the fine grid of the angle drag. -->
    <div class="angle" onpointerdown={() => (develop.straightening = develop.cropActive)} role="presentation">
      <SliderRow
        label="Angle"
        value={crop?.angle ?? 0}
        {...CROP_ANGLE_RANGE}
        unit="°"
        decimals={1}
        disabled={!usable}
        onchange={(v) => develop.setCropAngle(v)}
        onreset={() => develop.setCropAngle(0)}
      />
    </div>

    <div class="row buttons">
      <button
        type="button"
        disabled={!usable}
        aria-pressed={crop?.flip_horizontal ?? false}
        onclick={() => develop.flipCrop('horizontal')}
      >
        Flip horizontal
      </button>
      <button
        type="button"
        disabled={!usable}
        aria-pressed={crop?.flip_vertical ?? false}
        onclick={() => develop.flipCrop('vertical')}
      >
        Flip vertical
      </button>
    </div>

    <div class="row">
      <label for="{uid}-aspect">Aspect</label>
      <select id="{uid}-aspect" disabled={!usable} value={currentAspect} onchange={onAspect}>
        {#each aspectOptions as preset (preset.id)}
          <option value={preset.id}>{preset.label}</option>
        {/each}
      </select>
      <button
        type="button"
        disabled={!usable}
        aria-pressed={locked}
        aria-label="Lock aspect"
        title={locked ? 'Unlock the aspect' : 'Lock the aspect to the current shape'}
        onclick={() => develop.setCropLock(!locked)}
      >
        {locked ? 'Locked' : 'Unlocked'}
      </button>
      <button
        type="button"
        disabled={!usable}
        aria-label="Swap orientation"
        title="Swap portrait and landscape (X)"
        onclick={() => develop.swapCropOrientation()}
      >
        Swap
      </button>
    </div>

    <div class="row">
      <label for="{uid}-overlay">Overlay</label>
      <select id="{uid}-overlay" value={develop.cropOverlay} onchange={onOverlay} title="Next overlay (O)">
        {#each CROP_OVERLAYS as kind (kind)}
          <option value={kind}>{CROP_OVERLAY_LABELS[kind]}</option>
        {/each}
      </select>
    </div>

    <div class="actions">
      <button type="button" disabled={!usable} onclick={() => develop.resetCrop()}>Reset crop</button>
    </div>
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
  .row,
  .actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2) var(--space-3);
    padding: var(--space-2) var(--space-4);
    font-size: var(--font-size-sm);
  }
  .actions {
    padding: var(--space-3) var(--space-4);
  }
  label {
    color: var(--color-text);
    min-width: 7ch;
  }
  select {
    flex: 1;
    min-width: 0;
    padding: var(--space-1) var(--space-2);
    background: var(--color-bg-panel-raised);
    color: var(--color-text);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
    font-size: var(--font-size-sm);
  }
  button {
    background: var(--color-bg-panel-raised);
    color: var(--color-text);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
    padding: var(--space-1) var(--space-3);
    font-size: var(--font-size-sm);
  }
  button[aria-pressed='true'],
  .primary {
    background: var(--color-accent);
    color: var(--color-text-on-accent);
  }
  button:disabled,
  select:disabled {
    opacity: 0.5;
  }
</style>
