/** Typed wrappers for the export commands. */
import { Channel, invoke } from '@tauri-apps/api/core';
import type { ExportEvent, ExportRequest } from './types';

/**
 * Starts an export job. The backend sends progress events through a channel.
 *
 * @returns the job id. Rejects with a `CommandError` for a bad request, or
 * when a job already runs.
 */
export function startExport(request: ExportRequest, onEvent: (event: ExportEvent) => void): Promise<number> {
  const channel = new Channel<ExportEvent>();
  channel.onmessage = onEvent;
  return invoke<number>('export_start', { request, onEvent: channel });
}

/** Asks the backend to stop a job. The job sends a `finished` event with `cancelled: true`. */
export function cancelExport(jobId: number): Promise<void> {
  return invoke<void>('export_cancel', { jobId });
}
