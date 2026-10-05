<!--
  Center view of the Library module.

  First it reads the catalog status. If the catalog is not available, the
  view shows the error instead of the grid. Else it shows the toolbar and
  the photo grid. A double-click on a cell opens the loupe view (one large
  photo; a placeholder until the preview exists).
-->
<script lang="ts">
  import { formatCaptureTime } from '../../lib/library/dateFormat';
  import type { LibraryStore } from '../../lib/stores/library.svelte';
  import { canOpenLoupe, openLoupeAt, openLoupeOnSelection } from '../../lib/library/libraryNavigation';
  import type { LibraryViewMode, ModuleStore } from '../../lib/stores/modules.svelte';
  import LibraryGrid from '../library/LibraryGrid.svelte';
  import LibraryToolbar from '../library/LibraryToolbar.svelte';

  interface Props {
    modules: ModuleStore;
    library: LibraryStore;
    /** Opens the export dialog for the selected photos. */
    onexport?: () => void;
  }

  let { modules, library, onexport }: Props = $props();

  // The loupe shows the photo at the focus of the selection (see libraryNavigation.ts).
  const loupePhoto = $derived(library.focusIndex === null ? undefined : library.photoAt(library.focusIndex));

  $effect(() => {
    void library.init();
  });

  function setMode(mode: LibraryViewMode): void {
    if (mode === 'grid') {
      modules.libraryView = 'grid';
    } else {
      openLoupeOnSelection(modules, library);
    }
  }
</script>

<div class="library">
  {#if library.view.kind === 'loading'}
    <p class="message" role="status">Loading the catalog…</p>
  {:else if library.view.kind === 'unavailable'}
    <div class="message error" role="alert" data-testid="catalog-unavailable">
      <strong>The catalog is not available.</strong>
      <span>{library.view.message}</span>
    </div>
  {:else}
    <LibraryToolbar
      {library}
      {onexport}
      mode={modules.libraryView}
      canOpenLoupe={canOpenLoupe(library)}
      onmode={setMode}
    />
    {#if modules.libraryView === 'grid'}
      <LibraryGrid {library} onopen={(index) => openLoupeAt(modules, library, index)} />
    {:else}
      <div class="loupe" data-testid="library-loupe">
        <div class="photo">
          {#if loupePhoto === undefined}
            No photo
          {:else}
            <div>{loupePhoto.fileName}</div>
            <div class="date">{formatCaptureTime(loupePhoto.captureTimeMs)}</div>
          {/if}
        </div>
      </div>
    {/if}
  {/if}
</div>

<style>
  .library {
    height: 100%;
    display: flex;
    flex-direction: column;
  }
  .message {
    margin: auto;
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    align-items: center;
    max-width: 480px;
    text-align: center;
    color: var(--color-text-muted);
  }
  .message.error {
    color: var(--color-danger);
  }
  .loupe {
    flex: 1;
    display: grid;
    place-items: center;
    padding: var(--space-6);
  }
  .photo {
    width: min(100%, 900px);
    aspect-ratio: 3 / 2;
    display: grid;
    place-content: center;
    gap: var(--space-2);
    text-align: center;
    background: var(--color-bg-placeholder);
    color: var(--color-text-muted);
  }
  .date {
    font-size: var(--font-size-sm);
  }
</style>
