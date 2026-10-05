<!--
  The develop preview: a canvas that shows the frame of the newest develop
  settings. A `PreviewClient` requests the frames (latest wins, up to two
  requests in flight) and a `FramePresenter` draws them in order, at most
  once per display frame. The component sends the device-pixel size of
  its box as the target long edge, so the shell renders no more pixels
  than the screen shows. CSS scales the canvas to fit the box and keeps
  the aspect ratio.

  The target is the long edge of the frame. With geometry on, the frame
  has the aspect ratio of the crop, so the target uses the aspect ratio of
  the last drawn frame (limited by the session long edge). When a frame
  arrives with a new aspect ratio (a new crop), the target is computed
  again.

  For the crop tool, `applyGeometry = false` requests frames of the full
  image, `transform` is a CSS transform of the canvas (about its center),
  and `overlay` draws on top of the image rect (the session aspect, scaled
  to fit). A request with the same session and the same settings as the
  last one is not sent again.
-->
<script lang="ts">
  import { onDestroy, untrack, type Snippet } from 'svelte';
  import { isCommandError } from '../../lib/catalog';
  import { containRect, type ScreenRect } from '../../lib/develop/cropView';
  import { frameContext, type DrawMode } from '../../lib/develop/draw';
  import {
    DEFAULT_PRESENT_MODE,
    FramePresenter,
    type PresentedEvent,
    type PresenterStats,
  } from '../../lib/develop/presenter';
  import { PreviewClient, type PreviewClientStats } from '../../lib/develop/previewClient';
  import { sameAspect, targetLongEdge, type Size } from '../../lib/develop/size';
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
    /** `false`: frames without flip, straighten, and crop. Default true. */
    applyGeometry?: boolean;
    /** CSS transform of the canvas, about its center. Default `none`. */
    transform?: string;
    /** Adds space around the image (the crop tool shows handles there). */
    padded?: boolean;
    /** Drawn over the image rect (in CSS pixels of the stage), when set. */
    overlay?: Snippet<[ScreenRect]> | undefined;
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
    applyGeometry = true,
    transform = 'none',
    padded = false,
    overlay,
  }: Props = $props();

  let box: HTMLDivElement | undefined = $state();
  let stage: HTMLDivElement | undefined = $state();
  let stageSize = $state({ width: 0, height: 0 });
  let canvas: HTMLCanvasElement | undefined = $state();

  /** The image rect in the stage (session aspect, scaled to fit). */
  const content: ScreenRect = $derived(
    containRect(stageSize.width, stageSize.height, session?.width ?? 0, session?.height ?? 0),
  );
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

  /**
   * The size of the last drawn frame with geometry on, for the aspect
   * ratio of the target. Null: use the session size (no frame yet, or
   * geometry off). Not reactive: only `updateTarget` reads it.
   */
  let frameShape: Size | null = null;

  /** Keeps the frame aspect ratio for the target; a new ratio updates the target. */
  function noteFrameShape(width: number, height: number): void {
    if (!untrack(() => applyGeometry) || !client.applyGeometry) {
      return;
    }
    const shape = { width, height };
    if (frameShape !== null && sameAspect(frameShape, shape)) {
      return;
    }
    const current = untrack(() => session);
    const before = frameShape ?? (current === null ? null : { width: current.width, height: current.height });
    frameShape = shape;
    if (before === null || !sameAspect(before, shape)) {
      updateTarget();
    }
  }

  const presenter = new FramePresenter({
    context: getContext,
    mode: untrack(() => drawMode),
    space: untrack(() => displaySpace),
    onPresented: (event) => {
      message = null;
      noteFrameShape(event.frame.width, event.frame.height);
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

  /** Measures the stage, for the overlay. */
  function measureStage(): void {
    if (stage === undefined) {
      return;
    }
    // The client size is the layout size: a CSS transform does not change
    // it (the stage has none), and it has no border or padding.
    const width = stage.clientWidth;
    const height = stage.clientHeight;
    if (width !== stageSize.width || height !== stageSize.height) {
      stageSize = { width, height };
    }
  }

  /**
   * Sends the device-pixel size of the box as the target long edge: the
   * fit of the frame aspect ratio (see `frameShape`) into the box, limited
   * by the session long edge.
   */
  function updateTarget(): void {
    untrack(measureStage);
    const current = untrack(() => session);
    if (box === undefined || current === null) {
      return;
    }
    const rect = box.getBoundingClientRect();
    const ratio = typeof devicePixelRatio === 'number' ? devicePixelRatio : 1;
    const shape = untrack(() => applyGeometry) && frameShape !== null ? frameShape : current;
    client.setTargetLongEdge(targetLongEdge(rect, ratio, shape, Math.max(current.width, current.height)));
  }

  $effect(() => {
    const id = session?.session ?? null;
    if (id !== client.session) {
      client.setSession(id);
      presenter.reset();
      frameShape = null;
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
    // The stage changes size without the box when the padding changes.
    if (stage !== undefined) {
      observer.observe(stage);
    }
    return () => observer.disconnect();
  });

  $effect(() => {
    // Before the settings effect, so that a request of the same update
    // already carries the new value.
    const on = applyGeometry;
    if (on !== client.applyGeometry) {
      // The frame shape of the other mode does not apply. Set the target
      // first, so that the first request of the new mode has it (the
      // client drops frames of requests with the old flag).
      frameShape = null;
      updateTarget();
      client.setApplyGeometry(on);
    }
  });

  /** The session and settings of the last request (JSON), to skip repeats. */
  let lastRequest: string | null = null;

  $effect(() => {
    // The snapshot reads all nested fields, so a change of any field
    // requests a frame.
    const snapshot = $state.snapshot(settings);
    if (session === null) {
      lastRequest = null;
      return;
    }
    const key = `${session.session}:${JSON.stringify(snapshot)}`;
    if (key !== lastRequest) {
      lastRequest = key;
      client.request(snapshot);
    }
  });

  $effect(() => {
    // A padding change resizes the stage.
    void padded;
    measureStage();
  });

  onDestroy(() => {
    client.dispose();
    presenter.dispose();
  });
</script>

<div class="preview" class:padded bind:this={box}>
  <div class="stage" bind:this={stage}>
    <canvas
      bind:this={canvas}
      width={session?.width ?? 0}
      height={session?.height ?? 0}
      style:transform
      aria-label="Develop preview"
    ></canvas>
    {#if overlay !== undefined && session !== null && content.width > 0}
      <div
        class="overlay"
        style:left="{content.x}px"
        style:top="{content.y}px"
        style:width="{content.width}px"
        style:height="{content.height}px"
      >
        {@render overlay(content)}
      </div>
    {/if}
  </div>
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
    box-sizing: border-box;
  }

  .padded {
    padding: var(--space-6);
  }

  .stage {
    position: relative;
    width: 100%;
    height: 100%;
  }

  /* The canvas fills the stage; `object-fit` scales the frame to fit and
     keeps the aspect ratio, for frames smaller or larger than the stage. */
  canvas {
    display: block;
    width: 100%;
    height: 100%;
    object-fit: contain;
  }

  .overlay {
    position: absolute;
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
