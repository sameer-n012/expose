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
-->
<script lang="ts">
  import { onDestroy, untrack } from 'svelte';
  import { fitCanvas, frameImageData } from '../../lib/develop/draw';
  import type { RenderArea } from '../../lib/editor/api';
  import { EditorFrameClient, type EditorFrame } from '../../lib/editor/frameClient';
  import { frameBox, hasArea, renderArea, wheelZoomFactor } from '../../lib/editor/view';
  import { isEditableTarget } from '../../lib/shortcuts/editable';
  import { editorApi } from '../../lib/editor/api';
  import type { EditorStore } from '../../lib/stores/editor.svelte';
  import NewDocumentDialog from './NewDocumentDialog.svelte';
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

  function onpointerdown(event: PointerEvent & { currentTarget: HTMLDivElement }): void {
    if (event.button === 1 || (event.button === 0 && spaceDown)) {
      event.preventDefault();
      panning = true;
      last = { x: event.clientX, y: event.clientY };
      event.currentTarget.setPointerCapture?.(event.pointerId);
    }
  }

  function onpointermove(event: PointerEvent): void {
    if (!panning) {
      return;
    }
    editor.pan(event.clientX - last.x, event.clientY - last.y);
    last = { x: event.clientX, y: event.clientY };
  }

  function onpointerup(event: PointerEvent & { currentTarget: HTMLDivElement }): void {
    if (panning) {
      panning = false;
      event.currentTarget.releasePointerCapture?.(event.pointerId);
    }
  }

  function onwindowkey(event: KeyboardEvent): void {
    if (event.code !== 'Space') {
      return;
    }
    if (event.type === 'keyup') {
      spaceDown = false;
    } else if (!isEditableTarget(event.target) && editor.info !== null) {
      spaceDown = true;
    }
  }
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
    hidden={editor.status !== 'ready'}
    bind:clientWidth={width}
    bind:clientHeight={height}
    use:wheelAction
    {onpointerdown}
    {onpointermove}
    {onpointerup}
    onpointercancel={onpointerup}
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
    {#if editor.showMask}
      <div class="mask-badge" data-testid="mask-badge">Mask: {editor.selected?.name ?? ''}</div>
    {/if}
    {#if editor.zoomPercent !== null}
      <div class="zoom" aria-live="off">{editor.zoomPercent} %</div>
    {/if}
  </div>
</div>

{#if editor.newDocumentOpen}
  <NewDocumentDialog {editor} />
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
    top: var(--space-3);
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
