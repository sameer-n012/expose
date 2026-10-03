<!--
  The top bar: app name, module switcher, and panel toggle buttons.
-->
<script lang="ts">
  import { MODULE_IDS, MODULE_LABELS, type ModuleId } from '../../lib/modules';
  import type { Region } from '../../lib/stores/layout.svelte';

  interface Props {
    active: ModuleId;
    /** Shortcut text per module, for the tooltip. Null if the module has no shortcut. */
    shortcuts: Readonly<Record<ModuleId, string | null>>;
    visible: Readonly<Record<Region, boolean>>;
    version: string;
    onswitch: (id: ModuleId) => void;
    ontoggle: (region: Region) => void;
  }

  let { active, shortcuts, visible, version, onswitch, ontoggle }: Props = $props();

  const toggles: ReadonlyArray<{ region: Region; label: string; glyph: string }> = [
    { region: 'left', label: 'left panel', glyph: '◧' },
    { region: 'filmstrip', label: 'filmstrip', glyph: '⬓' },
    { region: 'right', label: 'right panel', glyph: '◨' },
  ];

  function tooltip(id: ModuleId): string {
    const key = shortcuts[id];
    return key === null ? MODULE_LABELS[id] : `${MODULE_LABELS[id]} (${key})`;
  }
</script>

<header class="topbar" data-testid="topbar">
  <div class="brand">
    <span class="name">Expose</span>
    {#if version !== ''}
      <span class="version">{version}</span>
    {/if}
  </div>

  <nav class="switcher" aria-label="Modules">
    {#each MODULE_IDS as id (id)}
      <button
        type="button"
        class="module"
        class:active={id === active}
        aria-current={id === active ? 'page' : undefined}
        title={tooltip(id)}
        onclick={() => onswitch(id)}
      >
        {MODULE_LABELS[id]}
      </button>
    {/each}
  </nav>

  <div class="toggles">
    {#each toggles as t (t.region)}
      <button
        type="button"
        class="toggle"
        aria-pressed={visible[t.region]}
        aria-label={t.label.charAt(0).toUpperCase() + t.label.slice(1)}
        title="Show or hide the {t.label}"
        onclick={() => ontoggle(t.region)}
      >
        {t.glyph}
      </button>
    {/each}
  </div>
</header>

<style>
  .topbar {
    height: var(--topbar-height);
    display: grid;
    grid-template-columns: 1fr auto 1fr;
    align-items: center;
    padding: 0 var(--space-4);
    background: var(--color-bg-panel);
    border-bottom: 1px solid var(--color-border);
  }
  .brand {
    display: flex;
    align-items: baseline;
    gap: var(--space-3);
  }
  .name {
    font-size: var(--font-size-lg);
    font-weight: var(--font-weight-strong);
    letter-spacing: 0.02em;
  }
  .version {
    font-size: var(--font-size-xs);
    color: var(--color-text-faint);
  }
  .switcher {
    display: flex;
    gap: var(--space-1);
  }
  .module {
    background: none;
    border: none;
    border-bottom: 2px solid transparent;
    padding: var(--space-2) var(--space-4);
    font-size: var(--font-size-lg);
    color: var(--color-text-muted);
    cursor: pointer;
  }
  .module:hover {
    color: var(--color-text);
  }
  .module.active {
    color: var(--color-text);
    border-bottom-color: var(--color-accent);
  }
  .toggles {
    justify-self: end;
    display: flex;
    gap: var(--space-1);
  }
  .toggle {
    width: 26px;
    height: 24px;
    background: none;
    border: 1px solid transparent;
    border-radius: var(--radius-sm);
    color: var(--color-text-faint);
    cursor: pointer;
  }
  .toggle:hover {
    background: var(--color-bg-hover);
  }
  .toggle[aria-pressed='true'] {
    color: var(--color-text);
    border-color: var(--color-border-strong);
  }
</style>
