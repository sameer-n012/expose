<!--
  The develop preview: a canvas that shows the frame of the newest develop
  settings. A `PreviewClient` requests the frames (latest wins, up to two
  requests in flight) and a `FramePresenter` draws them in order, at most
  once per display frame. The component sends the device-pixel size of
  its box as the target long edge, so the shell renders no more pixels
  than the screen shows. CSS scales the canvas to fit the box and keeps
  the aspect ratio.
-->
<script lang="ts">
  import { onDestroy, untrack } from 'svelte';
  import { isCommandError } from '../../lib/catalog';
  import { frameContext, type DrawMode } from '../../lib/develop/draw';
  import {
    DEFAULT_PRESENT_MODE,
    FramePresenter,
    type PresentedEvent,
    type PresenterStats,
  } from '../../lib/develop/presenter';
  import { PreviewClient, type PreviewClientStats } from '../../lib/develop/previewClient';
  import { targetLongEdge } from '../../lib/develop/size';
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
    /** The display color space of the session. Read once, at creation. */
    displaySpace?: DisplaySpace;
    /** The draw mode. Read once, at creation. Default: `bitmap` (falls back to `put`). */
    drawMode?: DrawMode;
    /** The maximum number of frame requests in flight. Read once, at creation. Default 2. */
    maxInFlight?: number;
    /** Called after each drawn frame. */
    onframe?: (event: PresentedEvent) => void;
    /** Called for each failed frame. The component also shows the message. */
    onerror?: (error: unknown) => void;
  }

  let {
    session,
    settings,
    transport,
    displaySpace = 'srgb',
    drawMode = DEFAULT_PRESENT_MODE,
    maxInFlight,
    onframe,
    onerror,
  }: Props = $props();

  let box: HTMLDivElement | undefined = $state();
  let canvas: HTMLCanvasElement | undefined = $state();
  let message: string | null = $state(null);
  let context: CanvasRenderingContext2D | null = null;

  function getContext(): CanvasRenderingContext2D | null {
    if (canvas === undefined) {
      return null;
    }
    context ??= frameContext(canvas, untrack(() => displaySpace));
    return context;
  }

  function report(error: unknown): void {
    message = isCommandError(error) ? error.message : describeError(error);
    onerror?.(error);
  }

  const presenter = new FramePresenter({
    context: getContext,
    mode: untrack(() => drawMode),
    space: untrack(() => displaySpace),
    onPresented: (event) => {
      message = null;
      onframe?.(event);
    },
    onError: report,
  });

  const client = new PreviewClient({
    transport: untrack(() => transport) ?? selectTransport(),
    onFrame: (event) => presenter.present(event),
    onError: report,
    ...untrack(() => (maxInFlight === undefined ? {} : { maxInFlight })),
  });

  /** The counters of the client and the presenter (for diagnostics). */
  export function getStats(): { client: PreviewClientStats; presenter: PresenterStats } {
    return { client: { ...client.stats }, presenter: { ...presenter.stats } };
  }

  /** Sends the device-pixel size of the box as the target long edge. */
  function updateTarget(): void {
    const current = untrack(() => session);
    if (box === undefined || current === null) {
      return;
    }
    const rect = box.getBoundingClientRect();
    const ratio = typeof devicePixelRatio === 'number' ? devicePixelRatio : 1;
    client.setTargetLongEdge(targetLongEdge(rect, ratio, current));
  }

  $effect(() => {
    const id = session?.session ?? null;
    if (id !== client.session) {
      client.setSession(id);
      presenter.reset();
    }
    // Measure before the first request of the session, so that its
    // first frame already has the right size.
    updateTarget();
  });

  $effect(() => {
    if (box === undefined || typeof ResizeObserver === 'undefined') {
      return;
    }
    const observer = new ResizeObserver(() => updateTarget());
    observer.observe(box);
    return () => observer.disconnect();
  });

  $effect(() => {
    // The snapshot reads all nested fields, so a change of any field
    // requests a frame.
    const snapshot = $state.snapshot(settings);
    if (session !== null) {
      client.request(snapshot);
    }
  });

  onDestroy(() => {
    client.dispose();
    presenter.dispose();
  });
</script>

<div class="preview" bind:this={box}>
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

  /* The canvas fills the box; `object-fit` scales the frame to fit and
     keeps the aspect ratio, for frames smaller or larger than the box. */
  canvas {
    width: 100%;
    height: 100%;
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
