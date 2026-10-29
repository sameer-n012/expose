<!--
  The tab strip of the Editor: one tab for each open document.

  - A click on a tab makes it active. A middle click or the × button closes
    it (a tab with unsaved changes asks first, see `EditorStore.closeTab`).
  - A dot after the name shows unsaved changes.
  - The left and right arrow keys move to the previous and next tab.
  - Too many tabs scroll horizontally; the active tab scrolls into view.
-->
<script lang="ts">
  import { tick } from 'svelte';
  import type { EditorStore, EditorTab } from '../../lib/stores/editor.svelte';

  interface Props {
    editor: EditorStore;
  }

  let { editor }: Props = $props();

  let strip: HTMLDivElement | undefined = $state();

  // Scroll the active tab into view (a new tab, or a switch by the keyboard).
  $effect(() => {
    const id = editor.activeTab?.id;
    if (strip === undefined || id === undefined) {
      return;
    }
    const node = strip.querySelector<HTMLElement>(`[data-tab-id="${id}"]`);
    // happy-dom and old web views can lack scrollIntoView.
    node?.scrollIntoView?.({ block: 'nearest', inline: 'nearest' });
  });

  function onauxclick(event: MouseEvent, tab: EditorTab): void {
    if (event.button === 1) {
      event.preventDefault();
      void editor.closeTab(tab);
    }
  }

  function onmousedown(event: MouseEvent): void {
    // The middle button must not start the scroll mode of the web view.
    if (event.button === 1) {
      event.preventDefault();
    }
  }

  async function onkeydown(event: KeyboardEvent): Promise<void> {
    if (event.key !== 'ArrowLeft' && event.key !== 'ArrowRight') {
      return;
    }
    event.preventDefault();
    editor.nextTab(event.key === 'ArrowRight' ? 1 : -1);
    await tick();
    strip?.querySelector<HTMLElement>('[role="tab"][aria-selected="true"]')?.focus();
  }
</script>

<div class="strip" role="tablist" aria-label="Open documents" bind:this={strip} data-testid="document-tabs">
  {#each editor.tabs as tab (tab.id)}
    {@const active = tab === editor.activeTab}
    <div class="tab" class:active class:loading={tab.status === 'loading'} class:error={tab.status === 'error'} data-tab-id={tab.id} role="presentation">
      <button
        type="button"
        class="name"
        role="tab"
        aria-selected={active}
        tabindex={active ? 0 : -1}
        title={tab.status === 'error' ? `The document could not be opened: ${tab.loadError ?? ''}` : tab.title}
        onclick={() => editor.activate(tab)}
        onauxclick={(event) => onauxclick(event, tab)}
        {onmousedown}
        {onkeydown}
      >
        <span class="title">{tab.title}</span>
        {#if tab.dirty}
          <span class="marker" aria-hidden="true" data-testid="tab-dirty">•</span>
          <span class="hidden-text">(unsaved changes)</span>
        {/if}
      </button>
      <button type="button" class="close" aria-label="Close {tab.title}" title="Close {tab.title}" onclick={() => void editor.closeTab(tab)}>×</button>
    </div>
  {/each}
</div>

<style>
  .strip {
    display: flex;
    flex: none;
    gap: 1px;
    overflow-x: auto;
    overflow-y: hidden;
    background: var(--color-bg-panel);
    border-bottom: 1px solid var(--color-border);
    font-size: var(--font-size-sm);
    scrollbar-width: thin;
  }
  .tab {
    display: flex;
    flex: none;
    align-items: center;
    max-width: 16em;
    border-right: 1px solid var(--color-border);
    color: var(--color-text-muted);
  }
  .tab.active {
    background: var(--color-bg-center);
    color: var(--color-text);
    box-shadow: inset 0 -2px 0 var(--color-accent);
  }
  .tab:not(.active):hover {
    background: var(--color-bg-hover);
  }
  .tab.loading .title {
    font-style: italic;
  }
  .tab.error .title {
    color: var(--color-danger);
  }
  button {
    background: none;
    border: none;
    color: inherit;
    font-size: var(--font-size-sm);
  }
  .name {
    display: flex;
    gap: var(--space-1);
    align-items: center;
    min-width: 0;
    padding: var(--space-1) var(--space-1) var(--space-1) var(--space-3);
  }
  .title {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .marker {
    color: var(--color-accent);
  }
  .close {
    padding: var(--space-1) var(--space-2);
    border-radius: var(--radius-sm);
    color: var(--color-text-faint);
  }
  .close:hover {
    background: var(--color-bg-panel-raised);
    color: var(--color-text);
  }
  .hidden-text {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }
</style>
