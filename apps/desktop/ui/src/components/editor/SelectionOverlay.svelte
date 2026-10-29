<!--
  The outline of the selection ("marching ants"), drawn as SVG over the canvas.

  - The outline comes from the shell. A new request goes out when the
    selection version, the document, or the sample step changes. The step is
    max(1, floor(1 / zoom)), so a zoomed-out view gets a coarser outline.
  - A pan or a zoom does not make a request (unless the step changes). The
    paths are in canvas pixels, and one group transform maps them to the screen.
  - A late answer is dropped: only the newest request counts, and the answer
    must have the document and selection version that the view has now.
  - The ants are a white line under a black dashed line. The dashes move,
    except when the user asks for reduced motion.
-->
<script lang="ts">
  import { untrack } from 'svelte';
  import type { SelectionOutline } from '../../lib/editor/api';
  import { contoursPath, outlineStep } from '../../lib/editor/selection';
  import type { EditorStore } from '../../lib/stores/editor.svelte';

  interface Props {
    editor: EditorStore;
  }

  let { editor }: Props = $props();

  let path = $state('');
  let latest = 0;

  const documentId = $derived(editor.info?.documentId ?? null);
  const version = $derived(editor.info !== null && editor.info.hasSelection ? editor.info.selectionVersion : null);
  const step = $derived(outlineStep(editor.view?.zoom ?? 1));

  async function load(id: number | null, wanted: number | null, sampleStep: number): Promise<void> {
    latest += 1;
    const request = latest;
    if (id === null || wanted === null) {
      path = '';
      return;
    }
    const outline: SelectionOutline | null = await editor.selectionOutline(id, sampleStep);
    const stale = request !== latest || editor.info?.documentId !== id || outline === null || outline.selectionVersion !== wanted;
    if (!stale) {
      path = contoursPath(outline.contours);
    }
  }

  $effect(() => {
    const id = documentId;
    const wanted = version;
    const sampleStep = step;
    untrack(() => void load(id, wanted, sampleStep));
  });

  const transform = $derived.by(() => {
    const view = editor.view;
    return view === null ? '' : `matrix(${view.zoom} 0 0 ${view.zoom} ${-view.originX * view.zoom} ${-view.originY * view.zoom})`;
  });
</script>

{#if path !== '' && version !== null}
  <svg class="outline" aria-hidden="true" data-testid="selection-outline">
    <g {transform}>
      <path class="under" d={path} />
      <path class="ants" d={path} />
    </g>
  </svg>
{/if}

<style>
  .outline {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    /* The view can rotate this layer: the viewport clips it, not the layer box. */
    overflow: visible;
    pointer-events: none;
  }
  path {
    fill: none;
    stroke-width: 1px;
    vector-effect: non-scaling-stroke;
  }
  .under {
    stroke: #fff;
  }
  .ants {
    stroke: #000;
    stroke-dasharray: 4 4;
    animation: march 0.6s linear infinite;
  }
  @keyframes march {
    to {
      stroke-dashoffset: -8;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .ants {
      animation: none;
    }
  }
</style>
