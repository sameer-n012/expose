<!--
  Center view of the Editor module: the document canvas.

  - A tab strip above the canvas shows the open documents (`DocumentTabs`).
    The canvas shows the active tab.
  - The view opens the library photo when the module is active and no
    tab is open (`editor.openTarget`).
  - A checkerboard shows behind the document. The shell renders the visible
    part of the document at device pixel size, with straight alpha.
  - Each view change or document change requests a new frame (at most 2 in
    flight, latest wins). Until the new frame arrives, the old frame shows
    scaled and moved to the new view.
  - Wheel: scroll. With the primary modifier (and for pinch): zoom about the
    pointer. Middle button, the space key and the left button, or the hand
    tool and the left button: pan.
  - View rotation (ADR 0026): the checkerboard, the frame, and all the
    canvas-space overlays are in one container (`.rotator`) with a CSS
    rotation about the viewport center. The view math works in the space
    before the rotation ("content space"); `local()` turns each pointer
    point back by the rotation. Screen-space items (zoom readout, mask
    badge, rotation readout, toolbar) are outside the container.
  - Rotate view tool: a drag turns the view about the viewport center;
    Shift snaps to 15° steps; Escape or a cancelled pointer gives back the
    start angle.
  - Tools (`editor.tool`): hand (drag to pan), rotate view, rectangle and
    ellipse marquee (drag), freehand lasso (drag; release closes), polygon
    lasso (click points). The modifier keys at
    the first pointer-down set the selection mode. The outline of the
    selection and the preview of a gesture are SVG layers over the canvas.
  - Brush and eraser: a drag is one stroke (`StrokeClient`). Escape or a
    cancelled pointer cancels it. A circle of the brush size follows the pointer.
    Alt+click with the brush does not paint: it samples the composite color
    under the pointer (current sample size) into the foreground color.
  - Magic wand and paint bucket: a click inside the canvas sends one edit
    (Shift adds, Alt subtracts, Shift+Alt intersects for the wand).
  - Gradient: a drag from start to end shows a line with two handles; Shift
    snaps the angle to 45° steps; Escape cancels; release sends one edit. A
    drag as short as a click sends nothing.
  - Eyedropper: a click samples the color under the pointer into the
    foreground color; Alt+click sets the background color. The options are
    the sample size and the source (all layers or the selected layer).
-->
<script lang="ts">
  import { onDestroy, untrack } from 'svelte';
  import { fitCanvas, frameImageData } from '../../lib/develop/draw';
  import type { DocumentInfo, RenderArea } from '../../lib/editor/api';
  import { CROSSHAIR_BELOW, cursorDiameter, pointerPressure } from '../../lib/editor/brush';
  import { StrokeClient } from '../../lib/editor/strokeClient';
  import { EditorFrameClient, type EditorFrame } from '../../lib/editor/frameClient';
  import {
    boxBetween,
    boxPath,
    appendLassoPoint,
    closesPolygon,
    dragShape,
    ellipsePath,
    isClick,
    isSelectionTool,
    modeFromModifiers,
    polygonShape,
    polylinePath,
    repeatsLastPoint,
  } from '../../lib/editor/selection';
  import { snapAngle } from '../../lib/editor/gradient';
  import { beginRotateDrag, cancelRotateDrag, dragRotation, rotationLabel, type RotateDrag } from '../../lib/editor/rotate';
  import {
    canvasToScreen,
    frameBox,
    hasArea,
    renderArea,
    screenToCanvas,
    unrotatePoint,
    wheelZoomFactor,
    type Point,
    type View,
  } from '../../lib/editor/view';
  import { isEditableTarget } from '../../lib/shortcuts/editable';
  import { editorApi, type SelectionModeId } from '../../lib/editor/api';
  import type { EditorStore } from '../../lib/stores/editor.svelte';
  import DocumentTabs from './DocumentTabs.svelte';
  import FeatherDialog from './FeatherDialog.svelte';
  import GradientEditorDialog from './GradientEditorDialog.svelte';
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
    // A frame that shows a mask can fail after the layer lost its mask: not an error then.
    showsMask: (layerId) => editor.showMaskLayerId === layerId,
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

  // A new document or a tab switch: forget the old frame.
  $effect(() => {
    const id = editor.info?.documentId ?? null;
    untrack(() => {
      client.setDocument(id);
      shown = null;
    });
  });

  /** Asks for a frame of the current view. */
  function requestFrame(): void {
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
  }

  // Every document or view change asks for a new frame.
  $effect(requestFrame);

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
  /** The view rotation of the active tab, in degrees. */
  const rotation = $derived(editor.rotation);
  const frameStyle = $derived.by(() => {
    const view = editor.view;
    return shown === null || view === null ? null : frameBox(shown, view);
  });

  // ---- Input ----

  function wheelAction(node: HTMLElement): { destroy(): void } {
    // A non-passive listener: the view stops the page from zooming or scrolling.
    const handler = (event: WheelEvent): void => {
      event.preventDefault();
      if (event.ctrlKey || event.metaKey) {
        // The canvas point under the pointer stays there: the anchor is in content space.
        editor.zoomBy(wheelZoomFactor(event.deltaY, event.deltaMode), local(event, node));
      } else {
        const line = event.deltaMode === 1 ? 16 : 1;
        // A screen distance: the store turns it by the view rotation.
        editor.pan(-event.deltaX * line, -event.deltaY * line);
      }
    };
    node.addEventListener('wheel', handler, { passive: false });
    return { destroy: () => node.removeEventListener('wheel', handler) };
  }

  let last = { x: 0, y: 0 };

  /** The pointer position in viewport pixels, on the screen (before the view rotation). */
  function screenPoint(event: PointerEvent | MouseEvent, node: HTMLElement): Point {
    const rect = node.getBoundingClientRect();
    return { x: event.clientX - rect.left, y: event.clientY - rect.top };
  }

  /**
   * The pointer position in content space: the screen point turned back by
   * the view rotation about the viewport center. All the canvas math and
   * the overlays in `.rotator` use this space. At rotation 0 it is the screen point.
   */
  function local(event: PointerEvent | MouseEvent, node: HTMLElement): Point {
    return unrotatePoint(screenPoint(event, node), editor.viewport, editor.rotation);
  }

  // ---- Rotate view tool ----

  /** The rotate drag that runs, or null. */
  let rotating = $state.raw<{ readonly drag: RotateDrag; readonly pointerId: number; readonly node: HTMLElement } | null>(null);

  /** Ends the rotate drag. `restore`: go back to the angle at the start (Escape, cancel). */
  function endRotate(restore: boolean): void {
    const current = rotating;
    if (current === null) {
      return;
    }
    rotating = null;
    current.node.releasePointerCapture?.(current.pointerId);
    if (restore) {
      editor.setRotation(cancelRotateDrag(current.drag));
    }
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

  /** A freehand lasso drag: the points are in canvas pixels; `start` is in screen pixels. */
  interface Freehand {
    readonly mode: SelectionModeId;
    readonly points: readonly Point[];
    readonly start: Point;
    /** True when the pointer has moved farther than a click. */
    readonly moved: boolean;
    readonly pointerId: number;
  }

  let marquee = $state.raw<Marquee | null>(null);
  let lasso = $state.raw<Lasso | null>(null);
  let freehand = $state.raw<Freehand | null>(null);

  /** A gradient drag. `start` and `end` are canvas pixels; `startScreen` is in content space. */
  interface GradientDrag {
    readonly start: Point;
    readonly end: Point;
    readonly startScreen: Point;
    readonly pointerId: number;
  }
  let gradientDrag = $state.raw<GradientDrag | null>(null);

  /** The end of a gradient drag at a pointer event. Shift snaps the angle to 45° steps. */
  function gradientEnd(event: PointerEvent, node: HTMLElement, view: View, start: Point): Point {
    const end = screenToCanvas(view, local(event, node));
    return event.shiftKey ? snapAngle(start, end) : end;
  }

  function endGradientDrag(event: PointerEvent, node: HTMLElement): void {
    const current = gradientDrag;
    gradientDrag = null;
    node.releasePointerCapture?.(event.pointerId);
    const view = editor.view;
    if (current === null || view === null || event.type === 'pointercancel') {
      return;
    }
    if (isClick(current.startScreen, local(event, node))) {
      return;
    }
    editor.gradientFrom(current.start, gradientEnd(event, node, view, current.start));
  }

  // A tool change or another document (also a tab switch) ends a gesture that runs.
  $effect(() => {
    void editor.tool;
    void editor.info?.documentId;
    untrack(() => {
      marquee = null;
      lasso = null;
      freehand = null;
      gradientDrag = null;
      cancelStroke();
      // Stop a rotate drag and keep the angle: after a tab switch the store
      // works on the new tab, so a restore would change the wrong view.
      endRotate(false);
    });
  });

  // ---- Brush and eraser strokes ----

  interface RunningStroke {
    readonly client: StrokeClient;
    readonly pointerId: number;
    readonly node: HTMLElement;
  }

  let stroke: RunningStroke | null = null;
  /** The pointer position in viewport pixels, for the brush cursor. Null: hidden. */
  let cursor = $state.raw<Point | null>(null);

  /** The points of a pointer event (and its coalesced events) as [x, y, pressure, ...] in canvas pixels. */
  function strokePoints(event: PointerEvent, node: HTMLElement, view: View): number[] {
    const events = typeof event.getCoalescedEvents === 'function' ? event.getCoalescedEvents() : [];
    const list = events.length > 0 ? events : [event];
    const out: number[] = [];
    for (const e of list) {
      const c = screenToCanvas(view, local(e, node));
      out.push(c.x, c.y, pointerPressure(e));
    }
    return out;
  }

  function startStroke(event: PointerEvent, node: HTMLElement, view: View): void {
    const spec = editor.strokeSpec();
    const info = editor.info;
    if (spec === null || info === null) {
      return;
    }
    const client = new StrokeClient({
      api: editor.api,
      documentId: info.documentId,
      layerId: spec.layerId,
      target: spec.target,
      brush: spec.brush,
      onProgress: requestFrame,
      onBeginError: (error) => {
        if (stroke?.client === client) {
          stroke = null;
        }
        toasts.error(`Start the stroke failed: ${describeError(error)}`, error);
      },
      onError: (error) => toasts.error(`Paint the stroke failed: ${describeError(error)}`, error),
    });
    stroke = { client, pointerId: event.pointerId, node };
    node.setPointerCapture?.(event.pointerId);
    client.addPoints(strokePoints(event, node, view));
  }

  function finishStroke(event: PointerEvent): void {
    const current = stroke;
    if (current === null || event.pointerId !== current.pointerId) {
      return;
    }
    stroke = null;
    current.node.releasePointerCapture?.(event.pointerId);
    const view = editor.view;
    if (event.type === 'pointercancel') {
      void current.client.cancel().then(applyStrokeInfo);
      return;
    }
    if (view !== null) {
      current.client.addPoints(strokePoints(event, current.node, view));
    }
    void current.client.end().then(applyStrokeInfo);
  }

  function cancelStroke(): void {
    const current = stroke;
    if (current === null) {
      return;
    }
    stroke = null;
    current.node.releasePointerCapture?.(current.pointerId);
    void current.client.cancel().then(applyStrokeInfo);
  }

  function applyStrokeInfo(info: DocumentInfo | null): void {
    if (info !== null && !disposed) {
      editor.applyInfo(info);
    }
  }

  const paintReady = $derived(editor.paintTool !== null && editor.paintBlockReason === null);
  const cursorShape = $derived.by(() => {
    const tool = editor.paintTool;
    const view = editor.view;
    if (tool === null || view === null || cursor === null || !paintReady || spaceDown || panning) {
      return null;
    }
    const diameter = cursorDiameter(editor.brushOf(tool).size, view.zoom);
    return { x: cursor.x, y: cursor.y, radius: diameter / 2, crosshair: diameter < CROSSHAIR_BELOW };
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

  /** Adds the points of a pointer event (with its coalesced events) to the freehand path. */
  function extendFreehand(event: PointerEvent, node: HTMLElement, view: View): void {
    const current = freehand;
    if (current === null) {
      return;
    }
    const events = typeof event.getCoalescedEvents === 'function' ? event.getCoalescedEvents() : [];
    const list = events.length > 0 ? events : [event];
    // One screen pixel in canvas pixels.
    const minDistance = 1 / view.zoom;
    const points = [...current.points];
    let moved = current.moved;
    let added = false;
    for (const e of list) {
      const screen = local(e, node);
      moved = moved || !isClick(current.start, screen);
      added = appendLassoPoint(points, screenToCanvas(view, screen), minDistance) || added;
    }
    if (added || moved !== current.moved) {
      freehand = { ...current, points, moved };
    }
  }

  /** Closes the freehand path and sends it. A click without a drag removes the selection. */
  function finishFreehand(event: PointerEvent, node: HTMLElement): void {
    const view = editor.view;
    if (view !== null && event.type !== 'pointercancel') {
      extendFreehand(event, node, view);
    }
    const current = freehand;
    freehand = null;
    node.releasePointerCapture?.(event.pointerId);
    if (current === null || event.type === 'pointercancel') {
      return;
    }
    if (!current.moved) {
      if (current.mode === 'replace' && editor.hasSelection) {
        editor.deselect();
      }
      return;
    }
    const shape = polygonShape(current.points);
    if (shape !== null) {
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

  /**
   * Samples the pixel under the pointer. A point outside the canvas is
   * ignored. The eyedropper uses its source option; the brush Alt+click
   * always reads all layers.
   */
  function sampleAtPointer(event: PointerEvent, node: HTMLElement, view: View, toBackground: boolean): void {
    const info = editor.info;
    if (info === null) {
      return;
    }
    const point = screenToCanvas(view, local(event, node));
    const x = Math.floor(point.x);
    const y = Math.floor(point.y);
    if (x < 0 || y < 0 || x >= info.width || y >= info.height) {
      return;
    }
    void editor.sampleAt(x, y, { toBackground, source: editor.tool === 'eyedropper' ? editor.sampleSource : 'composite' });
  }

  /**
   * A click of the magic wand or the paint bucket. A point outside the canvas
   * is ignored. The wand mode comes from the modifier keys.
   */
  function floodClick(event: PointerEvent, node: HTMLElement, view: View, tool: 'magicWand' | 'paintBucket'): void {
    const info = editor.info;
    if (info === null) {
      return;
    }
    const point = screenToCanvas(view, local(event, node));
    const x = Math.floor(point.x);
    const y = Math.floor(point.y);
    if (x < 0 || y < 0 || x >= info.width || y >= info.height) {
      return;
    }
    if (tool === 'magicWand') {
      editor.magicWandAt(x, y, modeFromModifiers(event));
    } else {
      editor.fillAt(x, y);
    }
  }

  function onpointerdown(event: PointerEvent & { currentTarget: HTMLDivElement }): void {
    if (rotating !== null) {
      return;
    }
    if (event.button === 1 || (event.button === 0 && (spaceDown || (editor.tool === 'hand' && editor.info !== null)))) {
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
    if (tool === 'rotate') {
      // The angle is around the viewport center on the screen, so this uses the screen point.
      rotating = {
        drag: beginRotateDrag(view.rotation, screenPoint(event, event.currentTarget), editor.viewport),
        pointerId: event.pointerId,
        node: event.currentTarget,
      };
      event.currentTarget.setPointerCapture?.(event.pointerId);
      return;
    }
    if (tool === 'magicWand' || tool === 'paintBucket') {
      floodClick(event, event.currentTarget, view, tool);
      return;
    }
    if (tool === 'gradient') {
      if (gradientDrag === null) {
        const startScreen = local(event, event.currentTarget);
        const start = screenToCanvas(view, startScreen);
        gradientDrag = { start, end: start, startScreen, pointerId: event.pointerId };
        event.currentTarget.setPointerCapture?.(event.pointerId);
      }
      return;
    }
    if (tool === 'eyedropper' || (tool === 'brush' && event.altKey)) {
      // Alt+click with the brush samples to the foreground (all layers); it paints nothing.
      sampleAtPointer(event, event.currentTarget, view, tool === 'eyedropper' && event.altKey);
      return;
    }
    if (tool === 'brush' || tool === 'eraser') {
      if (stroke === null) {
        startStroke(event, event.currentTarget, view);
      }
      return;
    }
    const p = local(event, event.currentTarget);
    if (tool === 'lasso') {
      if (freehand === null) {
        freehand = {
          mode: modeFromModifiers(event),
          points: [screenToCanvas(view, p)],
          start: p,
          moved: false,
          pointerId: event.pointerId,
        };
        event.currentTarget.setPointerCapture?.(event.pointerId);
      }
      return;
    }
    if (tool === 'polygonLasso') {
      lassoClick(event, p, view);
      return;
    }
    marquee = { tool, mode: modeFromModifiers(event), start: p, current: p };
    event.currentTarget.setPointerCapture?.(event.pointerId);
  }

  function onpointermove(event: PointerEvent & { currentTarget: HTMLDivElement }): void {
    cursor = local(event, event.currentTarget);
    if (rotating !== null) {
      if (event.pointerId === rotating.pointerId) {
        editor.setRotation(dragRotation(rotating.drag, screenPoint(event, event.currentTarget), editor.viewport, event.shiftKey));
      }
    } else if (stroke !== null) {
      const view = editor.view;
      if (view !== null && event.pointerId === stroke.pointerId) {
        stroke.client.addPoints(strokePoints(event, event.currentTarget, view));
      }
    } else if (panning) {
      editor.pan(event.clientX - last.x, event.clientY - last.y);
      last = { x: event.clientX, y: event.clientY };
    } else if (gradientDrag !== null) {
      const view = editor.view;
      if (view !== null && event.pointerId === gradientDrag.pointerId) {
        gradientDrag = { ...gradientDrag, end: gradientEnd(event, event.currentTarget, view, gradientDrag.start) };
      }
    } else if (freehand !== null) {
      const view = editor.view;
      if (view !== null && event.pointerId === freehand.pointerId) {
        extendFreehand(event, event.currentTarget, view);
      }
    } else if (marquee !== null) {
      marquee = { ...marquee, current: local(event, event.currentTarget) };
    } else if (lasso !== null) {
      lasso = { ...lasso, pointer: local(event, event.currentTarget) };
    }
  }

  function onpointerup(event: PointerEvent & { currentTarget: HTMLDivElement }): void {
    if (rotating !== null) {
      if (event.pointerId === rotating.pointerId) {
        endRotate(event.type === 'pointercancel');
      }
    } else if (stroke !== null) {
      finishStroke(event);
    } else if (panning) {
      panning = false;
      event.currentTarget.releasePointerCapture?.(event.pointerId);
    } else if (gradientDrag !== null) {
      if (event.pointerId === gradientDrag.pointerId) {
        endGradientDrag(event, event.currentTarget);
      }
    } else if (freehand !== null) {
      if (event.pointerId === freehand.pointerId) {
        finishFreehand(event, event.currentTarget);
      }
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
    if (event.type === 'keydown' && event.key === 'Escape' && rotating !== null) {
      event.preventDefault();
      endRotate(true);
      return;
    }
    if (event.type === 'keydown' && event.key === 'Escape' && stroke !== null) {
      event.preventDefault();
      cancelStroke();
      return;
    }
    if (event.type === 'keydown' && event.key === 'Escape' && gradientDrag !== null) {
      event.preventDefault();
      gradientDrag = null;
      return;
    }
    if (event.type === 'keydown' && (marquee !== null || lasso !== null || freehand !== null) && !isEditableTarget(event.target)) {
      if (event.key === 'Escape') {
        event.preventDefault();
        marquee = null;
        lasso = null;
        freehand = null;
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

  /** The line of the gradient drag with its two end handles, in viewport pixels. Null: no drag. */
  const gradientLine = $derived.by(() => {
    const view = editor.view;
    if (gradientDrag === null || view === null) {
      return null;
    }
    const a = canvasToScreen(view, gradientDrag.start);
    const b = canvasToScreen(view, gradientDrag.end);
    return { path: `M ${a.x} ${a.y} L ${b.x} ${b.y}`, ends: [a, b] };
  });

  /** The dashed preview of the gesture that runs, as an SVG path in viewport pixels. */
  const previewPath = $derived.by(() => {
    const view = editor.view;
    if (marquee !== null) {
      const box = boxBetween(marquee.start, marquee.current);
      return marquee.tool === 'rectSelect' ? boxPath(box) : ellipsePath(box);
    }
    if (freehand !== null && view !== null) {
      return polylinePath(freehand.points.map((c) => canvasToScreen(view, c)));
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
  {#if editor.tabs.length > 0}
    <DocumentTabs {editor} />
  {/if}
  <div class="stage">
    {#if editor.status === 'empty'}
      <div class="message" role="status">
        <p>Select a photo in the Library to edit it.</p>
        <button type="button" onclick={() => editor.requestNewDocument()}>New document…</button>
      </div>
    {:else if editor.status === 'loading'}
      <p class="message" role="status">Opening the document…</p>
    {:else if editor.status === 'error'}
      <p class="message error" role="alert" data-testid="editor-error">
        The document could not be opened in the Editor: {editor.loadError}
      </p>
    {/if}

    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div
      class="viewport"
      class:grab={(spaceDown || editor.tool === 'hand' || editor.tool === 'rotate') && !panning && rotating === null}
      class:grabbing={panning || rotating !== null}
      class:crosshair={(isSelectionTool(editor.tool) || editor.tool === 'eyedropper' || editor.tool === 'magicWand' || editor.tool === 'paintBucket' || editor.tool === 'gradient') && !spaceDown && !panning}
      class:no-cursor={cursorShape !== null}
      onpointerleave={() => (cursor = null)}
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
      <!-- Canvas-space layers: one CSS rotation about the viewport center (ADR 0026). -->
      <div class="rotator" style:transform={rotation === 0 ? null : `rotate(${rotation}deg)`} data-testid="editor-rotator">
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
        {#if gradientLine !== null}
          <svg class="preview" aria-hidden="true" data-testid="gradient-overlay">
            <path class="under" d={gradientLine.path} />
            <path class="dashes solid" d={gradientLine.path} />
            {#each gradientLine.ends as p, i (i)}
              <circle class="under" cx={p.x} cy={p.y} r="4" />
              <circle class="dashes solid" cx={p.x} cy={p.y} r="3" />
            {/each}
          </svg>
        {/if}
        {#if cursorShape !== null}
          <svg class="preview" aria-hidden="true" data-testid="brush-cursor">
            <circle class="under" cx={cursorShape.x} cy={cursorShape.y} r={cursorShape.radius} />
            <circle class="dashes solid" cx={cursorShape.x} cy={cursorShape.y} r={cursorShape.radius} />
            {#if cursorShape.crosshair}
              <path class="under" d="M {cursorShape.x - 6} {cursorShape.y} H {cursorShape.x + 6} M {cursorShape.x} {cursorShape.y - 6} V {cursorShape.y + 6}" />
              <path class="dashes solid" d="M {cursorShape.x - 5} {cursorShape.y} H {cursorShape.x + 5} M {cursorShape.x} {cursorShape.y - 5} V {cursorShape.y + 5}" />
            {/if}
          </svg>
        {/if}
      </div>
      {#if editor.showMask}
        <div class="mask-badge" data-testid="mask-badge">Mask: {editor.selected?.name ?? ''}</div>
      {/if}
      {#if editor.zoomPercent !== null}
        <div class="zoom" aria-live="off">{editor.zoomPercent} %</div>
      {/if}
    </div>
    {#if editor.status === 'ready' && rotation !== 0}
      <div class="rotation" role="group" aria-label="View rotation" data-testid="rotation-readout">
        <span>{rotationLabel(rotation)}</span>
        <button type="button" title="Set the view rotation to 0°" onclick={() => editor.resetRotation()}>Reset</button>
      </div>
    {/if}
    {#if editor.status === 'ready'}
      <SelectionToolbar {editor} />
    {/if}
  </div>
</div>

{#if editor.newDocumentOpen}
  <NewDocumentDialog {editor} />
{/if}

{#if editor.featherDialogOpen}
  <FeatherDialog {editor} />
{/if}

{#if editor.gradientDialogOpen}
  <GradientEditorDialog {editor} />
{/if}

<style>
  .editor {
    display: flex;
    flex-direction: column;
    width: 100%;
    height: 100%;
    background: var(--color-bg-center);
  }
  /* The area below the tab strip: the canvas, the toolbar, and the messages. */
  .stage {
    position: relative;
    flex: 1;
    min-height: 0;
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
  /* The brush circle is the cursor. */
  .no-cursor {
    cursor: none;
  }
  .preview .solid {
    stroke-dasharray: none;
  }
  .preview circle {
    fill: none;
    stroke-width: 1px;
  }
  .preview {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    /* The rotator can turn this layer: the viewport clips it, not the layer box. */
    overflow: visible;
    pointer-events: none;
  }
  /* Same box as the viewport; the rotation is about its center. */
  .rotator {
    position: absolute;
    inset: 0;
    transform-origin: 50% 50%;
  }
  .rotation {
    position: absolute;
    right: var(--space-3);
    bottom: calc(var(--space-3) * 2 + 1.75em);
    display: flex;
    gap: var(--space-2);
    align-items: center;
    padding: var(--space-1) var(--space-2);
    border-radius: var(--radius-sm);
    background: var(--color-bg-panel);
    color: var(--color-text-muted);
    font-size: var(--font-size-sm);
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
