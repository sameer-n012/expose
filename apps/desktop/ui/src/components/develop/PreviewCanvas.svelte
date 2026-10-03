<!--
  The develop preview: a canvas that shows the frame of the newest develop
  settings. A `PreviewClient` requests the frames (latest wins) and this
  component draws them. The canvas has the preview size in pixels; CSS
  scales it to fit the box and keeps the aspect ratio.
-->
<script lang="ts">
  import { onDestroy, untrack } from 'svelte';
  import { isCommandError } from '../../lib/catalog';
  import { DEFAULT_DRAW_MODE, drawFrame, frameContext, type DrawMode } from '../../lib/develop/draw';
  import { PreviewClient, type FrameEvent } from '../../lib/develop/previewClient';
  import {
    selectTransport,
    type DevelopSettingsJson,
    type DisplaySpace,
    type FrameTransport,
    type SessionInfo,
  } from '../../lib/develop/transport';
  import { describeError } from '../../lib/stores/toasts.svelte';

  interface Props {
    /** The open preview session, or null (the canvas is empty). */
    session: SessionInfo | null;
    /** The develop settings. Each change requests a new frame. */
    settings: DevelopSettingsJson;
    /** The frame transport. Read once, at creation. Default: `selectTransport()`. */
    transport?: FrameTransport;
    /** The display color space of the session. */
    displaySpace?: DisplaySpace;
    drawMode?: DrawMode;
    /** Called after each drawn frame. */
    onframe?: (event: FrameEvent) => void;
    /** Called for each failed frame. The component also shows the message. */
    onerror?: (error: unknown) => void;
  }

  let {
    session,
    settings,
    transport,
    displaySpace = 'srgb',
    drawMode = DEFAULT_DRAW_MODE,
    onframe,
    onerror,
  }: Props = $props();

  let canvas: HTMLCanvasElement | undefined = $state();
  let message: string | null = $state(null);
  let context: CanvasRenderingContext2D | null = null;
  /** Draws run one after the other, so that a slow draw cannot overwrite a newer frame. */
  let drawing: Promise<void> = Promise.resolve();

  function draw(event: FrameEvent): void {
    if (canvas === undefined) {
      return;
    }
    context ??= frameContext(canvas, displaySpace);
    const ctx = context;
    if (ctx === null) {
      message = 'The web view has no 2D canvas.';
      return;
    }
    drawing = drawing
      .then(() => drawFrame(ctx, event.frame, drawMode, displaySpace))
      .then(() => {
        message = null;
        onframe?.(event);
      })
      .catch(report);
  }

  function report(error: unknown): void {
    message = isCommandError(error) ? error.message : describeError(error);
    onerror?.(error);
  }

  const client = new PreviewClient({
    transport: untrack(() => transport) ?? selectTransport(),
    onFrame: draw,
    onError: report,
  });

  $effect(() => {
    client.setSession(session?.session ?? null);
  });

  $effect(() => {
    // The snapshot reads all nested fields, so a change of any field
    // requests a frame.
    const snapshot = $state.snapshot(settings);
    if (session !== null) {
      client.request(snapshot);
    }
  });

  onDestroy(() => client.dispose());
</script>

<div class="preview">
  <canvas
    bind:this={canvas}
    width={session?.width ?? 0}
    height={session?.height ?? 0}
    aria-label="Develop preview"
  ></canvas>
  {#if message !== null}
    <p class="message" role="alert">{message}</p>
  {/if}
</div>

<style>
  .preview {
    position: relative;
    display: flex;
    align-items: center;
    justify-content: center;
    width: 100%;
    height: 100%;
    overflow: hidden;
  }

  canvas {
    max-width: 100%;
    max-height: 100%;
    object-fit: contain;
  }

  .message {
    position: absolute;
    bottom: var(--space-2);
    margin: 0;
    padding: var(--space-1) var(--space-2);
    border-radius: var(--radius-sm);
    background: var(--color-bg-panel-raised);
    color: var(--color-danger);
    font-size: var(--font-size-sm);
  }
</style>
