<!--
  The options bar of the crop tool (ADR 0032). It shows while the tool `crop`
  is active.

  - Aspect: free, the original ratio, or a fixed ratio. The swap button turns
    the box and the ratio around.
  - W and H: the size of the box in canvas pixels. Angle: degrees, -45..45,
    clockwise on the screen.
  - "Delete cropped pixels": off keeps the pixels outside of the new canvas.
  - Interpolation: bicubic (default), bilinear, or nearest. It matters when
    the angle is not 0.
  - Commit (Enter) sends one edit. Cancel (Escape) leaves the tool.
-->
<script lang="ts">
  import { CROP_ASPECTS, MAX_CROP_ANGLE, boxFields, type CropAspect, type CropFields } from '../../lib/editor/crop';
  import type { Interpolation } from '../../lib/editor/transform';
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

  const session = $derived(editor.crop);
  const busy = $derived(editor.cropBusy);
  const fields = $derived(session === null ? null : boxFields(session.box));

  /** Rounds a value for the field text (2 decimals at most). */
  function show(value: number): string {
    return String(Math.round(value * 100) / 100 + 0);
  }

  function setField(key: keyof CropFields, input: HTMLInputElement): void {
    editor.setCropField({ [key]: input.valueAsNumber });
    // Show the accepted value again (a bad value is dropped).
    const next = editor.crop;
    if (next !== null) {
      input.value = show(boxFields(next.box)[key]);
    }
  }
</script>

{#if session !== null && fields !== null}
  <div class="options" role="group" aria-label="Crop options" data-testid="crop-options">
    <label>
      Aspect
      <select value={session.aspect} disabled={busy} onchange={(e) => editor.setCropAspect(e.currentTarget.value as CropAspect)}>
        {#each CROP_ASPECTS as option (option.id)}
          <option value={option.id}>{option.label}</option>
        {/each}
      </select>
    </label>
    <button type="button" title="Swap width and height" disabled={busy} onclick={() => editor.swapCrop()}>Swap</button>
    <label>
      W
      <input type="number" min="1" step="1" value={show(fields.width)} aria-label="Crop width" disabled={busy} onchange={(e) => setField('width', e.currentTarget)} />
    </label>
    <label>
      H
      <input type="number" min="1" step="1" value={show(fields.height)} aria-label="Crop height" disabled={busy} onchange={(e) => setField('height', e.currentTarget)} />
    </label>
    <label>
      Angle °
      <input
        type="number"
        min={-MAX_CROP_ANGLE}
        max={MAX_CROP_ANGLE}
        step="0.1"
        value={show(fields.angle)}
        aria-label="Crop angle"
        disabled={busy}
        onchange={(e) => setField('angle', e.currentTarget)}
      />
    </label>
    <label>
      <input type="checkbox" checked={session.deleteCropped} disabled={busy} onchange={(e) => editor.setCropDeleteCropped(e.currentTarget.checked)} />
      Delete cropped pixels
    </label>
    <label>
      Interpolation
      <select value={session.interpolation} disabled={busy} onchange={(e) => editor.setCropInterpolation(e.currentTarget.value as Interpolation)}>
        {#each INTERPOLATIONS as option (option.id)}
          <option value={option.id}>{option.label}</option>
        {/each}
      </select>
    </label>
    <button type="button" class="commit" title="Apply the crop (Enter)" disabled={busy} onclick={() => void editor.commitCrop()}>Commit</button>
    <button type="button" title="Leave without a change (Escape)" disabled={busy} onclick={() => editor.cancelCrop()}>Cancel</button>
    {#if busy}
      <span role="status" class="busy">Cropping…</span>
    {/if}
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
  .busy {
    color: var(--color-text-muted);
  }
</style>
