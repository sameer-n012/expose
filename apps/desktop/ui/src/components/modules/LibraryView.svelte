<!--
  Center view of the Library module (placeholder).
  Grid view shows placeholder cells. A double-click on a cell opens the
  loupe view (one large photo).
-->
<script lang="ts">
  import type { ModuleStore } from '../../lib/stores/modules.svelte';

  interface Props {
    modules: ModuleStore;
  }

  let { modules }: Props = $props();

  const CELL_COUNT = 30;
  const cells = Array.from({ length: CELL_COUNT }, (_, i) => i);
  let filter = $state('');
  let current = $state(0);

  function openLoupe(index: number): void {
    current = index;
    modules.libraryView = 'loupe';
  }
</script>

<div class="library">
  <div class="filterbar">
    <input
      type="search"
      class="filter"
      placeholder="Search photos"
      aria-label="Search photos"
      bind:value={filter}
    />
    <span class="mode">{modules.libraryView === 'grid' ? 'Grid' : 'Loupe'}</span>
  </div>

  {#if modules.libraryView === 'grid'}
    <div class="grid" data-testid="library-grid">
      {#each cells as i (i)}
        <button
          type="button"
          class="cell"
          class:selected={i === current}
          aria-label="Photo {i + 1}"
          onclick={() => (current = i)}
          ondblclick={() => openLoupe(i)}
        ></button>
      {/each}
    </div>
  {:else}
    <div class="loupe" data-testid="library-loupe">
      <div class="photo">Photo {current + 1}</div>
    </div>
  {/if}
</div>

<style>
  .library {
    height: 100%;
    display: flex;
    flex-direction: column;
  }
  .filterbar {
    display: flex;
    align-items: center;
    gap: var(--space-4);
    padding: var(--space-3) var(--space-4);
    border-bottom: 1px solid var(--color-border);
    background: var(--color-bg-panel);
  }
  .filter {
    width: 240px;
    padding: var(--space-2) var(--space-3);
    background: var(--color-bg-app);
    border: 1px solid var(--color-border-strong);
    border-radius: var(--radius-sm);
    color: var(--color-text);
    font: inherit;
    font-size: var(--font-size-sm);
    user-select: text;
    -webkit-user-select: text;
  }
  .filter::placeholder {
    color: var(--color-text-faint);
  }
  .mode {
    font-size: var(--font-size-sm);
    color: var(--color-text-muted);
  }
  .grid {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(160px, 1fr));
    gap: var(--space-3);
    padding: var(--space-4);
    align-content: start;
  }
  .cell {
    aspect-ratio: 3 / 2;
    background: var(--color-bg-placeholder);
    border: 2px solid transparent;
    border-radius: var(--radius-sm);
    padding: 0;
    cursor: pointer;
  }
  .cell:hover {
    border-color: var(--color-border-strong);
  }
  .cell.selected {
    border-color: var(--color-text-muted);
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
    place-items: center;
    background: var(--color-bg-placeholder);
    color: var(--color-text-muted);
  }
</style>
