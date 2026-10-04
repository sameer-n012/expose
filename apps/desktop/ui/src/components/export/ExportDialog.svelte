<!--
  The export dialog (modal). The user sets the format, color space, size,
  destination, and naming, and then starts the job.

  Keyboard: Tab and Shift+Tab stay inside the dialog. Esc closes it. Key
  events do not leave the dialog, so shortcuts of the shell do not run
  behind it.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import {
    bitDepthOf,
    SIZE_MODE_DEFAULTS,
    type FormatKind,
    type NamingMode,
    type SizeMode,
  } from '../../lib/export/options';
  import type {
    ExportColorSpace,
    ExportConflict,
    JpegSubsampling,
    TiffCompression,
  } from '../../lib/export/types';
  import type { ExportStore } from '../../lib/stores/export.svelte';

  interface Props {
    store: ExportStore;
  }

  let { store }: Props = $props();

  let dialog: HTMLDivElement | undefined = $state();
  const form = $derived(store.form);
  const errors = $derived(store.errors);
  const count = $derived(store.dialogIds?.length ?? 0);
  const canSubmit = $derived(Object.keys(errors).length === 0 && !store.busy);

  const COLOR_SPACES: ReadonlyArray<{ value: ExportColorSpace; label: string }> = [
    { value: 'srgb', label: 'sRGB' },
    { value: 'display_p3', label: 'Display P3' },
    { value: 'oprgb', label: 'opRGB' },
    { value: 'prophoto_rgb', label: 'ProPhoto RGB' },
    { value: 'rec2020', label: 'Rec. 2020' },
  ];
  const SIZE_MODES: ReadonlyArray<{ value: SizeMode; label: string; unit: string }> = [
    { value: 'original', label: 'Original size', unit: '' },
    { value: 'longEdge', label: 'Long edge', unit: 'px' },
    { value: 'shortEdge', label: 'Short edge', unit: 'px' },
    { value: 'width', label: 'Width', unit: 'px' },
    { value: 'height', label: 'Height', unit: 'px' },
    { value: 'megapixels', label: 'Megapixels', unit: 'MP' },
    { value: 'percent', label: 'Percent', unit: '%' },
  ];
  const sizeUnit = $derived(SIZE_MODES.find((m) => m.value === form.sizeMode)?.unit ?? '');

  const FOCUSABLE = 'button:not(:disabled), input:not(:disabled), select:not(:disabled), [href], [tabindex]:not([tabindex="-1"])';

  onMount(() => {
    const previous = document.activeElement;
    dialog?.querySelector<HTMLElement>('select, input, button')?.focus();
    return () => {
      // Give the focus back to the control that opened the dialog.
      if (previous instanceof HTMLElement) {
        previous.focus();
      }
    };
  });

  function onkeydown(event: KeyboardEvent): void {
    // Shell shortcuts must not run while the dialog is open.
    event.stopPropagation();
    if (event.key === 'Escape') {
      event.preventDefault();
      store.closeDialog();
      return;
    }
    if (event.key !== 'Tab' || dialog === undefined) {
      return;
    }
    const items = [...dialog.querySelectorAll<HTMLElement>(FOCUSABLE)];
    const first = items[0];
    const last = items[items.length - 1];
    if (first === undefined || last === undefined) {
      return;
    }
    const active = document.activeElement;
    if (event.shiftKey && (active === first || !dialog.contains(active))) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && (active === last || !dialog.contains(active))) {
      event.preventDefault();
      first.focus();
    }
  }

  function numberOf(event: Event): number {
    const text = (event.currentTarget as HTMLInputElement).value.trim();
    return text === '' ? Number.NaN : Number(text);
  }

  function onsizemode(event: Event): void {
    const mode = (event.currentTarget as HTMLSelectElement).value as SizeMode;
    form.sizeMode = mode;
    if (mode !== 'original') {
      form.sizeValue = SIZE_MODE_DEFAULTS[mode];
    }
  }

  function onsubmit(event: SubmitEvent): void {
    event.preventDefault();
    void store.submit();
  }
</script>

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div class="backdrop" onclick={(e) => e.target === e.currentTarget && store.closeDialog()}>
  <div
    class="dialog"
    role="dialog"
    aria-modal="true"
    aria-labelledby="export-title"
    tabindex="-1"
    bind:this={dialog}
    {onkeydown}
  >
    <form {onsubmit}>
      <h2 id="export-title">Export</h2>
      <p class="count" data-testid="export-count">
        {count === 1 ? '1 photo' : `${count} photos`}
      </p>
      {#if errors.photos !== undefined}
        <p class="error" role="alert">{errors.photos}</p>
      {/if}

      <fieldset>
        <legend>Format</legend>
        <label class="row">
          File format
          <select value={form.formatKind} onchange={(e) => (form.formatKind = e.currentTarget.value as FormatKind)}>
            <option value="jpeg">JPEG</option>
            <option value="png">PNG</option>
            <option value="tiff">TIFF</option>
          </select>
        </label>

        {#if form.formatKind === 'jpeg'}
          <label class="row">
            Quality
            <span class="inline">
              <input
                type="range"
                min="1"
                max="100"
                step="1"
                aria-label="Quality slider"
                value={form.jpegQuality}
                oninput={(e) => (form.jpegQuality = numberOf(e))}
              />
              <input
                type="number"
                min="1"
                max="100"
                step="1"
                class="num"
                aria-label="Quality"
                value={form.jpegQuality}
                oninput={(e) => (form.jpegQuality = numberOf(e))}
              />
            </span>
          </label>
          {#if errors.quality !== undefined}
            <p class="error" role="alert">{errors.quality}</p>
          {/if}
          <label class="row">
            Chroma subsampling
            <select
              value={form.jpegSubsampling}
              onchange={(e) => (form.jpegSubsampling = e.currentTarget.value as JpegSubsampling)}
            >
              <option value="444">4:4:4 (best color)</option>
              <option value="422">4:2:2</option>
              <option value="420">4:2:0 (smallest)</option>
            </select>
          </label>
        {:else if form.formatKind === 'png'}
          <label class="row">
            Bit depth
            <select
              value={String(form.pngBitDepth)}
              onchange={(e) => (form.pngBitDepth = e.currentTarget.value === '16' ? 16 : 8)}
            >
              <option value="8">8 bit</option>
              <option value="16">16 bit</option>
            </select>
          </label>
        {:else}
          <label class="row">
            Bit depth
            <select
              value={String(form.tiffBitDepth)}
              onchange={(e) => (form.tiffBitDepth = e.currentTarget.value === '16' ? 16 : 8)}
            >
              <option value="8">8 bit</option>
              <option value="16">16 bit</option>
            </select>
          </label>
          <label class="row">
            Compression
            <select
              value={form.tiffCompression}
              onchange={(e) => (form.tiffCompression = e.currentTarget.value as TiffCompression)}
            >
              <option value="none">None</option>
              <option value="lzw">LZW</option>
              <option value="deflate">Deflate</option>
            </select>
          </label>
        {/if}

        {#if bitDepthOf(form) === 8}
          <label class="check">
            <input type="checkbox" bind:checked={form.dither} />
            Dither (smooth gradients)
          </label>
        {/if}

        <label class="row">
          Color space
          <select
            value={form.colorSpace}
            onchange={(e) => (form.colorSpace = e.currentTarget.value as ExportColorSpace)}
          >
            {#each COLOR_SPACES as space (space.value)}
              <option value={space.value}>{space.label}</option>
            {/each}
          </select>
        </label>
      </fieldset>

      <fieldset>
        <legend>Size</legend>
        <label class="row">
          Resize to
          <select value={form.sizeMode} onchange={onsizemode}>
            {#each SIZE_MODES as mode (mode.value)}
              <option value={mode.value}>{mode.label}</option>
            {/each}
          </select>
        </label>
        {#if form.sizeMode !== 'original'}
          <label class="row">
            Value
            <span class="inline">
              <input
                type="number"
                class="num"
                aria-label="Size value"
                min="0"
                step={form.sizeMode === 'megapixels' ? 'any' : '1'}
                value={form.sizeValue}
                oninput={(e) => (form.sizeValue = numberOf(e))}
              />
              {sizeUnit}
            </span>
          </label>
        {/if}
        <label class="check">
          <input type="checkbox" bind:checked={form.allowUpscale} />
          Allow upscaling
        </label>
        {#if errors.size !== undefined}
          <p class="error" role="alert">{errors.size}</p>
        {/if}
      </fieldset>

      <fieldset>
        <legend>Destination</legend>
        <div class="row">
          <button type="button" onclick={() => void store.chooseDestination()}>Choose folder…</button>
          <span class="path" data-testid="export-destination" title={form.destination}>
            {form.destination === '' ? 'No folder chosen' : form.destination}
          </span>
        </div>
        {#if errors.destination !== undefined}
          <p class="error" role="alert">{errors.destination}</p>
        {/if}
        <label class="row">
          File names
          <select value={form.namingMode} onchange={(e) => (form.namingMode = e.currentTarget.value as NamingMode)}>
            <option value="original">Original name</option>
            <option value="suffix">Original name + suffix</option>
          </select>
        </label>
        {#if form.namingMode === 'suffix'}
          <label class="row">
            Suffix
            <input type="text" maxlength="200" bind:value={form.suffix} />
          </label>
          {#if errors.suffix !== undefined}
            <p class="error" role="alert">{errors.suffix}</p>
          {/if}
        {/if}
        <label class="row">
          If the file exists
          <select
            value={form.onConflict}
            onchange={(e) => (form.onConflict = e.currentTarget.value as ExportConflict)}
          >
            <option value="uniqueName">Use a new name</option>
            <option value="overwrite">Overwrite</option>
            <option value="skip">Skip the photo</option>
          </select>
        </label>
      </fieldset>

      <div class="actions">
        <button type="button" onclick={() => store.closeDialog()}>Cancel</button>
        <button type="submit" class="primary" disabled={!canSubmit}>Export</button>
      </div>
    </form>
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: var(--z-toast);
    display: flex;
    align-items: center;
    justify-content: center;
    background: rgb(0 0 0 / 55%);
  }
  .dialog {
    width: min(480px, 92vw);
    max-height: 92vh;
    overflow: auto;
    padding: var(--space-5);
    background: var(--color-bg-panel);
    border: 1px solid var(--color-border-strong);
    border-radius: var(--radius-md);
    font-size: var(--font-size-md);
  }
  h2 {
    margin: 0;
    font-size: var(--font-size-xl);
  }
  .count {
    margin: var(--space-2) 0 var(--space-4);
    color: var(--color-text-muted);
  }
  fieldset {
    margin: 0 0 var(--space-4);
    padding: var(--space-3) var(--space-4);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
  }
  legend {
    padding: 0 var(--space-2);
    color: var(--color-text-muted);
    font-size: var(--font-size-sm);
  }
  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-4);
    margin: var(--space-3) 0;
  }
  .check {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    margin: var(--space-3) 0;
  }
  .inline {
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }
  .num {
    width: 80px;
  }
  select,
  input[type='text'],
  input[type='number'],
  button {
    background: var(--color-bg-panel-raised);
    color: inherit;
    border: 1px solid var(--color-border-strong);
    border-radius: var(--radius-sm);
    padding: var(--space-2) var(--space-3);
    font: inherit;
  }
  button {
    cursor: pointer;
  }
  button:hover:not(:disabled) {
    background: var(--color-bg-hover);
  }
  button:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .primary {
    background: var(--color-accent);
    color: var(--color-text-on-accent);
  }
  .path {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--color-text-muted);
    direction: rtl;
    text-align: left;
  }
  .error {
    margin: var(--space-2) 0;
    color: var(--color-danger);
    font-size: var(--font-size-sm);
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-3);
  }
</style>
