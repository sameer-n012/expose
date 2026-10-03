<!--
  The virtualized photo grid of the Library module.

  All rows have the same height. The grid renders only the rows in (and
  near) the viewport, so it stays fast with 50 000 photos. When the user
  scrolls, it asks the library store for the pages that cover the visible
  positions.

  Pointer: click selects, Cmd/Ctrl+click toggles, Shift+click selects a
  range, double-click opens the loupe view. Keyboard selection and the
  rating keys are shortcut actions (see libraryActions.ts).
-->
<script lang="ts">
  import { untrack } from 'svelte';
  import { contentHeight, GRID_CONFIG, gridMetrics, scrollTopToReveal, visibleRows } from '../../lib/library/gridLayout';
  import type { LibraryStore } from '../../lib/stores/library.svelte';
  import PhotoCell from './PhotoCell.svelte';

  interface Props {
    library: LibraryStore;
    /** Called when the user double-clicks the photo at a grid position. */
    onopen: (index: number) => void;
  }

  let { library, onopen }: Props = $props();

  let viewport = $state<HTMLDivElement>();
  let width = $state(0);
  let height = $state(0);
  let scrollTop = $state(0);

  const metrics = $derived(gridMetrics(width));
  const rows = $derived(visibleRows(scrollTop, height, library.total, metrics));
  /** Grid positions of the cells to render. */
  const indices = $derived.by(() => {
    if (rows === null) {
      return [];
    }
    const first = rows.first * metrics.columns;
    const end = Math.min((rows.last + 1) * metrics.columns, library.total);
    return Array.from({ length: Math.max(end - first, 0) }, (_, i) => first + i);
  });
  /** The one cell in the tab order: the focused cell if it is rendered, else the first cell. */
  const tabIndex = $derived.by(() => {
    const focus = library.focusIndex;
    const first = indices[0];
    const last = indices[indices.length - 1];
    if (focus !== null && first !== undefined && last !== undefined && focus >= first && focus <= last) {
      return focus;
    }
    return first ?? -1;
  });

  // Measure the viewport. The size is 0 while the module is hidden; the
  // observer reports the real size when the module shows.
  $effect(() => {
    const el = viewport;
    if (el === undefined) {
      return;
    }
    const measure = (): void => {
      width = el.clientWidth;
      height = el.clientHeight;
    };
    measure();
    if (typeof ResizeObserver === 'undefined') {
      return;
    }
    const observer = new ResizeObserver(measure);
    observer.observe(el);
    return () => observer.disconnect();
  });

  $effect(() => {
    library.columns = metrics.columns;
  });

  // Load the pages of the visible rows, and the rows after them.
  $effect(() => {
    if (rows === null) {
      return;
    }
    const columns = metrics.columns;
    const start = rows.first * columns;
    const end = (rows.last + 3) * columns - 1;
    // `ensureRange` reads store state. Only the visible rows must trigger this effect.
    untrack(() => void library.ensureRange(start, end));
  });

  // Scroll to the focused cell when the keyboard moves the focus.
  $effect(() => {
    const focus = library.focusIndex;
    if (focus === null || viewport === undefined) {
      return;
    }
    untrack(() => {
      if (viewport === undefined) {
        return;
      }
      const next = scrollTopToReveal(Math.floor(focus / metrics.columns), viewport.scrollTop, height, metrics);
      if (next !== viewport.scrollTop) {
        viewport.scrollTop = next;
        scrollTop = next;
      }
    });
  });

  function cellStyle(index: number): string {
    const row = Math.floor(index / metrics.columns);
    const column = index % metrics.columns;
    const top = GRID_CONFIG.gap + row * metrics.rowHeight;
    const left = GRID_CONFIG.gap + column * (metrics.cellWidth + GRID_CONFIG.gap);
    return `top:${top}px;left:${left}px;width:${metrics.cellWidth}px;height:${metrics.cellHeight}px`;
  }

  function onscroll(event: Event): void {
    scrollTop = (event.currentTarget as HTMLElement).scrollTop;
  }

  function emptyText(): string {
    if (library.folderId === null) {
      return 'The catalog has no photos. Use "Import folder" to add some.';
    }
    return 'No photos match.';
  }
</script>

{#if library.loadFailed}
  <div class="banner" role="alert">
    <span>Some photos could not be loaded.</span>
    <button type="button" onclick={() => void library.retry()}>Retry</button>
  </div>
{/if}

<div class="viewport" bind:this={viewport} {onscroll} data-testid="library-grid" role="group" aria-label="Photos">
  <div class="content" style:height="{contentHeight(library.total, metrics)}px">
    {#each indices as index (index)}
      {@const photo = library.photoAt(index)}
      <PhotoCell
        {photo}
        {index}
        selected={photo !== undefined && library.isSelected(photo)}
        tabbable={index === tabIndex}
        style={cellStyle(index)}
        onclick={(event) => library.click(index, { toggle: event.metaKey || event.ctrlKey, range: event.shiftKey })}
        ondblclick={() => onopen(index)}
      />
    {/each}
  </div>
  {#if library.total === 0 && !library.loading && !library.loadFailed}
    <p class="empty">{emptyText()}</p>
  {/if}
</div>

<style>
  .viewport {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    position: relative;
  }
  .content {
    position: relative;
  }
  .empty {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    margin: 0;
    color: var(--color-text-muted);
    pointer-events: none;
  }
  .banner {
    display: flex;
    align-items: center;
    gap: var(--space-4);
    padding: var(--space-3) var(--space-4);
    background: var(--color-bg-panel-raised);
    color: var(--color-danger);
    font-size: var(--font-size-sm);
  }
  .banner button {
    background: var(--color-bg-hover);
    border: 1px solid var(--color-border-strong);
    border-radius: var(--radius-sm);
    padding: var(--space-1) var(--space-3);
    cursor: pointer;
  }
</style>
