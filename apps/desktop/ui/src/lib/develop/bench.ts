/**
 * Transport benchmark (development only). It measures, in the real web
 * view, how fast preview frames go from the shell to a canvas, for each
 * transport candidate.
 *
 * Run: start the app with `npm --prefix apps/desktop/ui run tauri dev`,
 * open the web inspector, and enter `location.search = '?bench=transport'`.
 * Optional query values: `frames` (default 60), `long` (preview long edge,
 * default 2560). The page shows a table and logs it to the console.
 *
 * The source is a synthetic 6000 x 4000 test image in the shell, so no
 * photo is necessary. Each frame has new develop settings (exposure
 * steps), so each frame is a real GPU render.
 *
 * Candidates:
 *
 * - `pipelined-*`: the production path. Two requests in flight and a
 *   `FramePresenter`. `@<n>` sends the target long edge `n`.
 * - `slider-*`: a request every 4 ms for 2 s. `-pipelined` is the
 *   production path; the others keep one request in flight.
 * - `ipc-*`, `scheme-*`, `channel-*`: one frame at a time (the paths
 *   before pipelining) and encoded frames, for comparison.
 */
import { Channel, invoke } from '@tauri-apps/api/core';
import { drawFrame, frameContext, type DrawMode } from './draw';
import { FRAME_HEADER_BYTES, parseFrame, type Frame } from './frame';
import { describeError } from '../stores/toasts.svelte';
import { FramePresenter } from './presenter';
import { PreviewClient } from './previewClient';
import {
  closePreview,
  ipcTransport,
  previewFrameUrl,
  schemeTransport,
  type DevelopSettingsJson,
  type FrameTransport,
  type SessionInfo,
} from './transport';

/** The result of one candidate. */
export interface BenchResult {
  readonly name: string;
  /** Frames measured. */
  readonly frames: number;
  /** Frames per second, end to end (request to drawn frame). */
  readonly fps: number;
  /** Mean time from request to received bytes, in milliseconds. */
  readonly transferMs: number;
  /** Mean time to decode and draw a frame, in milliseconds. */
  readonly drawMs: number;
  /** Mean render time in the shell, in milliseconds (0 if not known). */
  readonly shellMs: number;
  /** Mean body size in bytes. */
  readonly bytes: number;
  readonly note: string;
}

/** One measured frame. */
export interface FrameSample {
  readonly transferMs: number;
  readonly drawMs: number;
  readonly shellMs: number;
  readonly bytes: number;
}

/** Summarizes samples over the total elapsed time. */
export function summarize(
  name: string,
  samples: readonly FrameSample[],
  elapsedMs: number,
  note = '',
): BenchResult {
  const mean = (pick: (s: FrameSample) => number): number =>
    samples.length === 0 ? 0 : samples.reduce((sum, s) => sum + pick(s), 0) / samples.length;
  return {
    name,
    frames: samples.length,
    fps: elapsedMs > 0 ? (samples.length * 1000) / elapsedMs : 0,
    transferMs: mean((s) => s.transferMs),
    drawMs: mean((s) => s.drawMs),
    shellMs: mean((s) => s.shellMs),
    bytes: mean((s) => s.bytes),
    note,
  };
}

/** Formats results as a fixed-width text table. */
export function formatResults(results: readonly BenchResult[]): string {
  const header = ['candidate', 'frames', 'fps', 'transfer ms', 'draw ms', 'shell ms', 'MB/frame', 'note'];
  const rows = results.map((r) => [
    r.name,
    String(r.frames),
    r.fps.toFixed(1),
    r.transferMs.toFixed(2),
    r.drawMs.toFixed(2),
    r.shellMs.toFixed(2),
    (r.bytes / 1e6).toFixed(2),
    r.note,
  ]);
  const widths = header.map((h, i) => Math.max(h.length, ...rows.map((row) => (row[i] ?? '').length)));
  const line = (cells: readonly string[]): string => cells
      .map((c, i) => c.padEnd(widths[i] ?? 0))
      .join('  ')
      .trimEnd();
  return [line(header), line(widths.map((w) => '-'.repeat(w))), ...rows.map(line)].join('\n');
}

/** Settings for frame `i`: exposure steps, so that each frame is a new render. */
export function benchSettings(i: number): DevelopSettingsJson {
  return { tone: { exposure: (i % 100) * 0.02 } };
}

interface BenchContext {
  readonly session: SessionInfo;
  readonly ctx: CanvasRenderingContext2D;
  readonly frames: number;
}

const WARMUP_FRAMES = 3;

/** Runs `step` for warm-up and measured frames, one after the other. */
async function sequential(
  bench: BenchContext,
  name: string,
  step: (i: number) => Promise<FrameSample>,
  note = '',
): Promise<BenchResult> {
  for (let i = 0; i < WARMUP_FRAMES; i += 1) {
    await step(i);
  }
  const samples: FrameSample[] = [];
  const started = performance.now();
  for (let i = 0; i < bench.frames; i += 1) {
    samples.push(await step(WARMUP_FRAMES + i));
  }
  return summarize(name, samples, performance.now() - started, note);
}

/** Raw RGBA8 frames through a transport, drawn with `mode`. */
function rawCandidate(bench: BenchContext, transport: FrameTransport, mode: DrawMode): Promise<BenchResult> {
  return sequential(bench, `${transport.name}-rgba8-${mode}`, async (i) => {
    const t0 = performance.now();
    const body = await transport.render(bench.session.session, benchSettings(i));
    const t1 = performance.now();
    const frame = parseFrame(body);
    await drawFrame(bench.ctx, frame, mode);
    const t2 = performance.now();
    return { transferMs: t1 - t0, drawMs: t2 - t1, shellMs: frame.renderUs / 1000, bytes: body.byteLength };
  });
}

/** Reads the shell render time from the `X-Expose-Frame` header (`w,h,seq,queue_us,render_us`). */
export function shellMsFromHeader(value: string | null): number {
  const renderUs = Number(value?.split(',')[4]);
  return Number.isFinite(renderUs) ? renderUs / 1000 : 0;
}

/** Encoded frames (JPEG or PNG) by `fetch()`, decoded with `createImageBitmap`. */
function encodedFetchCandidate(bench: BenchContext, format: 'jpeg' | 'png'): Promise<BenchResult> {
  return sequential(
    bench,
    `scheme-${format}-bitmap`,
    async (i) => {
      const t0 = performance.now();
      const response = await fetch(previewFrameUrl(bench.session.session, format), {
        method: 'POST',
        headers: { 'Content-Type': 'text/plain' },
        body: JSON.stringify(benchSettings(i)),
      });
      if (!response.ok) {
        throw new Error(`HTTP ${response.status}: ${await response.text()}`);
      }
      const blob = await response.blob();
      const t1 = performance.now();
      const bitmap = await createImageBitmap(blob);
      bench.ctx.drawImage(bitmap, 0, 0);
      bitmap.close();
      const t2 = performance.now();
      return {
        transferMs: t1 - t0,
        drawMs: t2 - t1,
        shellMs: shellMsFromHeader(response.headers.get('x-expose-frame')),
        bytes: blob.size,
      };
    },
    encodedNote(format),
  );
}

/** The note of an encoded-frame candidate. */
export function encodedNote(format: 'jpeg' | 'png'): string {
  return format === 'jpeg' ? 'lossy at q85; shell encode on CPU' : 'lossless 8-bit; shell encode on CPU';
}

/** JPEG frames through an `<img>` element (GET; default settings, so the render is the same each time). */
function jpegImageCandidate(bench: BenchContext): Promise<BenchResult> {
  const image = new Image();
  return sequential(
    bench,
    'scheme-jpeg-img',
    async (i) => {
      const t0 = performance.now();
      image.src = `${previewFrameUrl(bench.session.session, 'jpeg')}&n=${i}`;
      await image.decode();
      const t1 = performance.now();
      bench.ctx.drawImage(image, 0, 0);
      const t2 = performance.now();
      return { transferMs: t1 - t0, drawMs: t2 - t1, shellMs: 0, bytes: 0 };
    },
    'transfer includes decode; GET has no settings',
  );
}

/** Frames pushed by the shell through a Tauri `Channel`, with no request per frame. */
async function channelCandidate(bench: BenchContext): Promise<BenchResult> {
  const samples: FrameSample[] = [];
  let last = performance.now();
  let failure: unknown = null;
  const channel = new Channel<unknown>((message) => {
    try {
      const t0 = performance.now();
      const body = message instanceof ArrayBuffer ? message : Uint8Array.from(message as number[]).buffer;
      const frame: Frame = parseFrame(body);
      bench.ctx.putImageData(new ImageData(frame.pixels, frame.width, frame.height), 0, 0);
      const t1 = performance.now();
      samples.push({ transferMs: t0 - last, drawMs: t1 - t0, shellMs: frame.renderUs / 1000, bytes: body.byteLength });
      last = t1;
    } catch (error: unknown) {
      failure = error;
    }
  });
  const started = performance.now();
  last = started;
  await invoke('preview_bench_channel', { session: bench.session.session, frames: bench.frames, channel });
  // Messages can arrive after the command result; wait a short time for them.
  const deadline = performance.now() + 2000;
  while (samples.length < bench.frames && performance.now() < deadline && failure === null) {
    await new Promise((resolve) => setTimeout(resolve, 10));
  }
  if (failure !== null) {
    throw failure;
  }
  return summarize('channel-rgba8-put', samples, performance.now() - started, 'push; transfer = gap between frames');
}

/** Waits until `done()` is true or `timeoutMs` has passed. */
async function waitFor(done: () => boolean, timeoutMs: number): Promise<void> {
  const deadline = performance.now() + timeoutMs;
  while (!done() && performance.now() < deadline) {
    await new Promise((resolve) => setTimeout(resolve, 5));
  }
}

/** The name suffix for a target long edge. */
function sizeSuffix(longEdge: number | null): string {
  return longEdge === null ? '' : `@${longEdge}`;
}

/** The mean of the draw times of a presenter. */
class DrawTimes {
  #sum = 0;
  #count = 0;
  add(ms: number): void {
    this.#sum += ms;
    this.#count += 1;
  }
  get mean(): number {
    return this.#count === 0 ? 0 : this.#sum / this.#count;
  }
}

/**
 * The production path as a stream: the client keeps two requests in
 * flight (render of frame N + 1 overlaps the transfer of frame N), and a
 * `FramePresenter` draws at most once per display frame. Each received
 * frame sends the next request at once, so the shell never waits for the
 * UI. `fps` counts received frames; the note gives the drawn frames (the
 * display refresh rate limits them).
 */
async function pipelinedCandidate(
  bench: BenchContext,
  transport: FrameTransport,
  mode: DrawMode,
  longEdge: number | null = null,
): Promise<BenchResult> {
  const total = WARMUP_FRAMES + bench.frames;
  const samples: FrameSample[] = [];
  const draws = new DrawTimes();
  let failure: unknown = null;
  let received = 0;
  let next = 0;
  let started = performance.now();
  let lastAt = started;
  const presenter = new FramePresenter({
    context: () => bench.ctx,
    mode,
    onPresented: (event) => draws.add(event.drawMs),
    onError: (error) => {
      failure = error;
    },
  });
  const sendNext = (): void => {
    if (next < total) {
      client.request(benchSettings(next));
      next += 1;
    }
  };
  const client = new PreviewClient({
    transport,
    maxInFlight: 2,
    onFrame: (event) => {
      received += 1;
      const now = performance.now();
      if (received === WARMUP_FRAMES) {
        started = now;
      } else if (received > WARMUP_FRAMES) {
        samples.push({
          transferMs: event.roundTripMs,
          drawMs: 0,
          shellMs: event.frame.renderUs / 1000,
          bytes: event.frame.pixels.byteLength + FRAME_HEADER_BYTES,
        });
        lastAt = now;
      }
      presenter.present(event);
      sendNext();
    },
    onError: (error) => {
      failure = error;
    },
  });
  client.setSession(bench.session.session);
  client.setTargetLongEdge(longEdge);
  try {
    // Restart the stream if it stops (for example, after a superseded
    // request), until all requests are sent.
    while (next < total && failure === null) {
      sendNext();
      sendNext();
      await client.idle();
    }
    await waitFor(() => presenter.idle, 1000);
  } finally {
    client.dispose();
    presenter.dispose();
  }
  if (failure !== null) {
    throw failure;
  }
  const c = client.stats;
  const result = summarize(
    `pipelined-${transport.name}-rgba8-${mode}${sizeSuffix(longEdge)}`,
    samples,
    lastAt - started,
    `drawn ${presenter.stats.presented}, superseded ${c.superseded}, dropped ${c.dropped}`,
  );
  return { ...result, drawMs: draws.mean };
}

/**
 * A slider simulation: `request()` every 4 ms for 2 s through the client
 * (latest wins). `pipelined` uses the production path (two requests in
 * flight, `FramePresenter` in `bitmap` mode); else one request in flight
 * and `putImageData` in the frame function (the path before pipelining).
 * The note gives the timer ticks: few ticks mean a busy main thread.
 */
async function sliderCandidate(
  bench: BenchContext,
  transport: FrameTransport,
  pipelined: boolean,
  longEdge: number | null = null,
): Promise<BenchResult> {
  const samples: FrameSample[] = [];
  const draws = new DrawTimes();
  const presenter = pipelined
    ? new FramePresenter({
        context: () => bench.ctx,
        onPresented: (event) => draws.add(event.drawMs),
        onError: (error) => console.error('bench slider draw failed', error),
      })
    : null;
  const client = new PreviewClient({
    transport,
    maxInFlight: pipelined ? 2 : 1,
    onFrame: (event) => {
      samples.push({
        transferMs: event.roundTripMs,
        drawMs: 0,
        shellMs: event.frame.renderUs / 1000,
        bytes: event.frame.pixels.byteLength + FRAME_HEADER_BYTES,
      });
      if (presenter !== null) {
        presenter.present(event);
        return;
      }
      const t0 = performance.now();
      bench.ctx.putImageData(new ImageData(event.frame.pixels, event.frame.width, event.frame.height), 0, 0);
      draws.add(performance.now() - t0);
    },
    onError: (error) => console.error('bench slider frame failed', error),
  });
  client.setSession(bench.session.session);
  client.setTargetLongEdge(longEdge);
  const started = performance.now();
  let ticks = 0;
  await new Promise<void>((resolve) => {
    const timer = setInterval(() => {
      client.request(benchSettings(ticks));
      ticks += 1;
      if (performance.now() - started >= 2000) {
        clearInterval(timer);
        resolve();
      }
    }, 4);
  });
  await client.idle();
  const elapsed = performance.now() - started;
  if (presenter !== null) {
    await waitFor(() => presenter.idle, 1000);
    presenter.dispose();
  }
  client.dispose();
  const s = client.stats;
  const drawn = presenter === null ? samples.length : presenter.stats.presented;
  const result = summarize(
    `slider-${transport.name}${pipelined ? '-pipelined' : ''}${sizeSuffix(longEdge)}`,
    samples,
    elapsed,
    `ticks ${ticks}, sent ${s.sent}, superseded ${s.superseded}, dropped ${s.dropped}, drawn ${drawn}`,
  );
  return { ...result, drawMs: draws.mean };
}

/** Runs all candidates and returns the results. `log` gets progress lines. */
export async function runTransportBench(options: {
  readonly canvas: HTMLCanvasElement;
  readonly frames?: number;
  readonly longEdge?: number;
  readonly log?: (line: string) => void;
}): Promise<BenchResult[]> {
  const log = options.log ?? ((line: string) => console.info(line));
  const ctx = frameContext(options.canvas);
  if (ctx === null) {
    throw new Error('The web view has no 2D canvas.');
  }
  log('Opening a 6000x4000 test image session...');
  const session = await invoke<SessionInfo>('preview_open_test_pattern', {
    width: 6000,
    height: 4000,
    maxLongEdge: options.longEdge ?? 2560,
    displaySpace: null,
  });
  log(`Session ${session.session}: preview ${session.width}x${session.height}.`);
  options.canvas.width = session.width;
  options.canvas.height = session.height;
  const bench: BenchContext = { session, ctx, frames: options.frames ?? 60 };
  const half = Math.round(Math.max(session.width, session.height) / 2);
  const candidates: Array<[string, () => Promise<BenchResult>]> = [
    // Production path: two requests in flight + presenter.
    ['pipelined-ipc-rgba8-bitmap', () => pipelinedCandidate(bench, ipcTransport, 'bitmap')],
    ['pipelined-ipc-rgba8-put', () => pipelinedCandidate(bench, ipcTransport, 'put')],
    ['pipelined-scheme-rgba8-bitmap', () => pipelinedCandidate(bench, schemeTransport, 'bitmap')],
    [`pipelined-ipc-rgba8-bitmap@${half}`, () => pipelinedCandidate(bench, ipcTransport, 'bitmap', half)],
    ['slider-ipc-pipelined', () => sliderCandidate(bench, ipcTransport, true)],
    [`slider-ipc-pipelined@${half}`, () => sliderCandidate(bench, ipcTransport, true, half)],
    // The paths before pipelining, for comparison.
    ['ipc-rgba8-put', () => rawCandidate(bench, ipcTransport, 'put')],
    ['ipc-rgba8-bitmap', () => rawCandidate(bench, ipcTransport, 'bitmap')],
    ['scheme-rgba8-put', () => rawCandidate(bench, schemeTransport, 'put')],
    ['scheme-rgba8-bitmap', () => rawCandidate(bench, schemeTransport, 'bitmap')],
    ['channel-rgba8-put', () => channelCandidate(bench)],
    ['slider-ipc', () => sliderCandidate(bench, ipcTransport, false)],
    ['slider-scheme', () => sliderCandidate(bench, schemeTransport, false)],
    // Encoded frames (slow in a debug build of the shell).
    ['scheme-jpeg-bitmap', () => encodedFetchCandidate(bench, 'jpeg')],
    ['scheme-png-bitmap', () => encodedFetchCandidate(bench, 'png')],
    ['scheme-jpeg-img', () => jpegImageCandidate(bench)],
  ];
  const results: BenchResult[] = [];
  try {
    for (const [name, run] of candidates) {
      log(`Running ${name}...`);
      try {
        results.push(await run());
      } catch (error: unknown) {
        console.error(`bench candidate ${name} failed`, error);
        results.push(summarize(name, [], 0, `FAILED: ${describeError(error)}`));
      }
    }
  } finally {
    await closePreview(session.session);
  }
  console.table(results);
  log(formatResults(results));
  return results;
}
