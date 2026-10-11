<!--
  The Layers panel of the Editor: the save state and the Save button, the
  layer tree (top layer first), the layer commands, and the opacity, fill,
  and blend mode of the selected layer.

  Sliders send live edits with one gesture per drag (see EditorStore).
  While edits are in flight, the sliders show the dragged value, so a late
  answer does not move the handle back.
-->
<script lang="ts">
  import type { BlendModeId, LayerInfo } from '../../lib/editor/api';
  import {
    ADJUSTMENT_LABELS,
    ADJUSTMENT_TYPES,
    BLEND_GROUPS,
    defaultAdjustment,
    layerRows,
    positionOf,
  } from '../../lib/editor/layers';
  import type { EditorStore } from '../../lib/stores/editor.svelte';
  import EditorSlider from './EditorSlider.svelte';

  interface Props {
    editor: EditorStore;
  }

  let { editor }: Props = $props();

  const info = $derived(editor.info);
  const rows = $derived(info === null ? [] : layerRows(info.layers, editor.expanded));
  const selected = $derived(editor.selected);
  const position = $derived(info === null || selected === null ? null : positionOf(info.layers, selected.id));

  /** The save state text of the document. */
  const saveState = $derived.by(() => {
    if (editor.saving) {
      return 'Saving…';
    }
    if (editor.dirty) {
      return 'Unsaved changes';
    }
    return info === null || info.savedPhotoId === null ? 'Not saved' : 'Saved';
  });

  let drafts = $state<Record<string, number>>({});
  let renaming = $state<number | null>(null);
  let menuOpen = $state(false);

  // Drop the dragged values when all edits are done.
  $effect(() => {
    if (editor.idle) {
      drafts = {};
    }
  });

  const KIND_ICON: Readonly<Record<LayerInfo['kind'], string>> = { pixel: '▦', adjustment: '◐', group: '▣' };
  const KIND_LABEL: Readonly<Record<LayerInfo['kind'], string>> = {
    pixel: 'Pixel layer',
    adjustment: 'Adjustment layer',
    group: 'Group',
  };

  function percent(layer: LayerInfo, key: 'opacity' | 'fillOpacity'): number {
    return Math.round(drafts[`${layer.id}:${key}`] ?? layer[key] * 100);
  }

  function setPercent(layer: LayerInfo, key: 'opacity' | 'fillOpacity', value: number): void {
    const clamped = Math.min(100, Math.max(0, value));
    drafts[`${layer.id}:${key}`] = clamped;
    const fraction = clamped / 100;
    editor.liveEdit(
      `${key}:${layer.id}`,
      key === 'opacity'
        ? { type: 'setOpacity', layerId: layer.id, opacity: fraction }
        : { type: 'setFillOpacity', layerId: layer.id, fillOpacity: fraction },
    );
  }

  function onblend(event: Event & { currentTarget: HTMLSelectElement }): void {
    if (selected !== null) {
      editor.edit({ type: 'setBlendMode', layerId: selected.id, mode: event.currentTarget.value as BlendModeId });
    }
  }

  function addAdjustment(type: (typeof ADJUSTMENT_TYPES)[number]): void {
    menuOpen = false;
    editor.edit({ type: 'addAdjustment', adjustment: defaultAdjustment(type), aboveLayerId: editor.selectedId });
  }

  function commitRename(layer: LayerInfo, value: string): void {
    renaming = null;
    editor.rename(layer.id, value);
  }

  function renameKey(event: KeyboardEvent & { currentTarget: HTMLInputElement }, layer: LayerInfo): void {
    if (event.key === 'Enter') {
      event.preventDefault();
      commitRename(layer, event.currentTarget.value);
    } else if (event.key === 'Escape') {
      event.preventDefault();
      event.stopPropagation();
      renaming = null;
    }
  }

  function focusInput(node: HTMLInputElement): void {
    node.focus();
    node.select();
  }
</script>

<details class="section" open>
  <summary>Layers</summary>
  <div class="body" role="group" aria-label="Layers">
    {#if info !== null}
      <div class="doc">
        <span class="state" class:dirty={editor.dirty} data-testid="editor-save-state">
          {#if editor.dirty}<span class="marker" aria-hidden="true">•</span>{/if}
          {saveState}
        </span>
        <button type="button" class="save" disabled={!editor.canSave} title="Save to library (Mod+S)" onclick={() => void editor.save()}>
          {editor.saving ? 'Saving…' : 'Save'}
        </button>
      </div>
    {/if}
    <div class="toolbar">
      <button type="button" disabled={!editor.canUndo} title="Undo (Mod+Z)" onclick={() => editor.undo()}>Undo</button>
      <button type="button" disabled={!editor.canRedo} title="Redo (Mod+Shift+Z)" onclick={() => editor.redo()}>Redo</button>
      <span class="menu">
        <button
          type="button"
          disabled={info === null}
          aria-haspopup="menu"
          aria-expanded={menuOpen}
          onclick={() => (menuOpen = !menuOpen)}
        >
          Add adjustment
        </button>
        {#if menuOpen}
          <div class="popup" role="menu" aria-label="Add adjustment">
            {#each ADJUSTMENT_TYPES as type (type)}
              <button type="button" role="menuitem" onclick={() => addAdjustment(type)}>{ADJUSTMENT_LABELS[type]}</button>
            {/each}
          </div>
        {/if}
      </span>
    </div>

    {#if info === null}
      <p class="empty">No document.</p>
    {:else}
      <ul class="tree" aria-label="Layer list">
        {#each rows as { layer, depth } (layer.id)}
          <li class="layer" class:selected={layer.id === editor.selectedId} style:padding-left="calc({depth} * var(--space-4))" data-layer-id={layer.id}>
            {#if layer.kind === 'group'}
              <button
                type="button"
                class="icon"
                aria-label={editor.expanded.has(layer.id) ? `Collapse ${layer.name}` : `Expand ${layer.name}`}
                aria-expanded={editor.expanded.has(layer.id)}
                onclick={() => editor.toggleExpanded(layer.id)}
              >
                {editor.expanded.has(layer.id) ? '▾' : '▸'}
              </button>
            {:else}
              <span class="icon spacer"></span>
            {/if}
            <button
              type="button"
              class="icon eye"
              class:off={!layer.visible}
              aria-label={layer.visible ? `Hide ${layer.name}` : `Show ${layer.name}`}
              aria-pressed={layer.visible}
              onclick={() => editor.setVisible(layer.id, !layer.visible)}
            >
              {layer.visible ? '◉' : '○'}
            </button>
            <span class="kind" role="img" aria-label={KIND_LABEL[layer.kind]} title={KIND_LABEL[layer.kind]}>{KIND_ICON[layer.kind]}</span>
            {#if renaming === layer.id}
              <input
                class="rename"
                aria-label="Layer name"
                value={layer.name}
                use:focusInput
                onkeydown={(e) => renameKey(e, layer)}
                onblur={(e) => commitRename(layer, e.currentTarget.value)}
              />
            {:else}
              <button
                type="button"
                class="name"
                class:hidden-layer={!layer.visible}
                aria-pressed={layer.id === editor.selectedId}
                onclick={() => editor.select(layer.id)}
                ondblclick={() => (renaming = layer.id)}
              >
                {layer.name}
              </button>
            {/if}
          </li>
        {/each}
      </ul>

      <div class="toolbar">
        <button type="button" disabled={selected === null} onclick={() => selected && editor.edit({ type: 'duplicate', layerId: selected.id })}>
          Duplicate
        </button>
        <button type="button" disabled={selected === null} onclick={() => selected && editor.edit({ type: 'delete', layerId: selected.id })}>
          Delete
        </button>
        <button
          type="button"
          aria-label="Move layer up"
          title="Move layer up"
          disabled={selected === null || position === null || position.index >= position.siblings - 1}
          onclick={() => selected && editor.move(selected.id, 'up')}
        >
          Up
        </button>
        <button
          type="button"
          aria-label="Move layer down"
          title="Move layer down"
          disabled={selected === null || position === null || position.index <= 0}
          onclick={() => selected && editor.move(selected.id, 'down')}
        >
          Down
        </button>
      </div>

      {#if selected !== null}
        <div class="mode">
          <label for="layer-blend">Blend mode</label>
          <select id="layer-blend" value={selected.blendMode} onchange={onblend}>
            {#each BLEND_GROUPS as group, g (g)}
              {#if g > 0}<hr />{/if}
              {#each group as mode (mode.id)}
                <option value={mode.id}>{mode.label}</option>
              {/each}
            {/each}
          </select>
        </div>
        <EditorSlider
          label="Opacity"
          value={percent(selected, 'opacity')}
          min={0}
          max={100}
          unit="%"
          onchange={(v) => setPercent(selected, 'opacity', v)}
          onend={() => editor.endGesture()}
          onreset={() => {
            setPercent(selected, 'opacity', 100);
            editor.endGesture();
          }}
        />
        <EditorSlider
          label="Fill"
          value={percent(selected, 'fillOpacity')}
          min={0}
          max={100}
          unit="%"
          onchange={(v) => setPercent(selected, 'fillOpacity', v)}
          onend={() => editor.endGesture()}
          onreset={() => {
            setPercent(selected, 'fillOpacity', 100);
            editor.endGesture();
          }}
        />
      {/if}
    {/if}
  </div>
</details>

<style>
  .section {
    border-bottom: 1px solid var(--color-border);
  }
  summary {
    padding: var(--space-3) var(--space-4);
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-strong);
    color: var(--color-text-muted);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    cursor: default;
  }
  .doc {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-4) 0;
    font-size: var(--font-size-sm);
  }
  .state {
    color: var(--color-text-muted);
  }
  .state.dirty {
    color: var(--color-text);
  }
  .marker {
    margin-right: var(--space-1);
    color: var(--color-accent);
  }
  .save:not(:disabled) {
    background: var(--color-accent);
    color: var(--color-text-on-accent);
  }
  .toolbar {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-4);
  }
  .menu {
    position: relative;
  }
  .popup {
    position: absolute;
    z-index: 5;
    top: 100%;
    left: 0;
    display: flex;
    flex-direction: column;
    min-width: 100%;
    background: var(--color-bg-panel-raised);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
  }
  .popup button {
    border: none;
    text-align: left;
    white-space: nowrap;
  }
  .empty {
    margin: 0;
    padding: var(--space-2) var(--space-4) var(--space-4);
    font-size: var(--font-size-sm);
    color: var(--color-text-muted);
  }
  .tree {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .layer {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    padding-right: var(--space-3);
    font-size: var(--font-size-sm);
  }
  .layer.selected {
    background: var(--color-bg-hover);
  }
  .icon {
    width: 1.6em;
    flex: none;
    padding: 0;
    border: none;
    background: none;
    color: var(--color-text-muted);
    text-align: center;
  }
  .icon.off {
    opacity: 0.5;
  }
  .kind {
    flex: none;
    color: var(--color-text-muted);
  }
  .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    padding: var(--space-2);
    border: none;
    background: none;
    color: var(--color-text);
    text-align: left;
    font-size: var(--font-size-sm);
  }
  .name.hidden-layer {
    color: var(--color-text-muted);
  }
  .rename {
    flex: 1;
    min-width: 0;
    background: var(--color-bg-panel-raised);
    color: var(--color-text);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
    font-size: var(--font-size-sm);
  }
  .mode {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--space-2) var(--space-4);
    font-size: var(--font-size-sm);
  }
  select,
  .toolbar button {
    background: var(--color-bg-panel-raised);
    color: var(--color-text);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
    padding: var(--space-1) var(--space-3);
    font-size: var(--font-size-sm);
  }
  .toolbar button:disabled {
    opacity: 0.5;
  }
</style>
