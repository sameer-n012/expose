<!--
  One develop slider: a label, a range input, and a number input.

  A double-click on the label or on the range resets the slider (the
  `onreset` callback). The range input sends each move at once (`oninput`).
  The number input sends its value when the user commits it (Enter or
  focus change).
-->
<script lang="ts">
  interface Props {
    label: string;
    value: number;
    min: number;
    max: number;
    step: number;
    /** The text after the number, for example "EV". */
    unit?: string;
    /** The number of decimals that the number input shows. */
    decimals?: number;
    disabled?: boolean;
    onchange: (value: number) => void;
    onreset: () => void;
  }

  let { label, value, min, max, step, unit = '', decimals = 0, disabled = false, onchange, onreset }: Props = $props();

  const uid = $props.id();
  const shown = $derived(value.toFixed(decimals));

  function fromRange(event: Event & { currentTarget: HTMLInputElement }): void {
    onchange(event.currentTarget.valueAsNumber);
  }

  function fromNumber(event: Event & { currentTarget: HTMLInputElement }): void {
    const input = event.currentTarget;
    if (Number.isNaN(input.valueAsNumber)) {
      // Not a number: show the last good value again.
      input.value = shown;
      return;
    }
    onchange(input.valueAsNumber);
    // The store can clamp the value. Show the value that it keeps.
    input.value = shown;
  }
</script>

<div class="row" class:disabled>
  <!-- A double-click on the label resets the slider. -->
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <label for="{uid}-range" ondblclick={disabled ? undefined : onreset}>{label}</label>
  <input
    id="{uid}-range"
    class="range"
    type="range"
    {min}
    {max}
    {step}
    {value}
    {disabled}
    aria-label={label}
    ondblclick={onreset}
    oninput={fromRange}
  />
  <span class="number">
    <input
      type="number"
      {min}
      {max}
      {step}
      value={shown}
      {disabled}
      aria-label="{label} value"
      onchange={fromNumber}
    />
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
    padding: var(--space-2) var(--space-4);
    font-size: var(--font-size-sm);
  }
  .disabled {
    opacity: 0.5;
  }
  label {
    grid-area: label;
    color: var(--color-text);
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
    width: 6ch;
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
