<!--
  Center view of the Editor module: the document canvas.

  - The view opens the library photo when the module is active and no
    document is open (`editor.openTarget`).
  - A checkerboard shows behind the document. The shell renders the visible
    part of the document at device pixel size, with straight alpha.
  - Each view change or document change requests a new frame (at most 2 in
    flight, latest wins). Until the new frame arrives, the old frame shows
    scaled and moved to the new view.
  - Wheel: scroll. With the primary modifier (and for pinch): zoom about the
    pointer. Middle button, or the space key and the left button: pan.
  - Tools (`editor.tool`): hand (the left button does nothing), rectangle and
    ellipse marquee (drag), polygon lasso (click points). The modifier keys at
    the first pointer-down set the selection mode. The outline of the
    selection and the preview of a gesture are SVG layers over the canvas.
-->
<script lang="ts">
  import { onDestroy, untrack } from 'svelte';
  import { fitCanvas, frameImageData } from '../../lib/develop/draw';
  import type { RenderArea } from '../../lib/editor/api';
  import { EditorFrameClient, type EditorFrame } from '../../lib/editor/frameClient';
  import {
    boxBetween,
    boxPath,
    closesPolygon,
    dragShape,
    ellipsePath,
    isClick,
    modeFromModifiers,
    polygonShape,
    polylinePath,
    repeatsLastPoint,
  } from '../../lib/editor/selection';
  import {
    canvasToScreen,
    frameBox,
    hasArea,
    renderArea,
    screenToCanvas,
    wheelZoomFactor,
    type Point,
    type View,
  } from '../../lib/editor/view';
  import { isEditableTarget } from '../../lib/shortcuts/editable';
  import { editorApi, type SelectionModeId } from '../../lib/editor/api';
  import type { EditorStore } from '../../lib/stores/editor.svelte';
  import FeatherDialog from './FeatherDialog.svelte';
  import NewDocumentDialog from './NewDocumentDialog.svelte';
  import SelectionOverlay from './SelectionOverlay.svelte';
  import SelectionToolbar from './SelectionToolbar.svelte';
  import { describeError, type ToastStore } from '../../lib/stores/toasts.svelte';

  interface Props {
    editor: EditorStore;
    toasts: ToastStore;
  }

  let { editor, toasts }: Props = $props();

  let canvas: HTMLCanvasElement | undefined = $state();
  let width = $state(0);
  let height = $state(0);
  /** The area of the frame on the canvas, or null. */
  let shown = $state.raw<RenderArea | null>(null);
  let spaceDown = $state(false);
  let panning = $state(false);
  let reportedError = false;

  let context: CanvasRenderingContext2D | null = null;
  let waiting: EditorFrame | null = null;
  let drawScheduled = false;
  let disposed = false;

  const space = untrack(() => (editor.displaySpace === 'displayP3' ? 'displayP3' : 'srgb'));

  function getContext(): CanvasRenderingContext2D | null {
    if (canvas === undefined) {
      return null;
    }
    // Alpha stays on: transparent pixels show the checkerboard.
    context ??= canvas.getContext('2d', { alpha: true, colorSpace: space === 'displayP3' ? 'display-p3' : 'srgb' });
    return context;
  }

  function draw(): void {
    drawScheduled = false;
    const next = waiting;
    waiting = null;
    if (next === null || disposed) {
      return;
    }
    const ctx = getContext();
    if (ctx === null) {
      toasts.error('The web view has no 2D canvas.');
      return;
    }
    try {
      fitCanvas(ctx.canvas, next.frame.width, next.frame.height);
      ctx.putImageData(frameImageData(next.frame, space), 0, 0);
      shown = next.area;
    } catch (error: unknown) {
      toasts.error('Draw the Editor frame failed.', error);
    }
  }

  function onFrame(frame: EditorFrame): void {
    reportedError = false;
    waiting = frame;
    if (!drawScheduled) {
      drawScheduled = true;
      if (typeof requestAnimationFrame === 'function') {
        requestAnimationFrame(draw);
      } else {
        setTimeout(draw, 0);
      }
    }
  }

  const client = new EditorFrameClient({
    render: (documentId, area) => editorApi.render(documentId, area),
    // The store drops the old document before it closes it in the shell.
    isCurrent: (documentId) => editor.info?.documentId === documentId,
    onFrame,
    onError: (error) => {
      // One toast per run of failed frames, so a slow drag does not fill the screen.
      if (!reportedError) {
        reportedError = true;
        toasts.error(`Render the document failed: ${describeError(error)}`);
      }
    },
  });

  $effect(() => {
    const id = editor.openTarget;
    if (id !== null) {
      untrack(() => void editor.openPhoto(id));
    }
  });

  // A new document: forget the old frame.
  $effect(() => {
    const id = editor.info?.documentId ?? null;
    untrack(() => {
      client.setDocument(id);
      shown = null;
    });
  });

  // Every document or view change asks for a new frame.
  $effect(() => {
    const info = editor.info;
    const view = editor.view;
    const viewport = editor.viewport;
    if (info === null || view === null || !hasArea(viewport)) {
      return;
    }
    void info.revision;
    const showMaskLayerId = editor.showMaskLayerId;
    const ratio = typeof window === 'undefined' ? 1 : window.devicePixelRatio;
    client.request({ ...renderArea(view, viewport, ratio), showMaskLayerId });
  });

  $effect(() => {
    const size = { width, height };
    untrack(() => editor.setViewport(size));
  });

  onDestroy(() => {
    disposed = true;
    client.dispose();
    void editor.dispose();
  });

  const docBox = $derived.by(() => {
    const info = editor.info;
    const view = editor.view;
    if (info === null || view === null) {
      return null;
    }
    return {
      left: -view.originX * view.zoom,
      top: -view.originY * view.zoom,
      width: info.width * view.zoom,
      height: info.height * view.zoom,
    };
  });
  const frameStyle = $derived.by(() => {
    const view = editor.view;
    return shown === null || view === null ? null : frameBox(shown, view);
  });

  // ---- Input ----

  function wheelAction(node: HTMLElement): { destroy(): void } {
    // A non-passive listener: the view stops the page from zooming or scrolling.
    const handler = (event: WheelEvent): void => {
      event.preventDefault();
      const rect = node.getBoundingClientRect();
      if (event.ctrlKey || event.metaKey) {
        editor.zoomBy(wheelZoomFactor(event.deltaY, event.deltaMode), { x: event.clientX - rect.left, y: event.clientY - rect.top });
      } else {
        const line = event.deltaMode === 1 ? 16 : 1;
        editor.pan(-event.deltaX * line, -event.deltaY * line);
      }
    };
    node.addEventListener('wheel', handler, { passive: false });
    return { destroy: () => node.removeEventListener('wheel', handler) };
  }

  let last = { x: 0, y: 0 };

  /** The pointer position in viewport pixels. */
  function local(event: PointerEvent | MouseEvent, node: HTMLElement): Point {
    const rect = node.getBoundingClientRect();
    return { x: event.clientX - rect.left, y: event.clientY - rect.top };
  }

  // ---- Selection gestures ----

  /** A rectangle or ellipse drag (screen points). */
  interface Marquee {
    readonly tool: 'rectSelect' | 'ellipseSelect';
    readonly mode: SelectionModeId;
    readonly start: Point;
    readonly current: Point;
  }
  /** A polygon lasso: the points are in canvas pixels, the pointer in screen pixels. */
  interface Lasso {
    readonly mode: SelectionModeId;
    readonly points: readonly Point[];
    readonly pointer: Point | null;
  }

  let marquee = $state.raw<Marquee | null>(null);
  let lasso = $state.raw<Lasso | null>(null);

  // A tool change or another document ends a gesture that runs.
  $effect(() => {
    void editor.tool;
    void editor.info?.documentId;
    untrack(() => {
      marquee = null;
      lasso = null;
    });
  });

  function lassoClick(event: PointerEvent, p: Point, view: View): void {
    if (lasso === null) {
      lasso = { mode: modeFromModifiers(event), points: [screenToCanvas(view, p)], pointer: p };
      return;
    }
    const screenPoints = lasso.points.map((c) => canvasToScreen(view, c));
    if (closesPolygon(screenPoints, p)) {
      finishLasso();
    } else if (!repeatsLastPoint(screenPoints, p)) {
      lasso = { ...lasso, points: [...lasso.points, screenToCanvas(view, p)], pointer: p };
    }
  }

  /** Sends the polygon if it has 3 points. Ends the gesture. */
  function finishLasso(): void {
    const current = lasso;
    lasso = null;
    const shape = current === null ? null : polygonShape(current.points);
    if (current !== null && shape !== null) {
      editor.selectShape(shape, current.mode);
    }
  }

  function finishMarquee(current: Marquee, end: Point, view: View): void {
    marquee = null;
    if (isClick(current.start, end)) {
      // A click with the replace mode removes the selection.
      if (current.mode === 'replace' && editor.hasSelection) {
        editor.deselect();
      }
      return;
    }
    const shape = dragShape(current.tool, current.start, end, view);
    if (shape !== null) {
      editor.selectShape(shape, current.mode);
    }
  }

  function onpointerdown(event: PointerEvent & { currentTarget: HTMLDivElement }): void {
    if (event.button === 1 || (event.button === 0 && spaceDown)) {
      event.preventDefault();
      panning = true;
      last = { x: event.clientX, y: event.clientY };
      event.currentTarget.setPointerCapture?.(event.pointerId);
      return;
    }
    const view = editor.view;
    const tool = editor.tool;
    if (event.button !== 0 || view === null || editor.info === null || tool === 'hand') {
      return;
    }
    event.preventDefault();
    const p = local(event, event.currentTarget);
    if (tool === 'polygonLasso') {
      lassoClick(event, p, view);
      return;
    }
    marquee = { tool, mode: modeFromModifiers(event), start: p, current: p };
    event.currentTarget.setPointerCapture?.(event.pointerId);
  }

  function onpointermove(event: PointerEvent & { currentTarget: HTMLDivElement }): void {
    if (panning) {
      editor.pan(event.clientX - last.x, event.clientY - last.y);
      last = { x: event.clientX, y: event.clientY };
    } else if (marquee !== null) {
      marquee = { ...marquee, current: local(event, event.currentTarget) };
    } else if (lasso !== null) {
      lasso = { ...lasso, pointer: local(event, event.currentTarget) };
    }
  }

  function onpointerup(event: PointerEvent & { currentTarget: HTMLDivElement }): void {
    if (panning) {
      panning = false;
      event.currentTarget.releasePointerCapture?.(event.pointerId);
    } else if (marquee !== null) {
      const view = editor.view;
      const current = marquee;
      event.currentTarget.releasePointerCapture?.(event.pointerId);
      if (view === null || event.type === 'pointercancel') {
        marquee = null;
      } else {
        finishMarquee(current, local(event, event.currentTarget), view);
      }
    }
  }

  function ondblclick(): void {
    if (lasso !== null) {
      finishLasso();
    }
  }

  function onwindowkey(event: KeyboardEvent): void {
    if (event.type === 'keydown' && (marquee !== null || lasso !== null) && !isEditableTarget(event.target)) {
      if (event.key === 'Escape') {
        event.preventDefault();
        marquee = null;
        lasso = null;
        return;
      }
      if (lasso !== null && event.key === 'Enter') {
        event.preventDefault();
        finishLasso();
        return;
      }
      if (lasso !== null && event.key === 'Backspace') {
        event.preventDefault();
        const points = lasso.points.slice(0, -1);
        lasso = points.length === 0 ? null : { ...lasso, points };
        return;
      }
    }
    if (event.code !== 'Space') {
      return;
    }
    if (event.type === 'keyup') {
      spaceDown = false;
    } else if (!isEditableTarget(event.target) && editor.info !== null) {
      spaceDown = true;
    }
  }

  /** The dashed preview of the gesture that runs, as an SVG path in viewport pixels. */
  const previewPath = $derived.by(() => {
    const view = editor.view;
    if (marquee !== null) {
      const box = boxBetween(marquee.start, marquee.current);
      return marquee.tool === 'rectSelect' ? boxPath(box) : ellipsePath(box);
    }
    if (lasso !== null && view !== null) {
      const points = lasso.points.map((c) => canvasToScreen(view, c));
      return polylinePath(lasso.pointer === null ? points : [...points, lasso.pointer]);
    }
    return '';
  });
</script>

<svelte:window onkeydown={onwindowkey} onkeyup={onwindowkey} onblur={() => (spaceDown = false)} />

<div class="editor">
  {#if editor.status === 'empty'}
    <div class="message" role="status">
      <p>Select a photo in the Library to edit it.</p>
      <button type="button" onclick={() => editor.requestNewDocument()}>New document…</button>
    </div>
  {:else if editor.status === 'loading'}
    <p class="message" role="status">Opening the photo…</p>
  {:else if editor.status === 'error'}
    <p class="message error" role="alert" data-testid="editor-error">
      The photo could not be opened in the Editor: {editor.loadError}
    </p>
  {/if}

  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="viewport"
    class:grab={spaceDown && !panning}
    class:grabbing={panning}
    class:crosshair={editor.tool !== 'hand' && !spaceDown && !panning}
    hidden={editor.status !== 'ready'}
    bind:clientWidth={width}
    bind:clientHeight={height}
    use:wheelAction
    {onpointerdown}
    {onpointermove}
    {onpointerup}
    onpointercancel={onpointerup}
    {ondblclick}
    data-testid="editor-viewport"
  >
    {#if docBox !== null}
      <div
        class="checker"
        style:left="{docBox.left}px"
        style:top="{docBox.top}px"
        style:width="{docBox.width}px"
        style:height="{docBox.height}px"
      ></div>
    {/if}
    <canvas
      bind:this={canvas}
      class="frame"
      hidden={frameStyle === null}
      style:left="{frameStyle?.left ?? 0}px"
      style:top="{frameStyle?.top ?? 0}px"
      style:width="{frameStyle?.width ?? 0}px"
      style:height="{frameStyle?.height ?? 0}px"
      aria-label="Document canvas"
    ></canvas>
    <SelectionOverlay {editor} />
    {#if previewPath !== ''}
      <svg class="preview" aria-hidden="true" data-testid="selection-preview">
        <path class="under" d={previewPath} />
        <path class="dashes" d={previewPath} />
      </svg>
    {/if}
    {#if editor.showMask}
      <div class="mask-badge" data-testid="mask-badge">Mask: {editor.selected?.name ?? ''}</div>
    {/if}
    {#if editor.zoomPercent !== null}
      <div class="zoom" aria-live="off">{editor.zoomPercent} %</div>
    {/if}
  </div>
  {#if editor.status === 'ready'}
    <SelectionToolbar {editor} />
  {/if}
</div>

{#if editor.newDocumentOpen}
  <NewDocumentDialog {editor} />
{/if}

{#if editor.featherDialogOpen}
  <FeatherDialog {editor} />
{/if}

<style>
  .editor {
    position: relative;
    width: 100%;
    height: 100%;
    background: var(--color-bg-center);
  }
  .viewport {
    position: absolute;
    inset: 0;
    overflow: hidden;
    touch-action: none;
  }
  .viewport[hidden] {
    display: none;
  }
  .grab {
    cursor: grab;
  }
  .grabbing {
    cursor: grabbing;
  }
  .crosshair {
    cursor: crosshair;
  }
  .preview {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    overflow: hidden;
    pointer-events: none;
  }
  .preview path {
    fill: none;
    stroke-width: 1px;
  }
  .preview .under {
    stroke: #fff;
  }
  .preview .dashes {
    stroke: #000;
    stroke-dasharray: 4 4;
  }
  .checker {
    position: absolute;
    background-color: #ffffff;
    background-image:
      linear-gradient(45deg, #cfcfcf 25%, transparent 25%, transparent 75%, #cfcfcf 75%),
      linear-gradient(45deg, #cfcfcf 25%, transparent 25%, transparent 75%, #cfcfcf 75%);
    background-size: 16px 16px;
    background-position:
      0 0,
      8px 8px;
  }
  .frame {
    position: absolute;
    image-rendering: auto;
    pointer-events: none;
  }
  .frame[hidden] {
    display: none;
  }
  .zoom {
    position: absolute;
    right: var(--space-3);
    bottom: var(--space-3);
    padding: var(--space-1) var(--space-2);
    border-radius: var(--radius-sm);
    background: var(--color-bg-panel);
    color: var(--color-text-muted);
    font-size: var(--font-size-sm);
    pointer-events: none;
  }
  .mask-badge {
    position: absolute;
    left: var(--space-3);
    bottom: var(--space-3);
    padding: var(--space-1) var(--space-2);
    border-radius: var(--radius-sm);
    background: var(--color-bg-panel);
    color: var(--color-text);
    font-size: var(--font-size-sm);
    pointer-events: none;
  }
  .message {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    align-items: center;
    justify-content: center;
    height: 100%;
    margin: 0;
    padding: var(--space-5);
    color: var(--color-text-muted);
    font-size: var(--font-size-md);
  }
  .error {
    color: var(--color-danger);
  }
</style>
