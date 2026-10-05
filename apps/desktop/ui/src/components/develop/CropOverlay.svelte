<!--
  The crop frame over the preview image in crop mode.

  The frame is axis-aligned in the W x H frame of the image (the parent
  places this element on the image rect). The area outside the crop rect
  is darkened. Eight handles resize the rect; a drag inside moves it.
  Each move starts again from the rect at pointer down, so the clamping
  (see `cropGeometry.ts`) does not add up errors. Guide lines show inside
  the rect; a fine grid shows while the angle changes.
-->
<script lang="ts">
  import { lockedRatio } from '../../lib/develop/crop';
  import {
    CROP_HANDLES,
    moveRect,
    resizeRect,
    type CropHandle,
    type NormalizedRect,
  } from '../../lib/develop/cropGeometry';
  import { applyCropKey, CROP_KEYS_HELP } from '../../lib/develop/cropKeys';
  import { overlayLines, toNormalizedDelta, type ScreenRect } from '../../lib/develop/cropView';
  import type { DevelopStore } from '../../lib/stores/develop.svelte';

  interface Props {
    develop: DevelopStore;
    /** The image rect in CSS pixels (its size maps pointer moves to the frame). */
    content: ScreenRect;
  }

  let { develop, content }: Props = $props();

  const crop = $derived(develop.crop);
  const rect = $derived(crop?.rect ?? null);
  const lines = $derived(overlayLines(develop.fineGrid ? 'fine' : develop.cropOverlay));

  const HANDLE_LABELS: Readonly<Record<CropHandle, string>> = {
    nw: 'top left',
    n: 'top',
    ne: 'top right',
    e: 'right',
    se: 'bottom right',
    s: 'bottom',
    sw: 'bottom left',
    w: 'left',
  };

  interface Drag {
    readonly kind: CropHandle | 'move';
    readonly pointerId: number;
    readonly startX: number;
    readonly startY: number;
    readonly startRect: NormalizedRect;
  }

  let drag: Drag | null = null;
  let dragging = $state(false);

  function start(event: PointerEvent, kind: CropHandle | 'move'): void {
    if (rect === null || event.button !== 0) {
      return;
    }
    event.preventDefault();
    event.stopPropagation();
    const target = event.currentTarget;
    if (target instanceof Element && typeof target.setPointerCapture === 'function') {
      target.setPointerCapture(event.pointerId);
    }
    drag = { kind, pointerId: event.pointerId, startX: event.clientX, startY: event.clientY, startRect: rect };
    dragging = true;
  }

  function move(event: PointerEvent): void {
    const current = drag;
    const settings = crop;
    const size = develop.frameSize;
    if (current === null || event.pointerId !== current.pointerId || settings === null || size === null) {
      return;
    }
    const { dx, dy } = toNormalizedDelta(event.clientX - current.startX, event.clientY - current.startY, content);
    const frame = { width: size.width, height: size.height, angle: settings.angle };
    const next =
      current.kind === 'move'
        ? moveRect(frame, current.startRect, dx, dy)
        : resizeRect(frame, current.startRect, current.kind, dx, dy, lockedRatio(settings, size));
    develop.setCropRect(next);
  }

  /**
   * Arrow keys move or resize the rect (see `cropKeys.ts`). Other keys go
   * on to the shortcut registry (Enter, Escape, O, X).
   */
  function onKey(event: KeyboardEvent): void {
    const settings = crop;
    const size = develop.frameSize;
    if (settings === null || size === null || event.target !== event.currentTarget) {
      return;
    }
    const next = applyCropKey(
      { width: size.width, height: size.height, angle: settings.angle },
      settings.rect,
      event,
      lockedRatio(settings, size),
    );
    if (next !== null) {
      event.preventDefault();
      develop.setCropRect(next);
    }
  }

  function end(event: PointerEvent): void {
    if (drag !== null && event.pointerId === drag.pointerId) {
      drag = null;
      dragging = false;
    }
  }
</script>

<!-- Pointer moves arrive here from the element that has the pointer capture. -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="crop-overlay"
  class:dragging
  data-testid="crop-overlay"
  onpointermove={move}
  onpointerup={end}
  onpointercancel={end}
>
  {#if rect !== null}
    <!-- The frame is a focusable group: arrow keys move or resize it (the label names the keys). -->
    <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
    <div
      class="crop-rect"
      data-testid="crop-rect"
      role="group"
      tabindex="0"
      aria-label={CROP_KEYS_HELP}
      title={CROP_KEYS_HELP}
      onkeydown={onKey}
      style:left="{rect.left * 100}%"
      style:top="{rect.top * 100}%"
      style:width="{rect.width * 100}%"
      style:height="{rect.height * 100}%"
      onpointerdown={(event) => start(event, 'move')}
    >
      <svg class="guides" viewBox="0 0 1 1" preserveAspectRatio="none" aria-hidden="true">
        {#each lines as line, index (index)}
          <line x1={line.x1} y1={line.y1} x2={line.x2} y2={line.y2} />
        {/each}
      </svg>
      {#each CROP_HANDLES as handle (handle)}
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div
          class="handle handle-{handle}"
          data-handle={handle}
          title="Resize the crop: {HANDLE_LABELS[handle]}"
          onpointerdown={(event) => start(event, handle)}
        ></div>
      {/each}
    </div>
  {/if}
</div>

<style>
  .crop-overlay {
    position: absolute;
    inset: 0;
    touch-action: none;
  }

  .crop-rect {
    position: absolute;
    box-sizing: border-box;
    border: 1px solid rgba(255, 255, 255, 0.9);
    /* Darkens all of the preview outside the rect; the preview box clips it. */
    box-shadow: 0 0 0 100vmax rgba(0, 0, 0, 0.55);
    cursor: move;
  }
  .crop-rect:focus-visible {
    outline: 2px solid var(--color-accent, #4c9aff);
    outline-offset: 2px;
  }

  .guides {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    pointer-events: none;
    opacity: 0.6;
  }
  .dragging .guides {
    opacity: 0.85;
  }
  .guides line {
    stroke: rgba(255, 255, 255, 0.8);
    stroke-width: 1;
    vector-effect: non-scaling-stroke;
  }

  .handle {
    position: absolute;
    width: 14px;
    height: 14px;
    margin: -7px 0 0 -7px;
    box-sizing: border-box;
    background: rgba(255, 255, 255, 0.95);
    border: 1px solid rgba(0, 0, 0, 0.6);
  }
  .handle-nw {
    left: 0;
    top: 0;
    cursor: nwse-resize;
  }
  .handle-n {
    left: 50%;
    top: 0;
    cursor: ns-resize;
  }
  .handle-ne {
    left: 100%;
    top: 0;
    cursor: nesw-resize;
  }
  .handle-e {
    left: 100%;
    top: 50%;
    cursor: ew-resize;
  }
  .handle-se {
    left: 100%;
    top: 100%;
    cursor: nwse-resize;
  }
  .handle-s {
    left: 50%;
    top: 100%;
    cursor: ns-resize;
  }
  .handle-sw {
    left: 0;
    top: 100%;
    cursor: nesw-resize;
  }
  .handle-w {
    left: 0;
    top: 50%;
    cursor: ew-resize;
  }
</style>
