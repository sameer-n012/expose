/**
 * The contract of the Editor commands of the shell, and typed wrappers for
 * them. All JSON uses camelCase, as the shell sends it.
 */
import { invoke } from '@tauri-apps/api/core';
import type { DisplaySpace } from '../develop/transport';

/** The blend modes, as the shell names them. */
export type BlendModeId =
  | 'normal'
  | 'dissolve'
  | 'darken'
  | 'multiply'
  | 'colorBurn'
  | 'linearBurn'
  | 'darkerColor'
  | 'lighten'
  | 'screen'
  | 'colorDodge'
  | 'linearDodge'
  | 'lighterColor'
  | 'overlay'
  | 'softLight'
  | 'hardLight'
  | 'vividLight'
  | 'linearLight'
  | 'pinLight'
  | 'hardMix'
  | 'difference'
  | 'exclusion'
  | 'subtract'
  | 'divide'
  | 'hue'
  | 'saturation'
  | 'color'
  | 'luminosity';

export interface LevelsChannelDto {
  inputBlack: number;
  inputWhite: number;
  gamma: number;
  outputBlack: number;
  outputWhite: number;
}

export interface CurvePointDto {
  x: number;
  y: number;
}

export type AdjustmentDto =
  | { type: 'brightnessContrast'; brightness: number; contrast: number }
  | { type: 'levels'; master: LevelsChannelDto; red: LevelsChannelDto; green: LevelsChannelDto; blue: LevelsChannelDto }
  | { type: 'curves'; master: CurvePointDto[]; red: CurvePointDto[]; green: CurvePointDto[]; blue: CurvePointDto[] }
  | { type: 'hueSaturation'; hue: number; saturation: number; lightness: number };

export type LayerKind = 'pixel' | 'adjustment' | 'group';

export interface LocksDto {
  transparency: boolean;
  pixels: boolean;
  position: boolean;
  all: boolean;
}

export interface LayerInfo {
  id: number;
  name: string;
  kind: LayerKind;
  visible: boolean;
  /** 0..1 */
  opacity: number;
  /** 0..1 */
  fillOpacity: number;
  blendMode: BlendModeId;
  hasMask: boolean;
  /** For kind `adjustment`. */
  adjustment: AdjustmentDto | null;
  /** For kind `group`: false means pass-through. */
  isolated: boolean | null;
  /** For kind `group`, bottom-first. */
  children: LayerInfo[] | null;
  /** The layer's own locks (a group's locks also apply to its children). */
  locks: LocksDto;
}

export interface DocumentInfo {
  documentId: number;
  width: number;
  height: number;
  /** The root list, bottom-first (index 0 is the bottom layer). */
  layers: LayerInfo[];
  canUndo: boolean;
  canRedo: boolean;
  /** Changes at every document change. */
  revision: number;
  /** True if the document changed after the last save (or the open). */
  dirty: boolean;
  /** The library photo of the last save, or null if the document has no saved files. */
  savedPhotoId: number | null;
  /** The library photo the document was opened from; null for a new document. */
  sourcePhotoId: number | null;
}

/** The result of `editor_save`. */
export interface SaveResult {
  /** The library photo of the flattened image. */
  photoId: number;
  /** The absolute path of the layered `.exdoc` file. */
  documentPath: string;
  /** The absolute path of the flattened TIFF. */
  imagePath: string;
  /** True if this save made the files and the library photo. */
  created: boolean;
  /** The document info after the save. */
  info: DocumentInfo;
}

export type EditDto =
  | { type: 'setVisible'; layerId: number; visible: boolean }
  | { type: 'setOpacity'; layerId: number; opacity: number }
  | { type: 'setFillOpacity'; layerId: number; fillOpacity: number }
  | { type: 'setBlendMode'; layerId: number; mode: BlendModeId }
  | { type: 'rename'; layerId: number; name: string }
  /** `index` is in the bottom-first list of the parent, after the layer is removed. */
  | { type: 'move'; layerId: number; parentId: number | null; index: number }
  | { type: 'delete'; layerId: number }
  | { type: 'duplicate'; layerId: number }
  /** `aboveLayerId` null: the top of the root list. */
  | { type: 'addAdjustment'; adjustment: AdjustmentDto; aboveLayerId: number | null }
  | { type: 'setAdjustment'; layerId: number; adjustment: AdjustmentDto }
  /** `aboveLayerId` null: the top of the root list. A transparent canvas-size layer named "Layer N". */
  | { type: 'addPixelLayer'; aboveLayerId: number | null }
  | { type: 'setLocks'; layerId: number; locks: LocksDto };

export type BackgroundId = 'white' | 'black' | 'transparent';

export interface NewDocumentSpec {
  /** Non-empty, max 120 chars, no '/', '\\', ':' or control chars (it becomes a file name). */
  name: string;
  /** 1..=30000 (integer). */
  width: number;
  /** 1..=30000; width * height must be at most MAX_PIXELS. */
  height: number;
  /** 1..=10000 */
  resolutionPpi: number;
  bitDepth: 8 | 16 | 32;
  background: BackgroundId;
}

/** The area of one frame request. */
export interface RenderArea {
  /** Canvas coordinate at the top-left output pixel. */
  originX: number;
  originY: number;
  /** Output pixels per canvas pixel. */
  scale: number;
  /** Output size in pixels. */
  width: number;
  height: number;
}

/** The backend calls of the Editor. Tests replace them. */
export interface EditorApi {
  open(photoId: number, displaySpace: DisplaySpace | null): Promise<DocumentInfo>;
  newDocument(spec: NewDocumentSpec, displaySpace: DisplaySpace | null): Promise<DocumentInfo>;
  render(documentId: number, area: RenderArea): Promise<ArrayBuffer>;
  edit(documentId: number, edit: EditDto, gesture: number | null): Promise<DocumentInfo>;
  undo(documentId: number): Promise<DocumentInfo>;
  redo(documentId: number): Promise<DocumentInfo>;
  /** Saves the document. With `folder` ("Save As"), it makes new files in that folder. */
  save(documentId: number, folder?: string): Promise<SaveResult>;
  close(documentId: number): Promise<void>;
}

/** The real calls through `invoke`. */
export const editorApi: EditorApi = {
  open: (photoId, displaySpace) => invoke<DocumentInfo>('editor_open', { photoId, displaySpace }),
  newDocument: (spec, displaySpace) => invoke<DocumentInfo>('editor_new', { spec, displaySpace }),
  render: (documentId, area) => invoke<ArrayBuffer>('editor_render', { documentId, ...area }),
  edit: (documentId, edit, gesture) => invoke<DocumentInfo>('editor_edit', { documentId, edit, gesture }),
  undo: (documentId) => invoke<DocumentInfo>('editor_undo', { documentId }),
  redo: (documentId) => invoke<DocumentInfo>('editor_redo', { documentId }),
  save: (documentId, folder) => invoke<SaveResult>('editor_save', { documentId, folder: folder ?? null }),
  close: async (documentId) => {
    await invoke<null>('editor_close', { documentId });
  },
};
