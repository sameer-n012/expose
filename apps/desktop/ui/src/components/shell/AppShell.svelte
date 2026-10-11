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
  import { cropModeActions, cropToggleAction, registerActions } from '../../lib/develop/cropActions';
  import BasicPanel from '../develop/BasicPanel.svelte';
  import CropPanel from '../develop/CropPanel.svelte';
  import DetailPanel from '../develop/DetailPanel.svelte';
  import HistogramView from '../develop/HistogramView.svelte';
  import ResetAllPanel from '../develop/ResetAllPanel.svelte';
  import ToneCurvePanel from '../develop/ToneCurvePanel.svelte';
  import DevelopView from '../modules/DevelopView.svelte';
  import { MODULE_LABELS, type ModuleId } from '../../lib/modules';
  import type { ShortcutRegistry } from '../../lib/shortcuts/registry';
  import type { LayoutStore, Region, Side } from '../../lib/stores/layout.svelte';
  import type { LibraryStore } from '../../lib/stores/library.svelte';
  import type { ModuleStore } from '../../lib/stores/modules.svelte';
  import type { ToastStore } from '../../lib/stores/toasts.svelte';
  import FolderPanel from '../library/FolderPanel.svelte';
  import { EditorStore } from '../../lib/stores/editor.svelte';
  import { EDITOR_ACTION, editInEditorAction, editorActions } from '../../lib/editor/editorActions';
  import AdjustmentPanel from '../editor/AdjustmentPanel.svelte';
  import EditInEditorButton from '../editor/EditInEditorButton.svelte';
  import EditorView from '../editor/EditorView.svelte';
  import LayersPanel from '../editor/LayersPanel.svelte';
  import LeaveDialog from '../editor/LeaveDialog.svelte';
  import { guardWindowClose, tauriCloseListener } from '../../lib/editor/closeGuard';
  import { bumpThumbnailRevision } from '../../lib/library/thumbnailRevisions.svelte';
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

  // The shell owns the editor store, too. After a save, the library shows
  // the new photo and loads the new thumbnail.
  const editor = untrack(
    () =>
      new EditorStore({
        library,
        modules,
        toasts,
        onSaved: (result) => {
          bumpThumbnailRevision(result.photoId);
          void library.refresh();
        },
      }),
  );

  // While the document has unsaved changes, a window close asks first.
  $effect(() => {
    if (!editor.dirty) {
      return;
    }
    return guardWindowClose(
      tauriCloseListener,
      () => editor.confirmLeave(),
      (error) => console.warn('The window close guard is not available.', error),
    );
  });

  /** Opens the active photo (Develop) or the focused photo (else) in the Editor. */
  function editInEditor(): void {
    const photo = modules.active === 'develop' ? develop.target : editor.candidate;
    void editor.editInEditor(photo ?? editor.candidate);
  }

  $effect(() => registry.register(editInEditorAction(editInEditor)));
  // Save, undo, redo, and zoom work in the Editor module only.
  $effect(() => registerActions(registry, editorActions(editor)));

  // The crop tool: `R` always; Enter, Esc, O, and X only while it is active.
  $effect(() => registry.register(cropToggleAction(develop)));
  $effect(() => {
    if (!develop.cropActive) {
      return;
    }
    return registerActions(registry, cropModeActions(develop));
  });

  // `Mod+Shift+R` resets all develop settings (a no-op at defaults).
  $effect(() =>
    registry.register({
      id: 'develop.resetAll',
      label: 'Reset all develop settings',
      scope: 'develop',
      defaultBinding: 'Mod+Shift+R',
      run: () => develop.resetAll(),
    }),
  );

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
          <HistogramView histogram={develop.histogram} />
          <CropPanel {develop} />
          <BasicPanel {develop} />
          <ToneCurvePanel {develop} />
          <DetailPanel {develop} />
          <PanelSections sections={PANEL_SECTIONS.develop.right} />
          <ResetAllPanel {develop} />
          <div class="edit-footer">
            <EditInEditorButton
              onedit={editInEditor}
              disabled={develop.target === null}
              shortcut={shortcutFor(EDITOR_ACTION.edit)}
            />
          </div>
        {:else if id === 'editor' && side === 'right'}
          <LayersPanel {editor} />
          <AdjustmentPanel {editor} />
          <PanelSections sections={PANEL_SECTIONS.editor.right} />
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
      onswitch={(id) => modules.activateTab(id)}
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
            <LibraryView {modules} {library} onexport={exportPhotos} onedit={editInEditor} />
          {:else if id === 'develop'}
            <DevelopView {develop} />
          {:else}
            <EditorView {editor} {toasts} />
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
{#if editor.leavePrompt !== null}
  <LeaveDialog {editor} />
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
  .edit-footer {
    padding: 0 var(--space-4) var(--space-3);
  }
  .edge-topbar,
  .edge-filmstrip {
    height: var(--space-3);
  }
</style>
