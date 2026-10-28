// @vitest-environment happy-dom
import { fireEvent, render, screen } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';
import type { NewDocumentSpec } from '../../lib/editor/api';
import NewDocumentDialog from './NewDocumentDialog.svelte';

function fakeEditor() {
  return {
    suggestedName: 'Untitled-4',
    closeNewDocument: vi.fn(),
    newDocument: vi.fn(async (_spec: NewDocumentSpec) => true),
  };
}

describe('NewDocumentDialog', () => {
  it('starts with the suggested name and a valid default', () => {
    const editor = fakeEditor();
    render(NewDocumentDialog, { editor: editor as never });
    expect(screen.getByRole<HTMLInputElement>('textbox', { name: 'Name' }).value).toBe('Untitled-4');
    expect(screen.getByRole<HTMLButtonElement>('button', { name: 'Create' }).disabled).toBe(false);
  });

  it('applies a preset and creates the document', async () => {
    const editor = fakeEditor();
    render(NewDocumentDialog, { editor: editor as never });
    await fireEvent.click(screen.getByRole('button', { name: 'A4 300 ppi' }));
    await fireEvent.click(screen.getByRole('button', { name: 'Create' }));
    expect(editor.closeNewDocument).toHaveBeenCalled();
    expect(editor.newDocument).toHaveBeenCalledWith(expect.objectContaining({ name: 'Untitled-4', width: 2480, height: 3508, resolutionPpi: 300 }));
  });

  it('disables Create and shows the message for an invalid value', async () => {
    const editor = fakeEditor();
    render(NewDocumentDialog, { editor: editor as never });
    const width = screen.getByLabelText<HTMLInputElement>('Width (px)');
    width.value = '0';
    await fireEvent.input(width);
    expect(screen.getByRole<HTMLButtonElement>('button', { name: 'Create' }).disabled).toBe(true);
    expect(screen.getByTestId('newdoc-problem').textContent).toContain('width');
  });

  it('cancels on Escape', async () => {
    const editor = fakeEditor();
    render(NewDocumentDialog, { editor: editor as never });
    await fireEvent.keyDown(screen.getByRole('dialog'), { key: 'Escape' });
    expect(editor.closeNewDocument).toHaveBeenCalled();
    expect(editor.newDocument).not.toHaveBeenCalled();
  });
});
