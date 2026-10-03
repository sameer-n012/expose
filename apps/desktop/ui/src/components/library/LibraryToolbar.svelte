<!--
  Toolbar of the Library module: import, search, sort, filter, and the
  color label buttons for the selected photos.
-->
<script lang="ts">
  import type { ColorLabel, Flag, SortField } from '../../lib/catalog';
  import type { LibraryStore } from '../../lib/stores/library.svelte';

  interface Props {
    library: LibraryStore;
    /** Text of the view mode, for example "Grid". */
    mode: string;
  }

  let { library, mode }: Props = $props();

  let search = $state('');

  const SORT_FIELDS: ReadonlyArray<{ value: SortField; label: string }> = [
    { value: 'captureTime', label: 'Capture time' },
    { value: 'fileName', label: 'File name' },
    { value: 'rating', label: 'Rating' },
    { value: 'importTime', label: 'Import time' },
  ];
  const FLAGS: ReadonlyArray<{ value: Flag | 'any'; label: string }> = [
    { value: 'any', label: 'Any flag' },
    { value: 'pick', label: 'Picked' },
    { value: 'reject', label: 'Rejected' },
    { value: 'none', label: 'Unflagged' },
  ];
  const LABELS: ReadonlyArray<{ value: ColorLabel | 'any'; label: string }> = [
    { value: 'any', label: 'Any label' },
    { value: 'red', label: 'Red' },
    { value: 'yellow', label: 'Yellow' },
    { value: 'green', label: 'Green' },
    { value: 'blue', label: 'Blue' },
    { value: 'purple', label: 'Purple' },
    { value: 'none', label: 'No label' },
  ];
  const LABEL_BUTTONS: readonly ColorLabel[] = ['red', 'yellow', 'green', 'blue', 'purple'];

  const SORT_VALUES: ReadonlySet<string> = new Set(SORT_FIELDS.map((f) => f.value));
  const FLAG_VALUES: ReadonlySet<string> = new Set(FLAGS.map((f) => f.value));
  const LABEL_VALUES: ReadonlySet<string> = new Set(LABELS.map((f) => f.value));

  /** Leaves the select after a change, so that the photo shortcuts work again. */
  function release(event: Event): void {
    (event.currentTarget as HTMLElement).blur();
  }

  function onsort(event: Event): void {
    const value = (event.currentTarget as HTMLSelectElement).value;
    if (SORT_VALUES.has(value)) {
      void library.setSort({ field: value as SortField });
    }
    release(event);
  }

  function onflag(event: Event): void {
    const value = (event.currentTarget as HTMLSelectElement).value;
    if (FLAG_VALUES.has(value)) {
      void library.setFilter({ flag: value as Flag | 'any' });
    }
    release(event);
  }

  function onlabel(event: Event): void {
    const value = (event.currentTarget as HTMLSelectElement).value;
    if (LABEL_VALUES.has(value)) {
      void library.setFilter({ colorLabel: value as ColorLabel | 'any' });
    }
    release(event);
  }

  function onrating(event: Event): void {
    const value = Number((event.currentTarget as HTMLSelectElement).value);
    if (Number.isInteger(value) && value >= 0 && value <= 5) {
      void library.setFilter({ minRating: value });
    }
    release(event);
  }
</script>

<div class="toolbar" role="toolbar" aria-label="Library toolbar">
  <button
    type="button"
    class="import"
    disabled={library.importing}
    onclick={() => void library.importPhotos()}
  >
    {library.importing ? 'Importing…' : 'Import folder…'}
  </button>

  <input
    type="search"
    class="search"
    placeholder="Search photos"
    aria-label="Search photos"
    bind:value={search}
  />

  <label class="field">
    Sort
    <select value={library.sort.field} onchange={onsort}>
      {#each SORT_FIELDS as f (f.value)}
        <option value={f.value}>{f.label}</option>
      {/each}
    </select>
  </label>
  <button
    type="button"
    class="direction"
    aria-label="Sort direction: {library.sort.direction}"
    title={library.sort.direction === 'ascending' ? 'Ascending' : 'Descending'}
    onclick={() => void library.setSort({ direction: library.sort.direction === 'ascending' ? 'descending' : 'ascending' })}
  >
    {library.sort.direction === 'ascending' ? '↑' : '↓'}
  </button>

  <label class="field">
    Rating
    <select value={String(library.filter.minRating)} onchange={onrating}>
      <option value="0">Any</option>
      {#each [1, 2, 3, 4, 5] as n (n)}
        <option value={String(n)}>{n}+ stars</option>
      {/each}
    </select>
  </label>
  <label class="field">
    Flag
    <select value={library.filter.flag} onchange={onflag}>
      {#each FLAGS as f (f.value)}
        <option value={f.value}>{f.label}</option>
      {/each}
    </select>
  </label>
  <label class="field">
    Label
    <select value={library.filter.colorLabel} onchange={onlabel}>
      {#each LABELS as f (f.value)}
        <option value={f.value}>{f.label}</option>
      {/each}
    </select>
  </label>

  <div class="swatches" role="group" aria-label="Color label of the selection">
    {#each LABEL_BUTTONS as color (color)}
      <button
        type="button"
        class="swatch {color}"
        aria-label="Set label {color}"
        title="Label {color}"
        disabled={library.selection.ids.size === 0}
        onclick={() => void library.setColorLabel(color)}
      ></button>
    {/each}
    <button
      type="button"
      class="swatch none"
      aria-label="Remove label"
      title="Remove label"
      disabled={library.selection.ids.size === 0}
      onclick={() => void library.setColorLabel('none')}
    >
      &times;
    </button>
  </div>

  <span class="status" aria-live="polite">
    {library.total} photos{library.selection.ids.size > 0 ? `, ${library.selection.ids.size} selected` : ''}
  </span>
  <span class="mode">{mode}</span>
</div>

<style>
  .toolbar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-3) var(--space-4);
    padding: var(--space-3) var(--space-4);
    border-bottom: 1px solid var(--color-border);
    background: var(--color-bg-panel);
    font-size: var(--font-size-sm);
  }
  .import,
  .direction,
  select {
    background: var(--color-bg-panel-raised);
    border: 1px solid var(--color-border-strong);
    border-radius: var(--radius-sm);
    padding: var(--space-2) var(--space-3);
    font: inherit;
    cursor: pointer;
  }
  .import:hover:not(:disabled),
  .direction:hover {
    background: var(--color-bg-hover);
  }
  .import:disabled {
    cursor: default;
    color: var(--color-text-muted);
  }
  .search {
    width: 200px;
    padding: var(--space-2) var(--space-3);
    background: var(--color-bg-app);
    border: 1px solid var(--color-border-strong);
    border-radius: var(--radius-sm);
    color: var(--color-text);
    font: inherit;
    user-select: text;
    -webkit-user-select: text;
  }
  .search::placeholder {
    color: var(--color-text-faint);
  }
  .field {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    color: var(--color-text-muted);
  }
  select {
    color: var(--color-text);
  }
  .swatches {
    display: flex;
    gap: var(--space-2);
  }
  .swatch {
    width: 16px;
    height: 16px;
    padding: 0;
    border-radius: 50%;
    border: 1px solid var(--color-border-strong);
    line-height: 1;
    font-size: var(--font-size-xs);
    background: var(--color-bg-panel-raised);
    cursor: pointer;
  }
  .swatch:disabled {
    opacity: 0.4;
    cursor: default;
  }
  .swatch.red {
    background: var(--color-label-red);
  }
  .swatch.yellow {
    background: var(--color-label-yellow);
  }
  .swatch.green {
    background: var(--color-label-green);
  }
  .swatch.blue {
    background: var(--color-label-blue);
  }
  .swatch.purple {
    background: var(--color-label-purple);
  }
  .status {
    margin-left: auto;
    color: var(--color-text-muted);
  }
  .mode {
    color: var(--color-text-muted);
  }
</style>
