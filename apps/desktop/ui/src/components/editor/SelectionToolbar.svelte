<!--
  The tool strip and the selection options of the Editor canvas.

  - Tools: Hand (H), Rotate view (R), Rectangle and Ellipse marquee (M cycles), Polygon lasso (L),
    Magic wand (W), Paint bucket and Gradient (G), Brush (B), Eraser (E). The paint tools show their options in `BrushOptions`.
    They are disabled, with the reason as the tooltip, when the selected layer
    cannot take paint.
  - Transform button (Mod+T): starts the free transform. `TransformOptions`
    then replaces the tool options until the transform ends.
  - Options of the selection tools: feather (0 to 250 px) and anti-alias.
  - Select commands: Select All, Deselect, Invert, Feather…. The last three
    need a selection.
-->
<script lang="ts">
  import { MAX_FEATHER, isSelectionTool, type EditorTool } from '../../lib/editor/selection';
  import type { PaintTool } from '../../lib/editor/brush';
  import type { EditorStore } from '../../lib/stores/editor.svelte';
  import type { CloneTool } from '../../lib/editor/clone';
  import type { FilterKind } from '../../lib/editor/filter';
  import BrushOptions from './BrushOptions.svelte';
  import CloneOptions from './CloneOptions.svelte';
  import CropOptions from './CropOptions.svelte';
  import EyedropperOptions from './EyedropperOptions.svelte';
  import GradientOptions from './GradientOptions.svelte';
  import MagicWandOptions from './MagicWandOptions.svelte';
  import PaintBucketOptions from './PaintBucketOptions.svelte';
  import TransformOptions from './TransformOptions.svelte';

  interface Props {
    editor: EditorStore;
  }

  let { editor }: Props = $props();

  const PAINT_TOOLS: { id: PaintTool; label: string; title: string }[] = [
    { id: 'brush', label: 'Brush', title: 'Brush (B). Drag to paint. [ and ] change the size.' },
    { id: 'eraser', label: 'Eraser', title: 'Eraser (E). Drag to erase. [ and ] change the size.' },
  ];

  const CLONE_TOOLS: { id: CloneTool; label: string; title: string }[] = [
    { id: 'cloneStamp', label: 'Clone stamp', title: 'Clone stamp (S). Alt+click sets the source point. Drag to copy pixels from it.' },
    { id: 'healingBrush', label: 'Healing brush', title: 'Healing brush (J). Alt+click sets the source point. Drag to copy pixels and match their tone.' },
  ];

  let filterMenuOpen = $state(false);
  const filterBlocked = $derived(editor.filterBlockReason);

  function openFilter(kind: FilterKind): void {
    filterMenuOpen = false;
    editor.openFilterDialog(kind);
  }

  const cloneBlocked =$derived(editor.cloneBlockReason);
  const cloneTool = $derived(editor.cloneTool);
  const blocked = $derived(editor.paintBlockReason);
  const paintTool = $derived(editor.paintTool);

  const TOOLS: { id: EditorTool; label: string; title: string }[] = [
    { id: 'hand', label: 'Hand', title: 'Hand tool (H). Drag to move the view.' },
    { id: 'rotate', label: 'Rotate', title: 'Rotate view (R). Drag to turn the view. Shift snaps to 15° steps. Escape cancels.' },
    { id: 'rectSelect', label: 'Rectangle', title: 'Rectangle marquee (M). Shift adds, Alt subtracts, Shift+Alt intersects.' },
    { id: 'ellipseSelect', label: 'Ellipse', title: 'Ellipse marquee (M). Shift adds, Alt subtracts, Shift+Alt intersects.' },
    { id: 'lasso', label: 'Freehand', title: 'Freehand lasso (L). Drag to draw the outline. Release to close. Shift adds, Alt subtracts.' },
    { id: 'polygonLasso', label: 'Polygon', title: 'Polygon lasso (L). Click for points. Click the first point, double-click, or press Enter to close.' },
    { id: 'magicWand', label: 'Magic wand', title: 'Magic wand (W). Click to select similar pixels. Shift adds, Alt subtracts, Shift+Alt intersects.' },
    { id: 'paintBucket', label: 'Paint bucket', title: 'Paint bucket (G). Click to fill similar pixels with the foreground color.' },
    { id: 'gradient', label: 'Gradient', title: 'Gradient (G). Drag from the start to the end. Shift snaps to 45° steps. Escape cancels.' },
    { id: 'crop', label: 'Crop', title: 'Crop (C). Drag the handles to size the canvas box, drag outside to straighten. Enter applies. Escape resets, then leaves.' },
    { id: 'eyedropper', label: 'Eyedropper', title: 'Eyedropper (I). Click to pick the foreground color. Alt+click picks the background color.' },
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
    {#each CLONE_TOOLS as tool (tool.id)}
      <span title={cloneBlocked ?? tool.title}>
        <button
          type="button"
          aria-pressed={editor.tool === tool.id}
          title={cloneBlocked ?? tool.title}
          disabled={cloneBlocked !== null}
          onclick={() => editor.selectCloneTool(tool.id)}
        >
          {tool.label}
        </button>
      </span>
    {/each}
  </div>

  <div class="group" role="group" aria-label="Transform">
    <button
      type="button"
      aria-pressed={editor.transform !== null}
      title="Free transform (Mod+T). Move, scale, and rotate the selected pixel layer. Enter applies. Escape cancels."
      onclick={() => void editor.startTransform()}
    >
      Transform
    </button>
  </div>

  {#if editor.transform !== null}
    <TransformOptions {editor} />
  {:else}
  {#if paintTool !== null}
    <BrushOptions {editor} tool={paintTool} />
  {/if}

  {#if cloneTool !== null}
    <CloneOptions {editor} tool={cloneTool} />
  {/if}

  {#if editor.tool === 'crop'}
    <CropOptions {editor} />
  {/if}

  {#if editor.tool === 'eyedropper'}
    <EyedropperOptions {editor} />
  {/if}

  {#if editor.tool === 'magicWand'}
    <MagicWandOptions {editor} />
  {/if}

  {#if editor.tool === 'paintBucket'}
    <PaintBucketOptions {editor} />
  {/if}

  {#if editor.tool === 'gradient'}
    <GradientOptions {editor} />
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
  {/if}

  <div class="group menu" role="group" aria-label="Filter">
    <button
      type="button"
      aria-haspopup="menu"
      aria-expanded={filterMenuOpen}
      title={filterBlocked ?? 'Filters: Gaussian blur and unsharp mask'}
      disabled={filterBlocked !== null || editor.filterDialog !== null}
      onclick={() => (filterMenuOpen = !filterMenuOpen)}
    >
      Filter ▾
    </button>
    {#if filterMenuOpen && filterBlocked === null}
      <div class="popup" role="menu">
        <button type="button" role="menuitem" onclick={() => openFilter('gaussianBlur')}>Gaussian Blur…</button>
        <button type="button" role="menuitem" onclick={() => openFilter('unsharpMask')}>Unsharp Mask…</button>
      </div>
    {/if}
  </div>

  <div class="group" role="group" aria-label="Select">
    <button type="button" title="Select all (Mod+A)" onclick={() => editor.selectAll()}>Select All</button>
    <button type="button" title="Deselect (Mod+D)" disabled={!editor.hasSelection} onclick={() => editor.deselect()}>Deselect</button>
    <button type="button" title="Reselect (Mod+Shift+D)" disabled={!editor.canReselect} onclick={() => editor.reselect()}>Reselect</button>
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
  .menu {
    position: relative;
  }
  .popup {
    position: absolute;
    left: 0;
    top: 100%;
    z-index: 3;
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    margin-top: var(--space-1);
    padding: var(--space-1);
    background: var(--color-bg-panel);
    border: 1px solid var(--color-border-strong);
    border-radius: var(--radius-sm);
    white-space: nowrap;
  }
</style>
