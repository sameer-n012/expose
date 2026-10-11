// @vitest-environment happy-dom
import { render, screen } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';
import { makePhoto } from '../../test/fakeCatalog';
import PhotoCell from './PhotoCell.svelte';

function cell(isEditorOutput: boolean) {
  return render(PhotoCell, {
    photo: makePhoto(3, { isEditorOutput }),
    index: 0,
    selected: false,
    tabbable: true,
    style: '',
    onclick: vi.fn(),
    ondblclick: vi.fn(),
  });
}

describe('PhotoCell', () => {
  it('shows the Editor badge on a photo saved from the Editor', () => {
    cell(true);
    expect(screen.getByTestId('editor-badge').textContent?.trim()).toBe('Layers');
  });

  it('shows no badge on other photos', () => {
    cell(false);
    expect(screen.queryByTestId('editor-badge')).toBeNull();
  });
});
