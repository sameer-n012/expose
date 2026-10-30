<!--
  The options bar of the free transform (ADR 0029). It shows instead of the
  tool options while the transform runs.

  - X and Y: the position of the reference point after the transform (canvas
    pixels). W and H: the scale in percent (a negative value flips). Angle:
    degrees.
  - The 3x3 grid moves the reference point to a corner, an edge, or the
    center of the layer bounds. The pixels do not move.
  - Interpolation: bicubic (default), bilinear, or nearest.
  - Commit (Enter) sends one edit. Cancel (Escape) sends nothing.
-->
<script lang="ts">
  import { referenceAtGrid, stateFields, withFields, type Interpolation, type TransformFields } from '../../lib/editor/transform';
  import type { EditorStore } from '../../lib/stores/editor.svelte';

  interface Props {
    editor: EditorStore;
  }

  let { editor }: Props = $props();

  const INTERPOLATIONS: { id: Interpolation; label: string }[] = [
    { id: 'bicubic', label: 'Bicubic' },
    { id: 'bilinear', label: 'Bilinear' },
    { id: 'nearest', label: 'Nearest' },
  ];

  const FIELD_LIST: { key: keyof TransformFields; label: string; step: string }[] = [
    { key: 'x', label: 'X', step: '1' },
    { key: 'y', label: 'Y', step: '1' },
    { key: 'width', label: 'W %', step: '1' },
    { key: 'height', label: 'H %', step: '1' },
    { key: 'angle', label: 'Angle °', step: '0.1' },
  ];

  const session = $derived(editor.transform);
  const busy = $derived(editor.transformBusy);
  const fields = $derived(session === null ? null : stateFields(session.state));

  /** Rounds a value for the field text (4 decimals at most). */
  function show(value: number): string {
    return String(Math.round(value * 10000) / 10000 + 0);
  }

  function setField(key: keyof TransformFields, input: HTMLInputElement): void {
    const current = editor.transform;
    if (current === null) {
      return;
    }
    editor.updateTransform(withFields(current.state, { [key]: input.valueAsNumber }));
    // Show the accepted value again (a bad value is dropped).
    const next = editor.transform;
    if (next !== null) {
      input.value = show(stateFields(next.state)[key]);
    }
  }

  function setGrid(column: number, row: number): void {
    const current = editor.transform;
    if (current !== null) {
      editor.updateTransform(referenceAtGrid(current.state, column, row));
    }
  }

  /** True if the reference point is at the grid position. */
  function atGrid(column: number, row: number): boolean {
    if (session === null) {
      return false;
    }
    const b = session.state.bounds;
    return Math.abs(session.state.ref.x - (b.x + (b.width * column) / 2)) < 1e-6 && Math.abs(session.state.ref.y - (b.y + (b.height * row) / 2)) < 1e-6;
  }
</script>

{#if session !== null && fields !== null}
  <div class="options" role="group" aria-label="Free transform options" data-testid="transform-options">
    <div class="grid" role="group" aria-label="Reference point">
      {#each [0, 1, 2] as row (row)}
        {#each [0, 1, 2] as column (column)}
          <button
            type="button"
            class="cell"
            aria-pressed={atGrid(column, row)}
            aria-label="Reference point {row * 3 + column + 1}"
            disabled={busy}
            onclick={() => setGrid(column, row)}
          ></button>
        {/each}
      {/each}
    </div>
    {#each FIELD_LIST as field (field.key)}
      <label>
        {field.label}
        <input
          type="number"
          step={field.step}
          value={show(fields[field.key])}
          aria-label={field.label}
          disabled={busy}
          onchange={(e) => setField(field.key, e.currentTarget)}
        />
      </label>
    {/each}
    <label>
      Interpolation
      <select
        value={session.interpolation}
        disabled={busy}
        onchange={(e) => editor.setTransformInterpolation(e.currentTarget.value as Interpolation)}
      >
        {#each INTERPOLATIONS as option (option.id)}
          <option value={option.id}>{option.label}</option>
        {/each}
      </select>
    </label>
    <button type="button" class="commit" title="Apply the transform (Enter)" disabled={busy} onclick={() => void editor.commitTransform()}>Commit</button>
    <button type="button" title="Leave without a change (Escape)" disabled={busy} onclick={() => editor.cancelTransform()}>Cancel</button>
  </div>
{/if}

<style>
  .options {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
  }
  label {
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
  input[type='number'] {
    width: 72px;
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(3, 10px);
    gap: 2px;
  }
  .cell {
    width: 10px;
    height: 10px;
    padding: 0;
    border-radius: 2px;
  }
  .cell[aria-pressed='true'] {
    background: var(--color-accent, #4a8cff);
  }
</style>
