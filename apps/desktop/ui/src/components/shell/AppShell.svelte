<!--
  The main window layout: top bar, left panel, center view, right panel,
  filmstrip, and toasts.

  Module views and panels stay mounted when they are not visible. They are
  hidden with the `hidden` attribute. A module view mounts on the first
  visit to the module (see ModuleStore). Thus module switches are instant
  and each module keeps its state.

  The shell sends window key events to the shortcut registry. The caller
  registers the actions (see createAppState).
-->
<script lang="ts">
  import { untrack } from 'svelte';
  import { APP_ACTION, exportAction } from '../../lib/appActions';
  import { DevelopStore } from '../../lib/stores/develop.svelte';
  import { ExportStore } from '../../lib/stores/export.svelte';
  import ExportDialog from '../export/ExportDialog.svelte';
  import ExportProgress from '../export/ExportProgress.svelte';
  import BasicPanel from '../develop/BasicPanel.svelte';
  import DevelopView from '../modules/DevelopView.svelte';
  import { MODULE_LABELS, type ModuleId } from '../../lib/modules';
  import type { ShortcutRegistry } from '../../lib/shortcuts/registry';
  import type { LayoutStore, Region, Side } from '../../lib/stores/layout.svelte';
  import type { LibraryStore } from '../../lib/stores/library.svelte';
  import type { ModuleStore } from '../../lib/stores/modules.svelte';
  import type { ToastStore } from '../../lib/stores/toasts.svelte';
  import FolderPanel from '../library/FolderPanel.svelte';
  import CanvasPlaceholder from '../modules/CanvasPlaceholder.svelte';
  import LibraryView from '../modules/LibraryView.svelte';
  import { PANEL_SECTIONS } from '../modules/panelSections';
  import Filmstrip from './Filmstrip.svelte';
  import PanelSections from './PanelSections.svelte';
  import SidePanel from './SidePanel.svelte';
  import ToastHost from './ToastHost.svelte';
  import TopBar from './TopBar.svelte';

  interface Props {
    modules: ModuleStore;
    layout: LayoutStore;
    library: LibraryStore;
    registry: ShortcutRegistry;
    toasts: ToastStore;
    /** Version text for the top bar. Empty while unknown. */
    version?: string;
  }

  let { modules, layout, library, registry, toasts, version = '' }: Props = $props();

  // The shell owns the develop store. The props do not change for the
  // life of the shell, so the store reads them once.
  const develop = untrack(() => new DevelopStore({ library, modules, toasts }));

  // The shell owns the export store, too.
  const exporter = untrack(() => new ExportStore({ toasts }));

  /**
   * Opens the export dialog. In Develop it exports the active photo; else it
   * exports the selected photos.
   */
  function exportPhotos(): void {
    const active = modules.active === 'develop' ? develop.target : null;
    exporter.openDialog(active === null ? library.selectedIds : [active]);
  }

  // The shortcut needs the shell, because the shell knows the active photo.
  $effect(() => registry.register(exportAction(exportPhotos)));

  /** Shortcut text per module. Null if the action is not registered or has no key. */
  function shortcutFor(actionId: string): string | null {
    return registry.has(actionId) ? registry.displayBinding(actionId) : null;
  }

  // The registry is not reactive. This value updates when the registry
  // prop changes; a future remap screen must also give a new value here.
  const moduleShortcuts: Readonly<Record<ModuleId, string | null>> = $derived({
    library: shortcutFor(APP_ACTION.libraryGrid),
    develop: shortcutFor(APP_ACTION.develop),
    editor: shortcutFor(APP_ACTION.editor),
  });

  const SIDE_LABELS: Readonly<Record<Side, string>> = { left: 'Left panel', right: 'Right panel' };
  const EDGE_LABELS: Readonly<Record<Region, string>> = {
    topbar: 'Show top bar',
    left: 'Show left panel',
    right: 'Show right panel',
    filmstrip: 'Show filmstrip',
  };

  function onkeydown(event: KeyboardEvent): void {
    if (registry.handleKeyDown(event, modules.active)) {
      event.preventDefault();
    }
  }
</script>

<svelte:window {onkeydown} />

{#snippet edge(region: Region)}
  {#if !layout.visible[region]}
    <button
      type="button"
      class="edge edge-{region}"
      aria-label={EDGE_LABELS[region]}
      title={EDGE_LABELS[region]}
      onclick={() => layout.toggle(region)}
    ></button>
  {/if}
{/snippet}

{#snippet panel(side: Side)}
  <SidePanel
    {side}
    label={SIDE_LABELS[side]}
    visible={layout.visible[side]}
    width={layout.width[side]}
    onresize={(w) => layout.setWidth(side, w)}
    onresetwidth={() => layout.resetWidth(side)}
  >
    {#each modules.mounted as id (id)}
      <div hidden={id !== modules.active}>
        {#if id === 'library' && side === 'left'}
          <FolderPanel {library} />
        {/if}
        {#if id === 'develop' && side === 'right'}
          {@const sections = PANEL_SECTIONS.develop.right}
          {@const basicAt = sections.indexOf('Basic')}
          <PanelSections sections={sections.slice(0, basicAt)} />
          <BasicPanel {develop} />
          <PanelSections sections={sections.slice(basicAt + 1)} />
        {:else}
          <PanelSections sections={PANEL_SECTIONS[id][side]} />
        {/if}
      </div>
    {/each}
  </SidePanel>
{/snippet}

<div class="shell">
  <div hidden={!layout.visible.topbar}>
    <TopBar
      active={modules.active}
      shortcuts={moduleShortcuts}
      visible={layout.visible}
      {version}
      onswitch={(id) => modules.switchTo(id)}
      ontoggle={(region) => layout.toggle(region)}
    />
  </div>
  {@render edge('topbar')}

  <div class="main">
    {@render edge('left')}
    {@render panel('left')}

    <main class="center">
      {#each modules.mounted as id (id)}
        <section
          class="module-view"
          hidden={id !== modules.active}
          aria-label={MODULE_LABELS[id]}
          data-testid="module-{id}"
        >
          {#if id === 'library'}
            <LibraryView {modules} {library} onexport={exportPhotos} />
          {:else if id === 'develop'}
            <DevelopView {develop} />
          {:else}
            <CanvasPlaceholder title="Editor" hint="The document canvas appears here." />
          {/if}
        </section>
      {/each}
    </main>

    {@render panel('right')}
    {@render edge('right')}
  </div>

  <Filmstrip visible={layout.visible.filmstrip} />
  {@render edge('filmstrip')}
</div>

{#if exporter.isOpen}
  <ExportDialog store={exporter} />
{/if}
<ExportProgress store={exporter} />
<ToastHost store={toasts} />

<style>
  .shell {
    height: 100%;
    display: flex;
    flex-direction: column;
    background: var(--color-bg-app);
  }
  .main {
    flex: 1;
    min-height: 0;
    display: flex;
  }
  .center {
    flex: 1;
    min-width: 0;
    position: relative;
    background: var(--color-bg-center);
  }
  .module-view {
    position: absolute;
    inset: 0;
    overflow: hidden;
  }
  /* A thin strip at the edge of a hidden region. A click shows the region. */
  .edge {
    flex: none;
    padding: 0;
    border: none;
    background: var(--color-bg-panel);
    cursor: pointer;
  }
  .edge:hover {
    background: var(--color-bg-hover);
  }
  .edge-left,
  .edge-right {
    width: var(--space-3);
  }
  .edge-topbar,
  .edge-filmstrip {
    height: var(--space-3);
  }
</style>
