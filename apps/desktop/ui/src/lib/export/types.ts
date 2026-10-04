/**
 * Types of the export commands of the Rust backend.
 *
 * They match the serde (camelCase) JSON of the desktop shell export
 * commands. Keep them in sync.
 */
import type { PhotoId } from '../catalog';

export type JpegSubsampling = '444' | '422' | '420';
export type BitDepth = 8 | 16;
export type TiffCompression = 'none' | 'lzw' | 'deflate';
export type ExportColorSpace = 'srgb' | 'display_p3' | 'oprgb' | 'prophoto_rgb' | 'rec2020';

export type ExportFormat =
  | { kind: 'jpeg'; quality: number; subsampling: JpegSubsampling }
  | { kind: 'png'; bitDepth: BitDepth }
  | { kind: 'tiff'; bitDepth: BitDepth; compression: TiffCompression };

export type ExportSize =
  | { mode: 'original' }
  | { mode: 'longEdge' | 'shortEdge' | 'width' | 'height'; pixels: number }
  | { mode: 'megapixels'; megapixels: number }
  | { mode: 'percent'; percent: number };

export type ExportNaming = { mode: 'original' } | { mode: 'suffix'; suffix: string };
export type ExportConflict = 'uniqueName' | 'overwrite' | 'skip';

export interface ExportRequest {
  photoIds: PhotoId[];
  destination: string;
  format: ExportFormat;
  colorSpace: ExportColorSpace;
  size: ExportSize;
  allowUpscale: boolean;
  naming: ExportNaming;
  onConflict: ExportConflict;
  /** Applies to 8-bit output only. */
  dither: boolean;
}

export type ExportEvent =
  | { kind: 'started'; jobId: number; total: number }
  | { kind: 'photoDone'; jobId: number; photoId: PhotoId; outputPath: string; index: number; total: number }
  | { kind: 'photoSkipped'; jobId: number; photoId: PhotoId; outputPath: string; reason: string }
  | { kind: 'photoFailed'; jobId: number; photoId: PhotoId; message: string }
  | {
      kind: 'finished';
      jobId: number;
      written: number;
      skipped: number;
      failed: number;
      cancelled: boolean;
    };

/** Limits of the request. They are the same as in the backend. */
export const EXPORT_LIMITS = {
  maxPhotos: 10_000,
  minPixels: 1,
  maxPixels: 65_535,
  maxMegapixels: 1000,
  minPercent: 1,
  maxPercent: 400,
  minQuality: 1,
  maxQuality: 100,
  maxSuffixLength: 64,
} as const;
