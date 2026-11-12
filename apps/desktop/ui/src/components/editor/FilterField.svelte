<!--
  One value of a filter dialog: a label, a slider, and a number field.

  With `log` the slider runs on a logarithmic scale between `min` and `max`
  (for a radius). The number field always shows the real value. The parent
  clamps and stores the value in `onchange`.
-->
<script lang="ts">
  import { radiusToSlider, sliderToRadius } from '../../lib/editor/filter';

  interface Props {
    label: string;
    value: number;
    min: number;
    max: number;
    step?: number;
    decimals?: number;
    unit?: string;
    /** Logarithmic slider (only for the radius limits of `filter.ts`). */
    log?: boolean;
    onchange: (value: number) => void;
  }

  let { label, value, min, max, step = 1, decimals = 0, unit = '', log = false, onchange }: Props = $props();

  const uid = $props.id();
  const SLIDER_STEPS = 1000;
  const shown = $derived(value.toFixed(decimals));

  function fromRange(event: Event & { currentTarget: HTMLInputElement }): void {
    const raw = event.currentTarget.valueAsNumber;
    onchange(log ? sliderToRadius(raw / SLIDER_STEPS) : raw);
  }

  function fromNumber(event: Event & { currentTarget: HTMLInputElement }): void {
    const input = event.currentTarget;
    if (Number.isNaN(input.valueAsNumber)) {
      input.value = shown;
      return;
    }
    onchange(input.valueAsNumber);
    // The parent can clamp the value: show what it kept.
    input.value = shown;
  }
</script>

<div class="row">
  <label for="{uid}-range">{label}</label>
  <input
    id="{uid}-range"
    class="range"
    type="range"
    min={log ? 0 : min}
    max={log ? SLIDER_STEPS : max}
    step={log ? 1 : step}
    value={log ? Math.round(radiusToSlider(value) * SLIDER_STEPS) : value}
    aria-label={label}
    oninput={fromRange}
  />
  <span class="number">
    <input type="number" {min} {max} {step} value={shown} aria-label="{label} value" onchange={fromNumber} />
    {#if unit !== ''}<span class="unit">{unit}</span>{/if}
  </span>
</div>

<style>
  .row {
    display: grid;
    grid-template-columns: 1fr auto;
    grid-template-areas:
      'label number'
      'range range';
    align-items: center;
    gap: var(--space-2) var(--space-3);
    padding: var(--space-2) 0;
    font-size: var(--font-size-sm);
  }
  label {
    grid-area: label;
  }
  .range {
    grid-area: range;
    width: 100%;
    accent-color: var(--color-accent);
  }
  .number {
    grid-area: number;
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
  .number input {
    width: 7ch;
    padding: var(--space-1) var(--space-2);
    background: var(--color-bg-panel-raised);
    color: var(--color-text);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
    font-size: var(--font-size-sm);
    text-align: right;
  }
  .unit {
    color: var(--color-text-muted);
  }
</style>
