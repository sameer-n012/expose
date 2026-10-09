/**
 * Placeholder section titles for the side panels of each module.
 * Real panel components replace these when the modules are implemented.
 */
import type { ModuleId } from '../../lib/modules';
import type { Side } from '../../lib/stores/layout.svelte';

export const PANEL_SECTIONS: Readonly<Record<ModuleId, Readonly<Record<Side, readonly string[]>>>> = {
  library: {
    // The folder tree (FolderPanel) shows above these sections.
    left: ['Collections'],
    right: ['Histogram', 'Quick develop', 'Keywords', 'Metadata'],
  },
  develop: {
    left: ['Navigator', 'Presets', 'Snapshots', 'History'],
    // The Histogram, Crop, Basic, Tone curve, and Detail panels (AppShell) show above these.
    right: ['Color', 'Optics'],
  },
  editor: {
    left: ['Tools'],
    // The Layers and Properties panels (AppShell) show above these.
    right: ['Color', 'History'],
  },
};
