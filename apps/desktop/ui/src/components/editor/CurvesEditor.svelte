<!--
  The Curves editor of an adjustment layer: a point curve editor with one
  channel at a time (master, red, green, blue).

  A click on empty space adds a point (one edit). A drag moves a point (one
  undo gesture). A double-click on a point, or a drag far outside the box,
  removes it. A focused point moves with the arrow keys (Shift: big step) and
  is removed with Delete or Backspace.
-->
<script lang="ts">
  import {
    addPoint,
    CURVE_CHANNELS,
    isFarOutside,
    isIdentity,
    movePoint,
    removePoint,
    sampleCurve,
    withCurve,
    type CurveChannel,
    type CurvePoint,
    type CurvesAdjustment,
  } from '../../lib/editor/curves';
  import type { EditorStore } from '../../lib/stores/editor.svelte';

  interface Props {
    editor: EditorStore;
    layerId: number;
    adjustment: CurvesAdjustment;
  }

  let { editor, layerId, adjustment }: Props = $props();

  /** The size of the plot in view box units, and the space around it. */
  const SIZE = 240;
  const PAD = 8;
  const STEP = 0.01;
  const BIG_STEP = 0.1;
  const LABELS: Readonly<Record<CurveChannel, string>> = {
    master: 'Master',
    red: 'Red',
    green: 'Green',
    blue: 'Blue',
  };

  let channel = $state<CurveChannel>('master');
  let svg = $state<SVGSVGElement>();
  /** The index of the point under a drag, or null. */
  let dragging = $state<number | null>(null);
  /** The edited adjustment. It stays until all edits are done. */
  let draft = $state<CurvesAdjustment | null>(null);

  $effect(() => {
    if (editor.idle) {
      draft = null;
    }
  });

  /** True if a lock of the layer or of a parent group blocks changes to its properties. */
  const locked = $derived(editor.selectedLocks.all);

  const current = $derived(draft ?? adjustment);
  const points = $derived<readonly CurvePoint[]>(current[channel]);
  const path = $derived(
    sampleCurve(points, 65)
      .map((p, i) => `${i === 0 ? 'M' : 'L'}${px(p.x).toFixed(2)} ${py(p.y).toFixed(2)}`)
      .join(' '),
  );

  function px(x: number): number {
    return x * SIZE;
  }
  function py(y: number): number {
    return (1 - y) * SIZE;
  }

  /** Sends a discrete edit (add, remove, reset): its own undo step. */
  function commit(next: readonly CurvePoint[]): void {
    const adj = withCurve(current, channel, next);
    draft = adj;
    editor.edit({ type: 'setAdjustment', layerId, adjustment: adj });
  }

  /** Sends a live edit (drag, arrow keys): one undo step for the gesture. */
  function live(next: readonly CurvePoint[]): void {
    const adj = withCurve(current, channel, next);
    draft = adj;
    editor.liveEdit(`adjustment:${layerId}`, { type: 'setAdjustment', layerId, adjustment: adj });
  }

  function locate(event: PointerEvent): { x: number; y: number } | null {
    if (svg === undefined) {
      return null;
    }
    const rect = svg.getBoundingClientRect();
    if (rect.width === 0 || rect.height === 0) {
      return null;
    }
    const scale = (SIZE + 2 * PAD) / rect.width;
    const ux = (event.clientX - rect.left) * scale - PAD;
    const uy = (event.clientY - rect.top) * scale - PAD;
    return { x: ux / SIZE, y: 1 - uy / SIZE };
  }

  function capture(event: PointerEvent): void {
    try {
      svg?.setPointerCapture?.(event.pointerId);
    } catch {
      // The pointer is already gone. The drag then ends with the next event.
    }
  }

  function onplotdown(event: PointerEvent): void {
    if (locked || event.button !== 0) {
      return;
    }
    const at = locate(event);
    if (at === null) {
      return;
    }
    const added = addPoint(points, at.x, at.y);
    if (added === null) {
      return;
    }
    commit(added.points);
    dragging = added.index;
    capture(event);
  }

  function onpointdown(event: PointerEvent, index: number): void {
    // A point must not start the "add" of the plot.
    event.stopPropagation();
    if (locked || event.button !== 0) {
      return;
    }
    editor.endGesture();
    dragging = index;
    capture(event);
  }

  function onmove(event: PointerEvent): void {
    if (dragging === null) {
      return;
    }
    const at = locate(event);
    if (at !== null) {
      live(movePoint(points, dragging, at.x, at.y));
    }
  }

  function onup(event: PointerEvent): void {
    const index = dragging;
    dragging = null;
    if (index === null) {
      return;
    }
    editor.endGesture();
    const at = locate(event);
    if (at !== null && isFarOutside(at.x, at.y)) {
      commit(removePoint(points, index));
    }
  }

  function onkey(event: KeyboardEvent, index: number): void {
    const step = event.shiftKey ? BIG_STEP : STEP;
    const point = points[index];
    if (locked || point === undefined) {
      return;
    }
    let dx = 0;
    let dy = 0;
    switch (event.key) {
      case 'ArrowLeft':
        dx = -step;
        break;
      case 'ArrowRight':
        dx = step;
        break;
      case 'ArrowUp':
        dy = step;
        break;
      case 'ArrowDown':
        dy = -step;
        break;
      case 'Delete':
      case 'Backspace':
        event.preventDefault();
        commit(removePoint(points, index));
        return;
      default:
        return;
    }
    event.preventDefault();
    live(movePoint(points, index, point.x + dx, point.y + dy));
  }

  function label(point: CurvePoint): string {
    return `${LABELS[channel]} curve point, input ${point.x.toFixed(2)}, output ${point.y.toFixed(2)}`;
  }
</script>

<div class="curves" role="group" aria-label="Curves">
  <div class="channels" role="group" aria-label="Curve channel">
    {#each CURVE_CHANNELS as name (name)}
      <button type="button" class="channel {name}" aria-pressed={channel === name} onclick={() => (channel = name)}
        >{LABELS[name]}</button
      >
    {/each}
  </div>
  <!-- The plot handles pointer input; the points handle the keyboard. -->
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <svg
    bind:this={svg}
    class="plot"
    viewBox="{-PAD} {-PAD} {SIZE + 2 * PAD} {SIZE + 2 * PAD}"
    role="group"
    aria-label="{LABELS[channel]} curve editor"
    data-testid="curve-plot"
    onpointerdown={onplotdown}
    onpointermove={onmove}
    onpointerup={onup}
    onpointercancel={onup}
  >
    <rect class="box" x="0" y="0" width={SIZE} height={SIZE} />
    {#each [1, 2, 3] as q (q)}
      <line class="grid" x1={(q * SIZE) / 4} y1="0" x2={(q * SIZE) / 4} y2={SIZE} />
      <line class="grid" x1="0" y1={(q * SIZE) / 4} x2={SIZE} y2={(q * SIZE) / 4} />
    {/each}
    <line class="diagonal" x1="0" y1={SIZE} x2={SIZE} y2="0" />
    <path class="curve {channel}" d={path} data-testid="curve-path" />
    {#each points as point, index (index)}
      <circle
        class="point"
        class:active={dragging === index}
        cx={px(point.x)}
        cy={py(point.y)}
        r="5"
        role="slider"
        tabindex="0"
        aria-label={label(point)}
        aria-valuemin="0"
        aria-valuemax="1"
        aria-valuenow={point.y}
        aria-valuetext="input {point.x.toFixed(2)}, output {point.y.toFixed(2)}"
        data-testid="curve-point"
        onpointerdown={(event) => onpointdown(event, index)}
        aria-disabled={locked}
        ondblclick={() => !locked && commit(removePoint(points, index))}
        onkeydown={(event) => onkey(event, index)}
        onkeyup={() => editor.endGesture()}
      />
    {/each}
  </svg>
  <div class="actions">
    <button type="button" disabled={locked || isIdentity(points)} onclick={() => commit([{ x: 0, y: 0 }, { x: 1, y: 1 }])}
      >Reset {LABELS[channel].toLowerCase()}</button
    >
  </div>
</div>

<style>
  .channels {
    display: flex;
    gap: var(--space-1);
    padding: var(--space-2) var(--space-4);
  }
  button {
    background: var(--color-bg-panel-raised);
    color: var(--color-text);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
    padding: var(--space-1) var(--space-3);
    font-size: var(--font-size-sm);
  }
  .channel {
    flex: 1;
  }
  .channel[aria-pressed='true'] {
    border-color: var(--color-accent);
  }
  button:disabled {
    opacity: 0.5;
  }
  .plot {
    display: block;
    width: calc(100% - 2 * var(--space-4));
    margin: var(--space-2) var(--space-4);
    aspect-ratio: 1;
    touch-action: none;
    user-select: none;
    cursor: crosshair;
  }
  .box {
    fill: var(--color-bg-panel-raised);
    stroke: var(--color-border);
  }
  .grid {
    stroke: var(--color-border);
    stroke-width: 0.5;
  }
  .diagonal {
    stroke: var(--color-text-muted);
    stroke-width: 0.5;
    stroke-dasharray: 3 3;
  }
  .curve {
    fill: none;
    stroke: var(--color-text);
    stroke-width: 1.5;
  }
  .curve.red {
    stroke: #e5484d;
  }
  .curve.green {
    stroke: #46a758;
  }
  .curve.blue {
    stroke: #3e63dd;
  }
  .point {
    fill: var(--color-bg-panel);
    stroke: var(--color-text);
    stroke-width: 1.5;
    cursor: grab;
  }
  .point:focus-visible,
  .point.active {
    stroke: var(--color-accent);
    outline: none;
  }
  .actions {
    padding: var(--space-3) var(--space-4);
  }
</style>
