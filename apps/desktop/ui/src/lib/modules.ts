/**
 * The application modules. This file has no runtime dependencies, so that
 * pure logic (for example the shortcut registry) can use the module types.
 */

/** The module identifiers, in the order of the module switcher. */
export const MODULE_IDS = ['library', 'develop', 'editor'] as const;

/** One application module. */
export type ModuleId = (typeof MODULE_IDS)[number];

/** Display labels of the modules. */
export const MODULE_LABELS: Readonly<Record<ModuleId, string>> = {
  library: 'Library',
  develop: 'Develop',
  editor: 'Editor',
};

/** True if the value is a module identifier. */
export function isModuleId(value: unknown): value is ModuleId {
  return typeof value === 'string' && (MODULE_IDS as readonly string[]).includes(value);
}
