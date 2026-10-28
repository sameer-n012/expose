<!--
  The tool strip and the selection options of the Editor canvas.

  - Tools: Hand (H), Rectangle and Ellipse marquee (M cycles), Polygon lasso (L),
    Brush (B), Eraser (E). The paint tools show their options in `BrushOptions`.
    They are disabled, with the reason as the tooltip, when the selected layer
    cannot take paint.
  - Options of the selection tools: feather (0 to 250 px) and anti-alias.
  - Select commands: Select All, Deselect, Invert, Feather…. The last three
    need a selection.
-->
<script lang="ts">
  import { MAX_FEATHER, isSelectionTool, type EditorTool } from '../../lib/editor/selection';
  import type { PaintTool } from '../../lib/editor/brush';
  import type { EditorStore } from '../../lib/stores/editor.svelte';
  import BrushOptions from './BrushOptions.svelte';

  interface Props {
    editor: EditorStore;
  }

  let { editor }: Props = $props();

  const PAINT_TOOLS: { id: PaintTool; label: string; title: string }[] = [
    { id: 'brush', label: 'Brush', title: 'Brush (B). Drag to paint. [ and ] change the size.' },
    { id: 'eraser', label: 'Eraser', title: 'Eraser (E). Drag to erase. [ and ] change the size.' },
  ];

  const blocked = $derived(editor.paintBlockReason);
  const paintTool = $derived(editor.paintTool);

  const TOOLS: { id: EditorTool; label: string; title: string }[] = [
    { id: 'hand', label: 'Hand', title: 'Hand tool (H). Drag to move the view.' },
    { id: 'rectSelect', label: 'Rectangle', title: 'Rectangle marquee (M). Shift adds, Alt subtracts, Shift+Alt intersects.' },
    { id: 'ellipseSelect', label: 'Ellipse', title: 'Ellipse marquee (M). Shift adds, Alt subtracts, Shift+Alt intersects.' },
    { id: 'polygonLasso', label: 'Lasso', title: 'Polygon lasso (L). Click for points. Click the first point, double-click, or press Enter to close.' },
  ];

  const selecting = $derived(isSelectionTool(editor.tool));
</script>

<div class="toolbar" role="toolbar" aria-label="Selection tools" data-testid="selection-toolbar">
  <div class="group" role="group" aria-label="Tools">
    {#each TOOLS as tool (tool.id)}
      <button type="button" aria-pressed={editor.tool === tool.id} title={tool.title} onclick={() => editor.setTool(tool.id)}>
        {tool.label}
      </button>
    {/each}
    {#each PAINT_TOOLS as tool (tool.id)}
      <!-- A disabled button shows no tooltip in some web views, so the wrapper has the title too. -->
      <span title={blocked ?? tool.title}>
        <button
          type="button"
          aria-pressed={editor.tool === tool.id}
          title={blocked ?? tool.title}
          disabled={blocked !== null}
          onclick={() => editor.selectPaintTool(tool.id)}
        >
          {tool.label}
        </button>
      </span>
    {/each}
  </div>

  {#if paintTool !== null}
    <BrushOptions {editor} tool={paintTool} />
  {/if}

  {#if selecting}
    <div class="group">
      <label>
        Feather
        <input
          type="number"
          min="0"
          max={MAX_FEATHER}
          step="1"
          value={editor.feather}
          aria-label="Feather (px)"
          onchange={(e) => {
            editor.setFeather(Number(e.currentTarget.value));
            e.currentTarget.value = String(editor.feather);
          }}
        />
      </label>
      <label>
        <input type="checkbox" checked={editor.antiAlias} onchange={(e) => (editor.antiAlias = e.currentTarget.checked)} />
        Anti-alias
      </label>
    </div>
  {/if}

  <div class="group" role="group" aria-label="Select">
    <button type="button" title="Select all (Mod+A)" onclick={() => editor.selectAll()}>Select All</button>
    <button type="button" title="Deselect (Mod+D)" disabled={!editor.hasSelection} onclick={() => editor.deselect()}>Deselect</button>
    <button type="button" title="Invert the selection (Mod+Shift+I)" disabled={!editor.hasSelection} onclick={() => editor.invertSelection()}>
      Invert
    </button>
    <button type="button" title="Feather the selection (Shift+F6)" disabled={!editor.hasSelection} onclick={() => editor.openFeatherDialog()}>
      Feather…
    </button>
  </div>
</div>

<style>
  .toolbar {
    position: absolute;
    left: var(--space-3);
    top: var(--space-3);
    z-index: 1;
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-3);
    align-items: center;
    width: max-content;
    max-width: calc(100% - 2 * var(--space-3));
    padding: var(--space-1) var(--space-2);
    border-radius: var(--radius-sm);
    background: var(--color-bg-panel);
    font-size: var(--font-size-sm);
    color: var(--color-text);
  }
  .group {
    display: flex;
    gap: var(--space-1);
    align-items: center;
  }
  label {
    display: flex;
    gap: var(--space-1);
    align-items: center;
    color: var(--color-text-muted);
  }
  button,
  input[type='number'] {
    background: var(--color-bg-panel-raised);
    color: var(--color-text);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
    padding: var(--space-1) var(--space-2);
    font-size: var(--font-size-sm);
  }
  input[type='number'] {
    width: 4.5em;
  }
  button[aria-pressed='true'] {
    background: var(--color-accent);
    color: var(--color-text-on-accent);
  }
  button:disabled {
    opacity: 0.5;
  }
</style>
