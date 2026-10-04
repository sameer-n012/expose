/** DOM tests of the export dialog: options per format, validation, and submit. */
import { fireEvent, render, screen } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';
import { defaultForm } from '../../lib/export/options';
import type { ExportRequest } from '../../lib/export/types';
import { ExportStore, type ExportApi } from '../../lib/stores/export.svelte';
import { ToastStore } from '../../lib/stores/toasts.svelte';
import ExportDialog from './ExportDialog.svelte';

function setup(destination = '/out') {
  const start = vi.fn(async (_r: ExportRequest) => 5);
  const store = new ExportStore({
    toasts: new ToastStore(),
    api: { start, cancel: async () => undefined } as unknown as ExportApi,
    loadForm: () => ({ ...defaultForm(), destination }),
    saveForm: () => undefined,
  });
  store.openDialog([4, 5]);
  render(ExportDialog, { store });
  return { store, start };
}

const exportButton = () => screen.getByRole('button', { name: 'Export' }) as HTMLButtonElement;

describe('ExportDialog', () => {
  it('is a modal dialog that shows the photo count', () => {
    setup();
    expect(screen.getByRole('dialog').getAttribute('aria-modal')).toBe('true');
    expect(screen.getByTestId('export-count').textContent?.trim()).toBe('2 photos');
  });

  it('shows the options of the selected format', async () => {
    setup();
    expect(screen.getByRole('spinbutton', { name: 'Quality' })).toBeTruthy();
    expect(screen.getByText('Chroma subsampling')).toBeTruthy();
    expect(screen.getByLabelText(/Dither/)).toBeTruthy();

    await fireEvent.change(screen.getByLabelText('File format'), { target: { value: 'png' } });
    expect(screen.queryByRole('spinbutton', { name: 'Quality' })).toBeNull();
    expect(screen.getByLabelText('Bit depth')).toBeTruthy();
    expect(screen.queryByLabelText('Compression')).toBeNull();

    await fireEvent.change(screen.getByLabelText('File format'), { target: { value: 'tiff' } });
    expect(screen.getByLabelText('Compression')).toBeTruthy();
    // TIFF defaults to 16 bit, so there is no dither option.
    expect(screen.queryByLabelText(/Dither/)).toBeNull();
    await fireEvent.change(screen.getByLabelText('Bit depth'), { target: { value: '8' } });
    expect(screen.getByLabelText(/Dither/)).toBeTruthy();
  });

  it('disables Export while the form is not valid', async () => {
    setup('');
    expect(exportButton().disabled).toBe(true);
    expect(screen.getByText('Choose a destination folder.')).toBeTruthy();
  });

  it('disables Export for a bad quality', async () => {
    setup();
    expect(exportButton().disabled).toBe(false);
    await fireEvent.input(screen.getByRole('spinbutton', { name: 'Quality' }), { target: { value: '150' } });
    expect(exportButton().disabled).toBe(true);
  });

  it('shows the size value only for a size mode, and checks upscaling', async () => {
    setup();
    expect(screen.queryByRole('spinbutton', { name: 'Size value' })).toBeNull();
    await fireEvent.change(screen.getByLabelText('Resize to'), { target: { value: 'percent' } });
    await fireEvent.input(screen.getByRole('spinbutton', { name: 'Size value' }), { target: { value: '200' } });
    expect(exportButton().disabled).toBe(true);
    await fireEvent.click(screen.getByLabelText('Allow upscaling'));
    expect(exportButton().disabled).toBe(false);
  });

  it('submits the request that the form describes', async () => {
    const { start } = setup();
    await fireEvent.change(screen.getByLabelText('File format'), { target: { value: 'tiff' } });
    await fireEvent.change(screen.getByLabelText('Compression'), { target: { value: 'deflate' } });
    await fireEvent.change(screen.getByLabelText('Color space'), { target: { value: 'display_p3' } });
    await fireEvent.change(screen.getByLabelText('Resize to'), { target: { value: 'longEdge' } });
    await fireEvent.input(screen.getByRole('spinbutton', { name: 'Size value' }), { target: { value: '1600' } });
    await fireEvent.change(screen.getByLabelText('File names'), { target: { value: 'suffix' } });
    await fireEvent.click(exportButton());
    await vi.waitFor(() => expect(start).toHaveBeenCalledOnce());
    expect(start.mock.calls[0]?.[0]).toEqual({
      photoIds: [4, 5],
      destination: '/out',
      format: { kind: 'tiff', bitDepth: 16, compression: 'deflate' },
      colorSpace: 'display_p3',
      size: { mode: 'longEdge', pixels: 1600 },
      allowUpscale: false,
      naming: { mode: 'suffix', suffix: '-export' },
      onConflict: 'uniqueName',
      dither: false,
    });
  });

  it('closes on Escape', async () => {
    const { store } = setup();
    await fireEvent.keyDown(screen.getByRole('dialog'), { key: 'Escape' });
    expect(store.isOpen).toBe(false);
  });
});
