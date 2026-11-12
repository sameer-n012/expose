<!-- The Gaussian blur dialog: one value, the radius (0.1 to 1000 pixels, logarithmic slider). -->
<script lang="ts">
  import { MAX_FILTER_RADIUS, MIN_FILTER_RADIUS, clampRadius } from '../../lib/editor/filter';
  import type { EditorStore, FilterSession } from '../../lib/stores/editor.svelte';
  import FilterDialogFrame from './FilterDialogFrame.svelte';
  import FilterField from './FilterField.svelte';

  interface Props {
    editor: EditorStore;
    session: FilterSession;
  }

  let { editor, session }: Props = $props();

  const values = $derived(editor.filterValues.gaussianBlur);
</script>

<FilterDialogFrame {editor} {session} title="Gaussian Blur" filter={{ type: 'gaussianBlur', radius: clampRadius(values.radius) }}>
  <FilterField
    label="Radius"
    unit="px"
    value={values.radius}
    min={MIN_FILTER_RADIUS}
    max={MAX_FILTER_RADIUS}
    step={0.1}
    decimals={1}
    log
    onchange={(v) => (editor.filterValues.gaussianBlur.radius = clampRadius(v))}
  />
</FilterDialogFrame>
