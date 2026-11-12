/**
 * The Gaussian blur and unsharp mask filters of the Editor (ADR 0031).
 *
 * - The dialog values use UI units (the amount in percent). `filterDto`
 *   makes the shell values (the amount as a fraction).
 * - `FilterClient` is the transport of one dialog: one `filterPreview` call
 *   in flight, the newest waiting value, then commit or cancel.
 */
import type { DocumentInfo, EditorApi, FilterDto, FilterTarget } from './api';

export type FilterKind = 'gaussianBlur' | 'unsharpMask';

/** Limits of the values. The shell checks the same limits. */
export const MIN_FILTER_RADIUS = 0.1;
export const MAX_FILTER_RADIUS = 1000;
/** The amount in percent (the shell takes a fraction: 0.01 to 5). */
export const MIN_AMOUNT_PERCENT = 1;
export const MAX_AMOUNT_PERCENT = 500;
export const MIN_THRESHOLD = 0;
export const MAX_THRESHOLD = 255;

export interface GaussianBlurValues {
  radius: number;
}

export interface UnsharpMaskValues {
  amountPercent: number;
  radius: number;
  threshold: number;
}

/** The last values of each filter. */
export interface FilterValues {
  gaussianBlur: GaussianBlurValues;
  unsharpMask: UnsharpMaskValues;
}

export function defaultFilterValues(): FilterValues {
  return {
    gaussianBlur: { radius: 2 },
    unsharpMask: { amountPercent: 100, radius: 1, threshold: 0 },
  };
}

/** The menu label and the dialog title of a filter. */
export const FILTER_LABEL: Record<FilterKind, string> = {
  gaussianBlur: 'Gaussian Blur',
  unsharpMask: 'Unsharp Mask',
};

const clamp = (value: number, min: number, max: number): number => Math.min(max, Math.max(min, value));

/** The radius inside the limits, rounded to 0.1 pixel. */
export function clampRadius(radius: number): number {
  return clamp(Math.round(radius * 10) / 10, MIN_FILTER_RADIUS, MAX_FILTER_RADIUS);
}

export function clampAmountPercent(percent: number): number {
  return clamp(Math.round(percent), MIN_AMOUNT_PERCENT, MAX_AMOUNT_PERCENT);
}

export function clampThreshold(threshold: number): number {
  return clamp(Math.round(threshold), MIN_THRESHOLD, MAX_THRESHOLD);
}

/** The slider position (0 to 1) of a radius on the logarithmic scale. */
export function radiusToSlider(radius: number): number {
  const r = clamp(radius, MIN_FILTER_RADIUS, MAX_FILTER_RADIUS);
  return Math.log(r / MIN_FILTER_RADIUS) / Math.log(MAX_FILTER_RADIUS / MIN_FILTER_RADIUS);
}

/** The radius of a slider position (0 to 1) on the logarithmic scale. */
export function sliderToRadius(position: number): number {
  const p = clamp(position, 0, 1);
  return clampRadius(MIN_FILTER_RADIUS * Math.pow(MAX_FILTER_RADIUS / MIN_FILTER_RADIUS, p));
}

/** The shell filter of a dialog state. */
export function filterDto(kind: FilterKind, values: FilterValues): FilterDto {
  if (kind === 'gaussianBlur') {
    return { type: 'gaussianBlur', radius: clampRadius(values.gaussianBlur.radius) };
  }
  const v = values.unsharpMask;
  return {
    type: 'unsharpMask',
    amount: clampAmountPercent(v.amountPercent) / 100,
    radius: clampRadius(v.radius),
    threshold: clampThreshold(v.threshold),
  };
}

export interface FilterClientOptions {
  readonly api: Pick<EditorApi, 'filterPreview' | 'filterCommit' | 'filterCancel'>;
  readonly documentId: number;
  readonly layerId: number;
  readonly target: FilterTarget;
  /** Called with each new document info (after a preview, a commit, or a cancel). The view asks for a new frame. */
  readonly onInfo: (info: DocumentInfo) => void;
  /** Called for each failed call. The owner closes the dialog. */
  readonly onError: (error: unknown) => void;
}

const same = (a: FilterDto | null, b: FilterDto): boolean => a !== null && JSON.stringify(a) === JSON.stringify(b);

export class FilterClient {
  readonly #options: FilterClientOptions;
  #inFlight: Promise<void> | null = null;
  #waiting: FilterDto | null = null;
  /** The filter of the newest preview that the shell answered. */
  #applied: FilterDto | null = null;
  /** True while the shell can have a filter session. */
  #session = false;
  #committing = false;
  #closed = false;

  constructor(options: FilterClientOptions) {
    this.#options = options;
  }

  /** True if the shell can have a filter session for this client. */
  get sessionActive(): boolean {
    return this.#session;
  }

  /**
   * Shows the filter. At most one call is in flight. A value that comes
   * while a call runs waits; a newer waiting value replaces it.
   */
  preview(filter: FilterDto): void {
    if (this.#closed || this.#committing) {
      return;
    }
    if (this.#inFlight !== null) {
      this.#waiting = filter;
      return;
    }
    this.#send(filter);
  }

  #send(filter: FilterDto): void {
    const { api, documentId, layerId, target, onInfo, onError } = this.#options;
    this.#session = true;
    const run = async (): Promise<void> => {
      try {
        const info = await api.filterPreview(documentId, layerId, target, filter);
        if (this.#closed) {
          return;
        }
        this.#applied = filter;
        onInfo(info);
      } catch (error: unknown) {
        this.#applied = null;
        this.#waiting = null;
        if (!this.#closed) {
          onError(error);
        }
      }
    };
    this.#inFlight = run().finally(() => {
      this.#inFlight = null;
      const next = this.#waiting;
      this.#waiting = null;
      if (next !== null && !this.#closed && !this.#committing) {
        this.#send(next);
      }
    });
  }

  /** Ends the session without a history step (the "Preview" box is off). The client can preview again. */
  async cancelSession(): Promise<void> {
    this.#waiting = null;
    await this.#inFlight;
    if (!this.#session || this.#committing) {
      return;
    }
    this.#session = false;
    this.#applied = null;
    try {
      this.#options.onInfo(await this.#options.api.filterCancel(this.#options.documentId));
    } catch (error: unknown) {
      this.#options.onError(error);
    }
  }

  /** Ends the client: no more previews, and the session ends (Cancel). */
  async dispose(): Promise<void> {
    this.#closed = true;
    await this.cancelSession();
  }

  /** Ends the client without a call. Another edit already ended the session in the shell. */
  abandon(): void {
    this.#closed = true;
    this.#waiting = null;
    this.#session = false;
    this.#applied = null;
  }

  /**
   * OK. Waits for the call in flight. If the last preview shows `filter`,
   * the shell keeps it; else the filter is previewed first. Returns true if
   * the filter is committed. A failed call goes to `onError`.
   */
  async commit(filter: FilterDto): Promise<boolean> {
    if (this.#closed || this.#committing) {
      return false;
    }
    this.#committing = true;
    this.#waiting = null;
    const { api, documentId, layerId, target, onInfo, onError } = this.#options;
    try {
      await this.#inFlight;
      if (this.#closed) {
        return false;
      }
      if (!(this.#session && same(this.#applied, filter))) {
        this.#session = true;
        onInfo(await api.filterPreview(documentId, layerId, target, filter));
        this.#applied = filter;
      }
      const info = await api.filterCommit(documentId);
      this.#session = false;
      this.#applied = null;
      this.#closed = true;
      onInfo(info);
      return true;
    } catch (error: unknown) {
      this.#committing = false;
      onError(error);
      return false;
    }
  }
}
