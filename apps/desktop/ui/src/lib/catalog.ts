/**
 * Typed wrappers for the catalog commands of the Rust backend.
 *
 * The types match the serde (camelCase) JSON of `expose-catalog` and of the
 * desktop shell commands in `src-tauri/src/commands/`. Keep them in sync.
 */
import { invoke } from '@tauri-apps/api/core';

export type PhotoId = number;
export type FolderId = number;
export type Flag = 'none' | 'pick' | 'reject';
export type ColorLabel = 'none' | 'red' | 'yellow' | 'green' | 'blue' | 'purple';
export type SortField = 'captureTime' | 'fileName' | 'rating' | 'importTime';
export type SortDirection = 'ascending' | 'descending';

export type ErrorKind = 'notFound' | 'invalidInput' | 'database' | 'io' | 'unavailable' | 'internal';

/** Structured error that a catalog command returns when it fails. */
export interface CommandError {
  kind: ErrorKind;
  message: string;
}

/**
 * Returns true if the value is a structured command error.
 *
 * Note: when Tauri cannot parse the arguments, it rejects with a plain string
 * before the command runs. That value is not a `CommandError`.
 */
export function isCommandError(value: unknown): value is CommandError {
  return typeof value === 'object' && value !== null && 'kind' in value && 'message' in value;
}

export type CatalogStatus =
  | { state: 'ready'; path: string | null; schemaVersion: number; photoCount: number }
  | { state: 'unavailable'; path: string | null; error: string };

export interface Folder {
  id: FolderId;
  parentId: FolderId | null;
  path: string;
  name: string;
}

export interface FolderNode {
  folder: Folder;
  photoCount: number;
  children: FolderNode[];
}

export interface Photo {
  id: PhotoId;
  folderId: FolderId;
  fileName: string;
  fileSize: number;
  fileModifiedMs: number;
  importedMs: number;
  captureTimeMs: number | null;
  width: number | null;
  height: number | null;
  rating: number;
  flag: Flag;
  colorLabel: ColorLabel;
}

export interface PhotoQuery {
  folderId: FolderId;
  recursive?: boolean;
  sort?: { field?: SortField; direction?: SortDirection };
  filter?: { minRating?: number | null; flag?: Flag | null; colorLabel?: ColorLabel | null };
  limit?: number | null;
  offset?: number;
}

export interface PhotoPage {
  photos: Photo[];
  total: number;
}

export interface ImportIssue {
  path: string;
  message: string;
}

export interface ImportReport {
  root: string;
  added: number;
  skipped: number;
  ignored: number;
  addedIds: PhotoId[];
  errors: ImportIssue[];
}

export function catalogStatus(): Promise<CatalogStatus> {
  return invoke<CatalogStatus>('catalog_status');
}

/** Imports a folder in place. The path must be absolute. */
export function importFolder(path: string, recursive: boolean): Promise<ImportReport> {
  return invoke<ImportReport>('import_folder', { path, recursive });
}

export function folderTree(): Promise<FolderNode[]> {
  return invoke<FolderNode[]>('folder_tree');
}

export function listPhotos(query: PhotoQuery): Promise<PhotoPage> {
  return invoke<PhotoPage>('list_photos', { query });
}

export function getPhoto(id: PhotoId): Promise<Photo> {
  return invoke<Photo>('get_photo', { id });
}

export function setRating(ids: PhotoId[], rating: number): Promise<void> {
  return invoke<void>('set_rating', { ids, rating });
}

export function setFlag(ids: PhotoId[], flag: Flag): Promise<void> {
  return invoke<void>('set_flag', { ids, flag });
}

export function setColorLabel(ids: PhotoId[], label: ColorLabel): Promise<void> {
  return invoke<void>('set_color_label', { ids, label });
}

/** Shows the native folder picker. Returns null if the user cancels. */
export function pickFolder(): Promise<string | null> {
  return invoke<string | null>('pick_folder');
}
