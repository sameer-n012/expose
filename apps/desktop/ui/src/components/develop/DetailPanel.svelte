<!--
  The Detail panel of the Develop module: sharpening sliders.

  Radius, detail, and masking have no effect when the amount is 0. Their
  rows are dimmed then, but they stay usable.
-->
<script lang="ts">
  import { SHARPEN_KEYS, SHARPEN_RANGES, sharpeningOf, type SharpenKey } from '../../lib/develop/settings';
  import type { DevelopStore } from '../../lib/stores/develop.svelte';
  import SliderRow from './SliderRow.svelte';

  interface Props {
    develop: DevelopStore;
  }

  let { develop }: Props = $props();

  const settings = $derived(develop.settings);
  const values = $derived(settings === null ? null : sharpeningOf(settings));
  const idle = $derived(values === null || values.amount === 0);

  const LABELS: Readonly<Record<SharpenKey, string>> = {
    amount: 'Amount',
    radius: 'Radius',
    detail: 'Detail',
    masking: 'Masking',
  };
</script>

<details class="section" open>
  <summary>Detail</summary>
  <div class="body" role="group" aria-label="Sharpening">
    {#each SHARPEN_KEYS as key (key)}
      <SliderRow
        label={LABELS[key]}
        value={values?.[key] ?? 0}
        {...SHARPEN_RANGES[key]}
        decimals={key === 'radius' ? 1 : 0}
        disabled={settings === null}
        dimmed={key !== 'amount' && idle}
        onchange={(v) => develop.setSharpening(key, v)}
        onreset={() => develop.resetSharpenKey(key)}
      />
    {/each}
    <div class="actions">
      <button type="button" disabled={settings === null} onclick={() => develop.resetSharpening()}>Reset</button>
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
