<!--
  The gradient editor (modal). A bar shows the gradient and the stop markers
  are under it.

  - A click on the bar adds a stop with the interpolated color and opacity.
  - A drag of a marker moves the stop. The list stays sorted. A drag far
    below the bar, or Delete or Backspace, removes the selected stop. At
    least 2 stops stay. At most 32 stops are possible.
  - The selected stop has a color, an opacity (0 to 100 %), and a position
    (0 to 100 %).
  - OK applies the stops to the settings and sets the preset to Custom.
    Cancel and Esc close the dialog and discard the changes.

  Keyboard: Tab and Shift+Tab stay inside the dialog. Key events do not
  leave the dialog, so shortcuts of the shell do not run behind it.
-->
<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import { clampUnit, hexToRgb, rgbToHex } from '../../lib/editor/brush';
  import {
    MAX_GRADIENT_STOPS,
    MIN_GRADIENT_STOPS,
    addStop,
    cssGradient,
    moveStop,
    presetStops,
    removeStop,
    updateStop,
    type GradientStop,
  } from '../../lib/editor/gradient';
  import type { EditorStore } from '../../lib/stores/editor.svelte';

  interface Props {
    editor: EditorStore;
  }

  /** A drag farther than this (CSS pixels) below the bar removes the stop. */
  const REMOVE_DISTANCE = 40;

  let { editor }: Props = $props();

  let dialog: HTMLDivElement | undefined = $state();
  let track: HTMLDivElement | undefined = $state();
  // The dialog works on a copy of the stops that show now.
  let stops = $state.raw<GradientStop[]>(
    untrack(() => presetStops(editor.gradient, hexToRgb(editor.foreground) ?? [0, 0, 0], hexToRgb(editor.background) ?? [1, 1, 1])),
  );
  let selected = $state(0);
  let dragging = $state.raw<{ pointerId: number } | null>(null);
  let removing = $state(false);

  const current = $derived(stops[selected]);
  const bar = $derived(cssGradient(stops));

  onMount(() => {
    const previous = document.activeElement;
    dialog?.focus();
    return () => {
      if (previous instanceof HTMLElement) {
        previous.focus();
      }
    };
  });

  /** The position (0..1) of a pointer event along the bar. */
  function positionOf(event: PointerEvent): number {
    const rect = track?.getBoundingClientRect();
    if (rect === undefined || rect.width <= 0) {
      return 0;
    }
    return Math.min(1, Math.max(0, (event.clientX - rect.left) / rect.width));
  }

  function markerIndex(target: EventTarget | null): number | null {
    const marker = target instanceof Element ? target.closest<HTMLElement>('[data-index]') : null;
    const index = marker === null ? Number.NaN : Number(marker.dataset.index);
    return Number.isInteger(index) ? index : null;
  }

  function onpointerdown(event: PointerEvent): void {
    if (event.button !== 0) {
      return;
    }
    const index = markerIndex(event.target);
    if (index !== null) {
      selected = index;
    } else {
      const added = addStop(stops, positionOf(event));
      if (added === null) {
        return;
      }
      stops = added.stops;
      selected = added.index;
    }
    dragging = { pointerId: event.pointerId };
    track?.setPointerCapture?.(event.pointerId);
  }

  function onpointermove(event: PointerEvent): void {
    if (dragging === null || event.pointerId !== dragging.pointerId) {
      return;
    }
    const rect = track?.getBoundingClientRect();
    removing = rect !== undefined && event.clientY > rect.bottom + REMOVE_DISTANCE && stops.length > MIN_GRADIENT_STOPS;
    if (removing) {
      return;
    }
    const moved = moveStop(stops, selected, positionOf(event));
    if (moved !== null) {
      stops = moved.stops;
      selected = moved.index;
    }
  }

  function onpointerup(event: PointerEvent): void {
    if (dragging === null || event.pointerId !== dragging.pointerId) {
      return;
    }
    dragging = null;
    track?.releasePointerCapture?.(event.pointerId);
    if (removing && event.type !== 'pointercancel') {
      removeSelected();
    }
    removing = false;
  }

  function removeSelected(): void {
    const next = removeStop(stops, selected);
    if (next !== null) {
      stops = next;
      selected = Math.min(selected, next.length - 1);
    }
  }

  function setColor(hex: string): void {
    const color = hexToRgb(hex);
    const next = color === null ? null : updateStop(stops, selected, { color });
    if (next !== null) {
      stops = next;
    }
  }

  function setOpacity(percent: number): void {
    if (!Number.isFinite(percent)) {
      return;
    }
    const next = updateStop(stops, selected, { opacity: clampUnit(percent / 100) });
    if (next !== null) {
      stops = next;
    }
  }

  function setPosition(percent: number): void {
    if (!Number.isFinite(percent)) {
      return;
    }
    const moved = moveStop(stops, selected, percent / 100);
    if (moved !== null) {
      stops = moved.stops;
      selected = moved.index;
    }
  }

  function accept(): void {
    editor.gradient.stops = stops;
    editor.gradient.preset = 'custom';
    editor.closeGradientDialog();
  }

  function onkeydown(event: KeyboardEvent): void {
    // Shell shortcuts must not run while the dialog is open.
    event.stopPropagation();
    if (event.key === 'Escape') {
      event.preventDefault();
      editor.closeGradientDialog();
      return;
    }
    const inField = event.target instanceof HTMLInputElement;
    if ((event.key === 'Delete' || event.key === 'Backspace') && !inField) {
      event.preventDefault();
      removeSelected();
      return;
    }
    if (event.key !== 'Tab' || dialog === undefined) {
      return;
    }
    const items = [...dialog.querySelectorAll<HTMLElement>('button:not(:disabled), input')];
    const first = items[0];
    const last = items[items.length - 1];
    if (first === undefined || last === undefined) {
      return;
    }
    const active = document.activeElement;
    if (event.shiftKey && (active === first || !dialog.contains(active))) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && (active === last || !dialog.contains(active))) {
      event.preventDefault();
      first.focus();
    }
  }
</script>

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div class="backdrop" onclick={(e) => e.target === e.currentTarget && editor.closeGradientDialog()}>
  <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="gradient-title" tabindex="-1" bind:this={dialog} {onkeydown}>
    <h2 id="gradient-title">Gradient editor</h2>
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div
      class="track"
      bind:this={track}
      {onpointerdown}
      {onpointermove}
      {onpointerup}
      onpointercancel={onpointerup}
      data-testid="gradient-track"
    >
      <div class="bar" style:background-image={bar} data-testid="gradient-bar"></div>
      <div class="markers">
        {#each stops as stop, index (index)}
          <button
            type="button"
            class="marker"
            class:selected={index === selected}
            class:removing={removing && index === selected}
            style:left="{stop.position * 100}%"
            style:--stop-color={rgbToHex(stop.color)}
            data-index={index}
            aria-label="Stop {index + 1}"
            aria-pressed={index === selected}
            data-testid="gradient-stop"
          ></button>
        {/each}
      </div>
    </div>
    <p class="hint">Click the bar to add a stop. Drag a stop to move it. Drag it far down or press Delete to remove it.</p>
    {#if current !== undefined}
      <div class="grid">
        <label for="gradient-color">Color</label>
        <input id="gradient-color" type="color" value={rgbToHex(current.color)} oninput={(e) => setColor(e.currentTarget.value)} />
        <label for="gradient-opacity">Opacity (%)</label>
        <input
          id="gradient-opacity"
          type="number"
          min="0"
          max="100"
          step="1"
          value={Math.round(current.opacity * 100)}
          onchange={(e) => setOpacity(Number(e.currentTarget.value))}
        />
        <label for="gradient-position">Position (%)</label>
        <input
          id="gradient-position"
          type="number"
          min="0"
          max="100"
          step="1"
          value={Math.round(current.position * 100)}
          onchange={(e) => setPosition(Number(e.currentTarget.value))}
        />
      </div>
    {/if}
    <p class="count" data-testid="gradient-count">{stops.length} of {MAX_GRADIENT_STOPS} stops</p>
    <div class="actions">
      <button type="button" onclick={() => editor.closeGradientDialog()}>Cancel</button>
      <button type="button" class="primary" onclick={accept}>OK</button>
    </div>
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: var(--z-toast);
    display: flex;
    align-items: center;
    justify-content: center;
    background: rgb(0 0 0 / 55%);
  }
  .dialog {
    width: min(420px, 92vw);
    padding: var(--space-5);
    background: var(--color-bg-panel);
    border: 1px solid var(--color-border-strong);
    border-radius: var(--radius-md);
    font-size: var(--font-size-md);
  }
  h2 {
    margin: 0 0 var(--space-3);
    font-size: var(--font-size-xl);
  }
  .track {
    position: relative;
    margin: 0 8px;
    touch-action: none;
    cursor: copy;
  }
  .bar {
    height: 28px;
    border: 1px solid var(--color-border);
    background-color: #fff;
  }
  .markers {
    position: relative;
    height: 24px;
  }
  .marker {
    position: absolute;
    top: 2px;
    width: 14px;
    height: 18px;
    padding: 0;
    transform: translateX(-50%);
    background: var(--stop-color);
    border: 2px solid var(--color-border-strong);
    border-radius: 2px;
    cursor: grab;
  }
  .marker.selected {
    border-color: var(--color-accent);
  }
  .marker.removing {
    opacity: 0.3;
  }
  .hint,
  .count {
    margin: var(--space-2) 0;
    font-size: var(--font-size-sm);
    color: var(--color-text-muted);
  }
  .grid {
    display: grid;
    grid-template-columns: max-content 1fr;
    align-items: center;
    gap: var(--space-3);
  }
  input,
  .actions button {
    background: var(--color-bg-panel-raised);
    color: var(--color-text);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
    padding: var(--space-1) var(--space-3);
    font-size: var(--font-size-sm);
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-3);
  }
  .actions .primary {
    background: var(--color-accent);
    color: var(--color-text-on-accent);
  }
</style>
