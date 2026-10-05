<!--
  The Basic panel of the Develop module: white balance and tone sliders.

  The sliders edit the settings of the active photo in the develop store.
  Without settings (no photo, or the settings still load) the controls are
  disabled.
-->
<script lang="ts">
  import { modeName, RANGES, sliderValue, TONE_KEYS, type SliderKey } from '../../lib/develop/settings';
  import type { DevelopStore } from '../../lib/stores/develop.svelte';
  import SliderRow from './SliderRow.svelte';

  interface Props {
    develop: DevelopStore;
  }

  let { develop }: Props = $props();

  const settings = $derived(develop.settings);
  const mode = $derived(settings === null ? 'as_shot' : modeName(settings));
  const TONE_LABELS: Readonly<Record<(typeof TONE_KEYS)[number], string>> = {
    contrast: 'Contrast',
    highlights: 'Highlights',
    shadows: 'Shadows',
    whites: 'Whites',
    blacks: 'Blacks',
  };

  function value(key: SliderKey): number {
    return settings === null ? 0 : sliderValue(settings, key);
  }

  function onmode(event: Event & { currentTarget: HTMLSelectElement }): void {
    const next = event.currentTarget.value;
    if (next === 'as_shot' || next === 'custom') {
      develop.setWhiteBalanceMode(next);
    }
  }
</script>

<details class="section" open>
  <summary>Basic</summary>
  <div class="body" role="group" aria-label="Basic develop settings">
    <div class="mode">
      <label for="wb-mode">White balance</label>
      <select id="wb-mode" value={mode} disabled={settings === null} onchange={onmode}>
        <option value="as_shot">As shot</option>
        <option value="custom">Custom</option>
        {#if mode === 'preset'}<option value="preset" disabled>Preset</option>{/if}
      </select>
    </div>
    <SliderRow
      label="Temperature"
      value={value('temperature')}
      {...RANGES.temperature}
      unit="K"
      disabled={settings === null || mode === 'as_shot'}
      onchange={(v) => develop.setSlider('temperature', v)}
      onreset={() => develop.resetSlider('temperature')}
    />
    <SliderRow
      label="Tint"
      value={value('tint')}
      {...RANGES.tint}
      disabled={settings === null || mode === 'as_shot'}
      onchange={(v) => develop.setSlider('tint', v)}
      onreset={() => develop.resetSlider('tint')}
    />
    <SliderRow
      label="Exposure"
      value={value('exposure')}
      {...RANGES.exposure}
      unit="EV"
      decimals={2}
      disabled={settings === null}
      onchange={(v) => develop.setSlider('exposure', v)}
      onreset={() => develop.resetSlider('exposure')}
    />
    {#each TONE_KEYS as key (key)}
      <SliderRow
        label={TONE_LABELS[key]}
        value={value(key)}
        {...RANGES.tone}
        disabled={settings === null}
        onchange={(v) => develop.setSlider(key, v)}
        onreset={() => develop.resetSlider(key)}
      />
    {/each}
    <div class="actions">
      <button type="button" disabled={settings === null} onclick={() => develop.resetBasic()}>Reset</button>
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
  .mode {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--space-2) var(--space-4);
    font-size: var(--font-size-sm);
  }
  select,
  button {
    background: var(--color-bg-panel-raised);
    color: var(--color-text);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
    padding: var(--space-1) var(--space-3);
    font-size: var(--font-size-sm);
  }
  .actions {
    padding: var(--space-3) var(--space-4);
  }
  button:disabled {
    opacity: 0.5;
  }
</style>
