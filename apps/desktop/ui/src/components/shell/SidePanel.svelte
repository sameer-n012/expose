<!--
  A side panel with a resize handle on its inner edge.

  The panel stays mounted when hidden (the `hidden` attribute), so that the
  panel content keeps its state. Drag the handle to resize. When the handle
  has focus, the arrow keys resize in steps and Enter resets the width.
  A double-click on the handle also resets the width.
-->
<script lang="ts">
  import type { Snippet } from 'svelte';
  import { PANEL_LIMITS, RESIZE_STEP, type Side } from '../../lib/stores/layout.svelte';

  interface Props {
    side: Side;
    label: string;
    visible: boolean;
    width: number;
    onresize: (width: number) => void;
    onresetwidth: () => void;
    children: Snippet;
  }

  let { side, label, visible, width, onresize, onresetwidth, children }: Props = $props();

  /** Drag state. Null when no drag is in progress. */
  let drag: { pointerId: number; startX: number; startWidth: number } | null = null;
  let dragging = $state(false);

  /** +1 if a pointer move to the right makes the panel wider, else -1. */
  const direction = $derived(side === 'left' ? 1 : -1);

  function onpointerdown(event: PointerEvent): void {
    if (event.button !== 0) {
      return;
    }
    const handle = event.currentTarget as HTMLElement;
    handle.setPointerCapture(event.pointerId);
    drag = { pointerId: event.pointerId, startX: event.clientX, startWidth: width };
    dragging = true;
    event.preventDefault();
  }

  function onpointermove(event: PointerEvent): void {
    if (drag === null || event.pointerId !== drag.pointerId) {
      return;
    }
    onresize(drag.startWidth + (event.clientX - drag.startX) * direction);
  }

  function endDrag(event: PointerEvent): void {
    if (drag === null || event.pointerId !== drag.pointerId) {
      return;
    }
    const handle = event.currentTarget as HTMLElement;
    if (handle.hasPointerCapture(event.pointerId)) {
      handle.releasePointerCapture(event.pointerId);
    }
    drag = null;
    dragging = false;
  }

  function onkeydown(event: KeyboardEvent): void {
    let next: number | null = null;
    if (event.key === 'ArrowRight') {
      next = width + RESIZE_STEP * direction;
    } else if (event.key === 'ArrowLeft') {
      next = width - RESIZE_STEP * direction;
    } else if (event.key === 'Home') {
      next = PANEL_LIMITS.min;
    } else if (event.key === 'End') {
      next = PANEL_LIMITS.max;
    } else if (event.key === 'Enter') {
      onresetwidth();
    } else {
      return;
    }
    if (next !== null) {
      onresize(next);
    }
    // The key was used for the resize; the global shortcut handler must not use it too.
    event.preventDefault();
    event.stopPropagation();
  }
</script>

<aside
  class="panel {side}"
  class:dragging
  aria-label={label}
  hidden={!visible}
  style:width="{width}px"
  data-testid="panel-{side}"
>
  <div class="content">
    {@render children()}
  </div>
  <!-- A focusable separator with key handlers is the ARIA "window splitter"
       pattern. The linter does not know that this separator is interactive. -->
  <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
  <div
    class="handle"
    role="separator"
    aria-orientation="vertical"
    aria-label="Resize {label.toLowerCase()}"
    aria-valuenow={width}
    aria-valuemin={PANEL_LIMITS.min}
    aria-valuemax={PANEL_LIMITS.max}
    tabindex="0"
    {onpointerdown}
    {onpointermove}
    onpointerup={endDrag}
    onpointercancel={endDrag}
    ondblclick={onresetwidth}
    {onkeydown}
  ></div>
</aside>

<style>
  .panel {
    position: relative;
    flex: none;
    min-width: var(--panel-min-width);
    max-width: var(--panel-max-width);
    background: var(--color-bg-panel);
    display: flex;
    flex-direction: column;
  }
  .left {
    border-right: 1px solid var(--color-border);
  }
  .right {
    border-left: 1px solid var(--color-border);
  }
  .content {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }
  .handle {
    position: absolute;
    top: 0;
    bottom: 0;
    width: var(--resize-handle-size);
    cursor: col-resize;
    touch-action: none;
    z-index: 1;
  }
  .left .handle {
    right: calc(var(--resize-handle-size) / -2);
  }
  .right .handle {
    left: calc(var(--resize-handle-size) / -2);
  }
  .handle:hover,
  .handle:focus-visible,
  .dragging .handle {
    background: var(--color-border-strong);
  }
</style>
