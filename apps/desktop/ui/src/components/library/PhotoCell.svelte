<!--
  One cell of the library grid: thumbnail (or placeholder), file name,
  rating, flag, color label, and capture date.
  A photo that is not loaded yet shows an empty skeleton cell.
-->
<script lang="ts">
  import type { Photo } from '../../lib/catalog';
  import { formatCaptureTime } from '../../lib/library/dateFormat';
  import { thumbnailUrl } from '../../lib/library/thumbnail';

  interface Props {
    /** The photo, or undefined while its page loads. */
    photo: Photo | undefined;
    /** Grid position. Used for the label of a cell that is not loaded. */
    index: number;
    selected: boolean;
    /** True for the one cell in the tab order. */
    tabbable: boolean;
    /** CSS for the position and size. */
    style: string;
    onclick: (event: MouseEvent) => void;
    ondblclick: () => void;
  }

  let { photo, index, selected, tabbable, style, onclick, ondblclick }: Props = $props();

  const FLAG_GLYPH = { none: '', pick: '⚑', reject: '⊘' } as const;
  const FLAG_TEXT = { none: '', pick: 'Pick', reject: 'Reject' } as const;

  const url = $derived(photo === undefined ? null : thumbnailUrl(photo));
  const stars = $derived(photo === undefined ? '' : '★'.repeat(photo.rating) + '☆'.repeat(5 - photo.rating));
</script>

{#if photo === undefined}
  <div class="cell skeleton" {style} aria-hidden="true" data-index={index}></div>
{:else}
  <button
    type="button"
    class="cell"
    class:selected
    {style}
    tabindex={tabbable ? 0 : -1}
    aria-pressed={selected}
    aria-label={photo.fileName}
    data-index={index}
    data-photo-id={photo.id}
    {onclick}
    {ondblclick}
  >
    <span class="thumb">
      {#if url !== null}
        <img src={url} alt="" loading="lazy" draggable="false" />
      {/if}
      {#if photo.flag !== 'none'}
        <span class="flag {photo.flag}" title={FLAG_TEXT[photo.flag]} data-testid="flag">
          {FLAG_GLYPH[photo.flag]}
        </span>
      {/if}
      {#if photo.colorLabel !== 'none'}
        <span class="label {photo.colorLabel}" title="Label: {photo.colorLabel}" data-testid="color-label"></span>
      {/if}
    </span>
    <span class="info">
      <span class="name" title={photo.fileName}>{photo.fileName}</span>
      <span class="stars" title="{photo.rating} of 5 stars" data-testid="rating">{stars}</span>
      <span class="date">{formatCaptureTime(photo.captureTimeMs)}</span>
    </span>
  </button>
{/if}

<style>
  .cell {
    position: absolute;
    display: flex;
    flex-direction: column;
    padding: 0;
    overflow: hidden;
    text-align: left;
    background: var(--color-bg-panel);
    border: 2px solid transparent;
    border-radius: var(--radius-sm);
    cursor: pointer;
  }
  .skeleton {
    background: var(--color-bg-panel);
    cursor: default;
  }
  .cell:hover {
    border-color: var(--color-border-strong);
  }
  .cell.selected {
    border-color: var(--color-accent);
    background: var(--color-bg-active);
  }
  .thumb {
    position: relative;
    flex: 1;
    min-height: 0;
    background: var(--color-bg-placeholder);
  }
  .thumb img {
    width: 100%;
    height: 100%;
    object-fit: contain;
    display: block;
  }
  .flag {
    position: absolute;
    top: var(--space-2);
    left: var(--space-2);
    font-size: var(--font-size-lg);
    line-height: 1;
  }
  .flag.pick {
    color: var(--color-success);
  }
  .flag.reject {
    color: var(--color-danger);
  }
  .label {
    position: absolute;
    top: var(--space-2);
    right: var(--space-2);
    width: 12px;
    height: 12px;
    border-radius: 50%;
    border: 1px solid var(--color-bg-app);
  }
  .info {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: var(--space-1) var(--space-3);
    padding: var(--space-2) var(--space-3);
    font-size: var(--font-size-sm);
  }
  .name {
    grid-column: 1 / -1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--color-text);
  }
  .stars {
    color: var(--color-text-muted);
    letter-spacing: 1px;
  }
  .date {
    color: var(--color-text-muted);
    font-size: var(--font-size-xs);
    align-self: center;
    justify-self: end;
  }
  .label.red {
    background: var(--color-label-red);
  }
  .label.yellow {
    background: var(--color-label-yellow);
  }
  .label.green {
    background: var(--color-label-green);
  }
  .label.blue {
    background: var(--color-label-blue);
  }
  .label.purple {
    background: var(--color-label-purple);
  }
</style>
