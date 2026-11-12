<!-- The unsharp mask dialog: amount (1 to 500 %), radius (0.1 to 1000 pixels), threshold (0 to 255). -->
<script lang="ts">
  import {
    MAX_AMOUNT_PERCENT,
    MAX_FILTER_RADIUS,
    MAX_THRESHOLD,
    MIN_AMOUNT_PERCENT,
    MIN_FILTER_RADIUS,
    MIN_THRESHOLD,
    clampAmountPercent,
    clampRadius,
    clampThreshold,
    filterDto,
  } from '../../lib/editor/filter';
  import type { EditorStore, FilterSession } from '../../lib/stores/editor.svelte';
  import FilterDialogFrame from './FilterDialogFrame.svelte';
  import FilterField from './FilterField.svelte';

  interface Props {
    editor: EditorStore;
    session: FilterSession;
  }

  let { editor, session }: Props = $props();

  const values = $derived(editor.filterValues.unsharpMask);
  // The amount goes to the shell as a fraction: `filterDto` converts it.
  const filter = $derived(filterDto('unsharpMask', editor.filterValues));
</script>

<FilterDialogFrame {editor} {session} title="Unsharp Mask" {filter}>
  <FilterField
    label="Amount"
    unit="%"
    value={values.amountPercent}
    min={MIN_AMOUNT_PERCENT}
    max={MAX_AMOUNT_PERCENT}
    onchange={(v) => (editor.filterValues.unsharpMask.amountPercent = clampAmountPercent(v))}
  />
  <FilterField
    label="Radius"
    unit="px"
    value={values.radius}
    min={MIN_FILTER_RADIUS}
    max={MAX_FILTER_RADIUS}
    step={0.1}
    decimals={1}
    log
    onchange={(v) => (editor.filterValues.unsharpMask.radius = clampRadius(v))}
  />
  <FilterField
    label="Threshold"
    unit="levels"
    value={values.threshold}
    min={MIN_THRESHOLD}
    max={MAX_THRESHOLD}
    onchange={(v) => (editor.filterValues.unsharpMask.threshold = clampThreshold(v))}
  />
</FilterDialogFrame>
