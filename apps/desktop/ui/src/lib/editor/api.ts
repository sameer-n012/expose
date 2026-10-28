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
  /** True if the layer has a mask and the mask is on. False if there is no mask. */
  maskEnabled: boolean;
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
  /** True if the document has a selection. */
  hasSelection: boolean;
  /** Changes at each selection change, also at undo and redo. */
  selectionVersion: number;
}

/** How a new selection shape joins the selection that exists. */
export type SelectionModeId = 'replace' | 'add' | 'subtract' | 'intersect';

/** A selection shape, in canvas pixels. */
export type SelectionShapeDto =
  | { type: 'rect'; x: number; y: number; width: number; height: number }
  /** The box around the ellipse. */
  | { type: 'ellipse'; x: number; y: number; width: number; height: number }
  /** 3 or more points. */
  | { type: 'polygon'; points: [number, number][] };

/** The outline of the selection (for the marching ants). */
export interface SelectionOutline {
  selectionVersion: number;
  /** The step used (can be larger than the requested step). */
  step: number;
  /** Closed outlines. Each one is flat: [x0, y0, x1, y1, ...] in canvas pixels. */
  contours: number[][];
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
  | { type: 'setLocks'; layerId: number; locks: LocksDto }
  /** A canvas-size mask: `reveal` is all white, `hide` is all black. It is an error if the layer has a mask. */
  | { type: 'addMask'; layerId: number; fill: MaskFillId }
  | { type: 'deleteMask'; layerId: number }
  | { type: 'setMaskEnabled'; layerId: number; enabled: boolean }
  /** `feather` is 0..=250 px. */
  | { type: 'selectShape'; shape: SelectionShapeDto; mode: SelectionModeId; antiAlias: boolean; feather: number }
  | { type: 'selectAll' }
  /** Deselect, invert, and feather are errors when there is no selection. */
  | { type: 'deselect' }
  | { type: 'invertSelection' }
  /** `radius` is 0..=250 px. */
  | { type: 'featherSelection'; radius: number }
  /** Replaces a mask that exists. Without a selection it makes a reveal-all mask. */
  | { type: 'maskFromSelection'; layerId: number };

/** The start state of a new layer mask. */
export type MaskFillId = 'reveal' | 'hide';

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
  /** Id of a layer with a mask: the frame shows its mask as a gray image. Absent or null: the composite. */
  showMaskLayerId?: number | null;
}

/** What a stroke changes. */
export type StrokeTargetDto =
  /** Gamma-encoded sRGB, 0..1. The shell converts it to linear Rec. 2020. */
  | { type: 'paint'; color: [number, number, number] }
  | { type: 'erase' }
  /** 0 hides, 1 reveals. */
  | { type: 'mask'; value: number };

/** The brush of a stroke. */
export interface BrushSettingsDto {
  /** Diameter, 1..5000 canvas pixels. */
  size: number;
  /** 0..1 */
  hardness: number;
  /** 0..1 */
  opacity: number;
  /** 0..1 */
  flow: number;
  /** A fraction of the size, 0.01..10. */
  spacing: number;
  pressureSize: boolean;
  pressureOpacity: boolean;
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
  /** The outline of the selection. `step` is the sample step in canvas pixels (1 or more). */
  selectionOutline(documentId: number, step: number): Promise<SelectionOutline>;
  /** Starts a stroke on a layer. Returns the stroke id. One stroke is active in a document. */
  strokeBegin(documentId: number, layerId: number, target: StrokeTargetDto, brush: BrushSettingsDto): Promise<number>;
  /** Adds points: a flat list [x, y, pressure, ...] in canvas pixels. 1 to 3000 points per call. */
  strokePoints(documentId: number, strokeId: number, points: number[]): Promise<null>;
  /** Ends the stroke. It is one undo step. */
  strokeEnd(documentId: number, strokeId: number): Promise<DocumentInfo>;
  /** Drops the stroke. */
  strokeCancel(documentId: number, strokeId: number): Promise<DocumentInfo>;
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
  selectionOutline: (documentId, step) => invoke<SelectionOutline>('editor_selection_outline', { documentId, step }),
  strokeBegin: (documentId, layerId, target, brush) => invoke<number>('editor_stroke_begin', { documentId, layerId, target, brush }),
  strokePoints: (documentId, strokeId, points) => invoke<null>('editor_stroke_points', { documentId, strokeId, points }),
  strokeEnd: (documentId, strokeId) => invoke<DocumentInfo>('editor_stroke_end', { documentId, strokeId }),
  strokeCancel: (documentId, strokeId) => invoke<DocumentInfo>('editor_stroke_cancel', { documentId, strokeId }),
};
