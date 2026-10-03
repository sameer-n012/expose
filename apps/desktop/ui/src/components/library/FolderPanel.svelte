<!--
  Folder tree of the Library module (left panel). Shows the photo count of
  each folder. Click selects a folder. The arrow keys move through the tree:
  Up and Down select the previous or next row, Right opens a folder, Left
  closes it. The "Include subfolders" option decides if the grid also shows
  the photos of the subfolders.
-->
<script lang="ts">
  import { flattenTree } from '../../lib/library/folderRows';
  import type { LibraryStore } from '../../lib/stores/library.svelte';

  interface Props {
    library: LibraryStore;
  }

  let { library }: Props = $props();

  const rows = $derived(flattenTree(library.tree, library.expanded));

  function onkeydown(event: KeyboardEvent, index: number): void {
    const row = rows[index];
    if (row === undefined) {
      return;
    }
    let handled = true;
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      const next = rows[index + (event.key === 'ArrowDown' ? 1 : -1)];
      if (next !== undefined) {
        void library.selectFolder(next.node.folder.id);
        const items = (event.currentTarget as HTMLElement).parentElement?.querySelectorAll<HTMLElement>('[role="treeitem"]');
        items?.[rows.indexOf(next)]?.focus();
      }
    } else if (event.key === 'ArrowRight' && row.expandable && !row.expanded) {
      library.toggleExpanded(row.node.folder.id);
    } else if (event.key === 'ArrowLeft' && row.expanded) {
      library.toggleExpanded(row.node.folder.id);
    } else if (event.key === 'Enter' || event.key === ' ') {
      void library.selectFolder(row.node.folder.id);
    } else {
      handled = false;
    }
    if (handled) {
      // The tree used the key. The grid shortcuts (arrow keys) must not use it too.
      event.preventDefault();
      event.stopPropagation();
    }
  }
</script>

<section class="folders" aria-label="Folders">
  <label class="option">
    <input
      type="checkbox"
      checked={library.recursive}
      onchange={(event) => void library.setRecursive(event.currentTarget.checked)}
    />
    Include subfolders
  </label>

  {#if rows.length === 0}
    <p class="empty">No folders yet.</p>
  {:else}
    <div class="tree" role="tree" aria-label="Folder tree">
      {#each rows as row, index (row.node.folder.id)}
        {@const id = row.node.folder.id}
        <div
          class="row"
          class:selected={id === library.folderId}
          role="treeitem"
          aria-level={row.depth + 1}
          aria-selected={id === library.folderId}
          aria-expanded={row.expandable ? row.expanded : undefined}
          tabindex={id === library.folderId || (library.folderId === null && index === 0) ? 0 : -1}
          style:padding-left="{row.depth * 14 + 4}px"
          title={row.node.folder.path}
          onclick={() => void library.selectFolder(id)}
          onkeydown={(event) => onkeydown(event, index)}
        >
          {#if row.expandable}
            <button
              type="button"
              class="twisty"
              tabindex="-1"
              aria-label="{row.expanded ? 'Collapse' : 'Expand'} {row.node.folder.name}"
              onclick={(event) => {
                event.stopPropagation();
                library.toggleExpanded(id);
              }}
            >
              {row.expanded ? '▾' : '▸'}
            </button>
          {:else}
            <span class="twisty"></span>
          {/if}
          <span class="name">{row.node.folder.name}</span>
          <span class="count">{row.node.photoCount}</span>
        </div>
      {/each}
    </div>
  {/if}
</section>

<style>
  .folders {
    padding: var(--space-3) 0;
    border-bottom: 1px solid var(--color-border);
  }
  .option {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: 0 var(--space-4) var(--space-3);
    font-size: var(--font-size-sm);
    color: var(--color-text-muted);
  }
  .empty {
    margin: 0;
    padding: 0 var(--space-4);
    font-size: var(--font-size-sm);
    color: var(--color-text-faint);
  }
  .row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding-right: var(--space-4);
    min-height: 24px;
    font-size: var(--font-size-md);
    cursor: default;
  }
  .row:hover {
    background: var(--color-bg-hover);
  }
  .row.selected {
    background: var(--color-bg-active);
  }
  .twisty {
    flex: none;
    width: 16px;
    padding: 0;
    background: none;
    border: none;
    color: var(--color-text-muted);
    cursor: pointer;
  }
  .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .count {
    flex: none;
    font-size: var(--font-size-xs);
    color: var(--color-text-muted);
  }
</style>
