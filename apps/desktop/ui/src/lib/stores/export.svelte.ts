/**
 * State of the export: the dialog (its photos and form) and the job that
 * runs or has finished.
 *
 * - `openDialog(ids)` opens the dialog. With no photo, it shows a toast.
 * - `submit()` starts a job with the form. The job sends events; the store
 *   turns them into counters and a list of failures.
 * - The shell allows one job at a time. If the shell rejects a start, the
 *   store shows its message as an error toast.
 */
import { pickFolder, type PhotoId } from '../catalog';
import { cancelExport, startExport } from '../export/api';
import { buildRequest, defaultForm, loadForm, saveForm, type ExportForm } from '../export/options';
import type { ExportEvent, ExportRequest } from '../export/types';
import { validateForm, type FormErrors } from '../export/validate';
import { describeError, type ToastStore } from './toasts.svelte';

export interface ExportApi {
  start(request: ExportRequest, onEvent: (event: ExportEvent) => void): Promise<number>;
  cancel(jobId: number): Promise<void>;
}

export interface ExportFailure {
  readonly photoId: PhotoId;
  readonly message: string;
}

export interface ExportStoreOptions {
  readonly toasts: Pick<ToastStore, 'error' | 'info'>;
  readonly api?: ExportApi;
  /** Chooses the destination folder. Default: the native folder dialog. */
  readonly pickFolder?: () => Promise<string | null>;
  /** Loads the form at the start. Default: the stored options. */
  readonly loadForm?: () => ExportForm;
  /** Stores the form after a start. Default: browser storage. */
  readonly saveForm?: (form: ExportForm) => void;
}

export type JobPhase = 'idle' | 'starting' | 'running' | 'finished';

const REAL_API: ExportApi = { start: startExport, cancel: cancelExport };

export class ExportStore {
  /** The photos of the open dialog, or null if the dialog is closed. */
  dialogIds = $state<PhotoId[] | null>(null);
  form = $state<ExportForm>(defaultForm());

  phase = $state<JobPhase>('idle');
  jobId = $state<number | null>(null);
  total = $state(0);
  written = $state(0);
  skipped = $state(0);
  failures = $state<ExportFailure[]>([]);
  cancelled = $state(false);
  /** True after the user asks to cancel and before the job finishes. */
  cancelling = $state(false);

  readonly #toasts: ExportStoreOptions['toasts'];
  readonly #api: ExportApi;
  readonly #pick: () => Promise<string | null>;
  readonly #save: (form: ExportForm) => void;
  /** True if the user cancelled before the start call answered. */
  #cancelWanted = false;

  constructor(options: ExportStoreOptions) {
    this.#toasts = options.toasts;
    this.#api = options.api ?? REAL_API;
    this.#pick = options.pickFolder ?? pickFolder;
    this.#save = options.saveForm ?? ((form) => saveForm(form));
    this.form = (options.loadForm ?? loadForm)();
  }

  get isOpen(): boolean {
    return this.dialogIds !== null;
  }

  /** Photos that the job has handled so far. */
  get done(): number {
    return this.written + this.skipped + this.failures.length;
  }

  /** True while a job starts or runs. */
  get busy(): boolean {
    return this.phase === 'starting' || this.phase === 'running';
  }

  /** The error text per field for the open dialog. */
  get errors(): FormErrors {
    return validateForm(this.form, this.dialogIds?.length ?? 0);
  }

  /** Opens the dialog for the photos. With no photo, shows a toast instead. */
  openDialog(ids: readonly PhotoId[]): void {
    if (ids.length === 0) {
      this.#toasts.info('Select at least one photo to export.');
      return;
    }
    if (this.busy) {
      this.#toasts.info('An export is running. Wait until it finishes.');
      return;
    }
    this.dialogIds = [...ids];
  }

  closeDialog(): void {
    this.dialogIds = null;
  }

  /** Lets the user choose the destination folder. */
  async chooseDestination(): Promise<void> {
    try {
      const path = await this.#pick();
      if (path !== null) {
        this.form.destination = path;
      }
    } catch (error: unknown) {
      this.#toasts.error('Choosing the folder failed.', error);
    }
  }

  /**
   * Starts the job with the form and the photos of the dialog. It does
   * nothing if the form is not valid. The dialog closes when the job starts.
   */
  async submit(): Promise<void> {
    const ids = this.dialogIds;
    if (ids === null || this.busy || Object.keys(this.errors).length > 0) {
      return;
    }
    const request = buildRequest(this.form, ids);
    this.#reset();
    this.phase = 'starting';
    try {
      const jobId = await this.#api.start(request, (event) => this.#onEvent(event));
      this.#adoptJob(jobId);
    } catch (error: unknown) {
      // Show the message of the shell. The dialog stays open for a fix.
      this.phase = 'idle';
      const message = error instanceof Object && 'message' in error ? String(error.message) : describeError(error);
      this.#toasts.error(`The export did not start: ${message}`, error);
      return;
    }
    this.#save(this.form);
    this.closeDialog();
    if (this.#cancelWanted) {
      this.#cancelWanted = false;
      void this.#sendCancel();
    }
  }

  /** Asks the job to stop. Photos that are done stay on disk. */
  async cancel(): Promise<void> {
    if (!this.busy || this.cancelling) {
      return;
    }
    this.cancelling = true;
    if (this.jobId === null) {
      // The start call has not answered yet. Cancel when the id is known.
      this.#cancelWanted = true;
      return;
    }
    await this.#sendCancel();
  }

  /** Hides the summary of a finished job. */
  dismiss(): void {
    if (this.phase === 'finished') {
      this.phase = 'idle';
    }
  }

  async #sendCancel(): Promise<void> {
    if (this.jobId === null) {
      return;
    }
    try {
      await this.#api.cancel(this.jobId);
    } catch (error: unknown) {
      this.cancelling = false;
      this.#toasts.error('Cancelling the export failed.', error);
    }
  }

  #reset(): void {
    this.jobId = null;
    this.total = 0;
    this.written = 0;
    this.skipped = 0;
    this.failures = [];
    this.cancelled = false;
    this.cancelling = false;
    this.#cancelWanted = false;
  }

  #adoptJob(jobId: number): void {
    // A `started` event can come before the start call answers.
    if (this.jobId === null) {
      this.jobId = jobId;
    }
  }

  #onEvent(event: ExportEvent): void {
    if (event.kind === 'started') {
      if (this.jobId === null) {
        this.jobId = event.jobId;
      }
    } else if (this.jobId !== null && event.jobId !== this.jobId) {
      return;
    }
    if (this.phase === 'idle' || this.phase === 'finished') {
      return;
    }
    switch (event.kind) {
      case 'started':
        this.total = event.total;
        this.phase = 'running';
        break;
      case 'photoDone':
        this.written += 1;
        break;
      case 'photoSkipped':
        this.skipped += 1;
        break;
      case 'photoFailed':
        this.failures.push({ photoId: event.photoId, message: event.message });
        break;
      case 'finished':
        // The counters of the backend are the truth. Take them over.
        this.written = event.written;
        this.skipped = event.skipped;
        this.cancelled = event.cancelled;
        this.cancelling = false;
        this.phase = 'finished';
        break;
    }
  }
}
