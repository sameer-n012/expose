# Expose — Feature List

This file is the master list of features for Expose.

## How to use this file

- `[ ]` = not complete. `[x]` = complete.
- Mark a feature complete **only** when all of these are true:
  1. The feature is fully implemented (no stubs, no placeholder UI).
  2. Automated tests for the feature exist and pass.
  3. The feature is available to the user in the application.
- Do not mark a feature complete if it is only partly implemented. Track partial work in `TODO.md`.
- Priority tags help to plan the work:
  - **P1** — core. The application is not usable without it.
  - **P2** — standard. Most users expect it.
  - **P3** — advanced. Add after P1 and P2 are stable.

---

## 1. Library and Catalog

### 1.1 Import
- [ ] **P1** Import photos from folders (add in place, no copy)
- [ ] **P1** Import with copy/move to a managed folder structure (date-based templates)
- [ ] **P2** Import from memory cards and connected cameras
- [ ] **P2** Duplicate detection during import
- [ ] **P2** Apply metadata, keywords, and develop presets during import
- [ ] **P2** Rename files during import with naming templates
- [ ] **P3** Make backup copies to a second location during import
- [ ] **P3** Watched folders (automatic import)
- [ ] **P3** Tethered capture (control a connected camera and import each shot)

### 1.2 Catalog management
- [ ] **P1** Catalog database that stores photos, metadata, and edit parameters
- [ ] **P1** Folder tree that mirrors the file system
- [ ] **P1** Thumbnail and preview generation with a persistent cache
- [ ] **P2** Detect missing or moved files and relink them
- [ ] **P2** Synchronize a folder (find new, changed, and removed files)
- [ ] **P2** Catalog backup and integrity check
- [ ] **P2** Read and write edit settings to sidecar files (XMP)
- [ ] **P3** Export a subset as a new catalog, and merge catalogs
- [ ] **P3** Smart previews (small proxy files for editing when originals are offline)

### 1.3 Organization
- [ ] **P1** Star ratings (0–5)
- [ ] **P1** Pick / reject flags
- [ ] **P1** Color labels
- [ ] **P2** Collections (manual albums)
- [ ] **P2** Smart collections (rule-based albums)
- [ ] **P2** Collection sets (nested folders of collections)
- [ ] **P2** Hierarchical keywords, keyword sets, and synonyms
- [ ] **P2** Stacks (group related photos under one top photo)
- [ ] **P2** Virtual copies (several edit versions of one file, with no file copy)
- [ ] **P3** Auto-stack by capture time

### 1.4 Browse, filter, and search
- [ ] **P1** Grid view with adjustable thumbnail size
- [ ] **P1** Loupe view (single photo, fit and 1:1 zoom)
- [ ] **P1** Filmstrip
- [ ] **P1** Sort by capture date, file name, rating, and other fields
- [ ] **P2** Filter bar: text, attributes (rating, flag, label), and metadata columns
- [ ] **P2** Compare view (two photos side by side, synchronized zoom/pan)
- [ ] **P2** Survey view (several photos at once for selection)
- [ ] **P2** Full-screen and lights-out display modes
- [ ] **P3** Face detection and people tagging
- [ ] **P3** Content search (find photos by what they show)

### 1.5 Metadata
- [ ] **P1** Read and show EXIF data
- [ ] **P2** Edit IPTC metadata (title, caption, copyright, creator, location)
- [ ] **P2** Metadata presets
- [ ] **P2** Batch-edit metadata on many photos
- [ ] **P2** Correct capture time (time-zone shift, set a date)
- [ ] **P3** Map view: show GPS positions, geotag from a GPS track log

---

## 2. Develop (Non-Destructive RAW and Photo Processing)

### 2.1 Pipeline and formats
- [ ] **P1** Decode RAW files from major camera makers (demosaic, black/white levels, camera color matrix)
- [ ] **P1** Open JPEG, PNG, TIFF (8/16/32-bit), HEIF/HEIC, WebP
- [ ] **P1** Non-destructive edit stack (stored as parameters, original never changed)
- [ ] **P1** Linear, wide-gamut working color space
- [ ] **P1** Fast interactive preview while sliders move
- [ ] **P2** DNG read and write
- [ ] **P2** Camera color profiles (standard and creative), with custom profile import (DCP)
- [ ] **P2** Process-version support (old edits look the same after pipeline upgrades)
- [ ] **P3** Creative profiles based on LUTs (3D LUT import: .cube)

### 2.2 Basic adjustments
- [ ] **P1** White balance: temperature and tint, presets (as shot, daylight, cloudy, …), eyedropper picker
- [x] **P1** Exposure
- [x] **P1** Contrast
- [x] **P1** Highlights, shadows, whites, blacks
- [ ] **P1** Automatic tone
- [ ] **P2** Texture, clarity, dehaze
- [ ] **P2** Vibrance and saturation
- [ ] **P2** Black-and-white conversion with per-color mix

### 2.3 Tone and color
- [x] **P1** Tone curve: point curve (RGB and per channel)
- [ ] **P2** Tone curve: parametric (highlights, lights, darks, shadows with split points)
- [ ] **P2** HSL / color mixer (hue, saturation, luminance per color band)
- [ ] **P2** Color grading (shadows, midtones, highlights wheels; blending and balance)
- [ ] **P2** Targeted adjustment tool (drag on the photo to change a curve or color band)
- [ ] **P3** Point color (pick a specific color and shift it, with range control)
- [ ] **P3** Camera calibration (shadow tint, primary hue/saturation)

### 2.4 Detail
- [x] **P1** Sharpening (amount, radius, detail, masking)
- [ ] **P2** Luminance noise reduction
- [ ] **P2** Color noise reduction
- [ ] **P3** AI noise reduction (writes a new DNG)
- [ ] **P3** Raw detail enhancement / super-resolution (2× upscale)

### 2.5 Optics and geometry
- [x] **P1** Crop with aspect-ratio presets and overlays (rule of thirds, golden ratio, grid)
- [x] **P1** Straighten and rotate; flip
- [ ] **P2** Lens profile corrections (distortion, vignetting) from a lens database
- [ ] **P2** Remove chromatic aberration and color fringing
- [ ] **P2** Manual distortion and vignetting correction
- [ ] **P2** Perspective correction: automatic, level, vertical, guided (draw lines)
- [ ] **P2** Manual transform (vertical, horizontal, rotate, aspect, scale, offset)

### 2.6 Effects
- [ ] **P2** Post-crop vignette (styles, midpoint, roundness, feather, highlights)
- [ ] **P2** Film grain (amount, size, roughness)

### 2.7 Local adjustments and masks
- [ ] **P1** Brush mask (size, feather, flow, density, erase, auto-mask edges)
- [ ] **P1** Linear gradient mask
- [ ] **P1** Radial gradient mask
- [ ] **P2** Color range mask
- [ ] **P2** Luminance range mask
- [ ] **P2** Depth range mask (from depth maps when present)
- [ ] **P2** Combine masks (add, subtract, intersect, invert)
- [ ] **P2** Per-mask adjustments (all basic, tone, color, detail, and effect sliders)
- [ ] **P3** AI subject mask
- [ ] **P3** AI sky mask
- [ ] **P3** AI background mask
- [ ] **P3** AI object mask (draw around an object to select it)
- [ ] **P3** AI people masks (face, skin, hair, eyes, lips, teeth, clothing)
- [ ] **P3** Mask presets and mask copy between photos

### 2.8 Retouching in develop
- [ ] **P2** Spot heal and clone (circle and brush shapes, auto source finding)
- [ ] **P2** Visualize spots (high-contrast view to find dust)
- [ ] **P2** Red-eye and pet-eye correction
- [ ] **P3** AI object removal (remove distractions, people, reflections)

### 2.9 Workflow tools
- [ ] **P1** Undo/redo history with named steps
- [ ] **P1** Before/after view (split, side by side, toggle)
- [ ] **P1** Reset (all settings, or one panel)
- [ ] **P2** Snapshots (save a named edit state)
- [ ] **P2** Copy/paste settings with a choice of which settings
- [ ] **P2** Synchronize settings across many selected photos
- [ ] **P2** Develop presets (create, import, export, organize, amount slider)
- [x] **P2** Histogram with shadow/highlight clipping warnings
- [ ] **P2** Show clipping overlay on the photo
- [ ] **P3** Auto sync (edits apply to all selected photos while you edit)
- [ ] **P3** Match exposure across photos
- [ ] **P3** Soft-proofing (simulate output color space and paper; out-of-gamut warning)
- [ ] **P3** Reference view (show a reference photo next to the photo you edit)

### 2.10 Merges
- [ ] **P2** HDR merge (exposure bracket to a 32-bit result, with alignment and deghosting)
- [ ] **P2** Panorama merge (spherical, cylindrical, perspective projections; boundary warp; fill edges)
- [ ] **P3** HDR panorama merge (in one step)
- [ ] **P3** Focus stacking

---

## 3. Editor (Layer-Based Pixel Editing)

### 3.1 Documents and canvas
- [ ] **P1** New document (size, resolution, color mode, bit depth, background)
- [ ] **P1** Open and save native layered document format
- [ ] **P1** Zoom, pan, rotate view; fit and 100% view
- [ ] **P1** Multiple open documents (tabs)
- [ ] **P2** Canvas size and image size (with resampling methods)
- [ ] **P2** Rulers, guides, grid, snapping
- [ ] **P2** Color modes: RGB, grayscale, CMYK, Lab; 8/16/32 bits per channel
- [ ] **P2** Open and save layered PSD files (read and write, best effort)
- [ ] **P3** Artboards
- [ ] **P3** Navigator panel
- [ ] **P3** Recovery: autosave and crash recovery

### 3.2 Layers
- [ ] **P1** Pixel layers: add, delete, duplicate, reorder, rename
- [ ] **P1** Opacity and fill opacity
- [ ] **P1** Show/hide, lock (transparency, pixels, position, all)
- [ ] **P1** Blend modes (normal, multiply, screen, overlay, soft light, hard light, color dodge/burn, darken/lighten, difference, exclusion, hue, saturation, color, luminosity, and others)
- [ ] **P1** Layer masks (pixel masks)
- [ ] **P2** Vector masks
- [ ] **P2** Layer groups (nested) with group blend modes and group masks
- [ ] **P2** Clipping masks
- [ ] **P2** Merge down, merge visible, flatten, stamp visible
- [ ] **P2** Align and distribute layers
- [ ] **P2** Layer styles: drop shadow, inner shadow, outer/inner glow, bevel and emboss, stroke, color/gradient/pattern overlay, satin
- [ ] **P2** Blend-if (blend by luminance ranges of this layer and underlying layers)
- [ ] **P2** Fill layers (solid color, gradient, pattern)
- [ ] **P2** Embedded non-destructive layers (contain a source image or document; transforms and filters stay editable)
- [ ] **P3** Linked external layers (source file updates the document)
- [ ] **P3** Layer comps (save and restore visibility/position/style states)
- [ ] **P3** Auto-align layers and auto-blend layers

### 3.3 Adjustment layers (non-destructive)
- [ ] **P1** Brightness/contrast
- [ ] **P1** Levels
- [ ] **P1** Curves
- [ ] **P1** Hue/saturation
- [ ] **P2** Exposure
- [ ] **P2** Vibrance
- [ ] **P2** Color balance
- [ ] **P2** Black and white (per-color mix)
- [ ] **P2** Photo filter
- [ ] **P2** Channel mixer
- [ ] **P2** Color lookup (3D LUTs)
- [ ] **P2** Invert, posterize, threshold
- [ ] **P2** Gradient map
- [ ] **P2** Selective color
- [ ] **P3** Shadows/highlights (as an adaptive filter)
- [ ] **P3** HDR toning
- [ ] **P3** Match color, replace color
- [ ] **P3** Develop panel as a filter (apply all develop adjustments to a layer)

### 3.4 Selections
- [ ] **P1** Rectangle and ellipse marquee
- [ ] **P1** Lasso, polygonal lasso
- [ ] **P1** Magic wand (tolerance, contiguous, sample all layers)
- [ ] **P1** Select all, deselect, reselect, invert
- [ ] **P1** Add, subtract, intersect selection modes
- [ ] **P2** Magnetic lasso (edge snapping)
- [ ] **P2** Quick selection brush (grows to edges)
- [ ] **P2** Color range selection
- [ ] **P2** Modify selection: expand, contract, feather, border, smooth
- [ ] **P2** Transform selection
- [ ] **P2** Quick mask mode (paint a selection)
- [ ] **P2** Save and load selections as alpha channels
- [ ] **P2** Refine edge / select and mask workspace (edge detection, hair refinement, decontaminate colors)
- [ ] **P3** AI select subject
- [ ] **P3** AI select sky
- [ ] **P3** AI object selection (click or draw a box)
- [ ] **P3** Focus area selection

### 3.5 Painting and brushes
- [ ] **P1** Brush tool (size, hardness, opacity, flow, blend mode)
- [ ] **P1** Eraser
- [ ] **P1** Foreground/background colors, color picker, eyedropper
- [ ] **P1** Paint bucket
- [ ] **P1** Gradient tool (linear, radial, angle, reflected, diamond; gradient editor)
- [ ] **P2** Pencil tool
- [ ] **P2** Pressure and tilt support for graphics tablets
- [ ] **P2** Brush engine settings: spacing, shape dynamics, scattering, texture, dual brush, color dynamics, transfer, smoothing
- [ ] **P2** Brush presets; import ABR brushes
- [ ] **P2** Swatches panel
- [ ] **P2** Background eraser and magic eraser
- [ ] **P3** Mixer brush (wet paint simulation)
- [ ] **P3** History brush and art history brush
- [ ] **P3** Symmetry painting

### 3.6 Retouching
- [ ] **P1** Clone stamp (aligned/non-aligned, sample current/all layers)
- [ ] **P1** Healing brush (source-based, texture/tone blend)
- [ ] **P2** Spot healing brush (no source needed)
- [ ] **P2** Patch tool
- [ ] **P2** Content-aware move (move or extend an object and fill the gap)
- [ ] **P2** Content-aware fill (fill a selection from surrounding pixels)
- [ ] **P2** Red-eye tool
- [ ] **P2** Dodge, burn, sponge
- [ ] **P2** Blur, sharpen, smudge tools
- [ ] **P2** Clone source panel (several sources, offset, scale, rotation, overlay)
- [ ] **P3** Remove tool (AI distraction removal by brush stroke)
- [ ] **P3** AI generative fill and expand (fill a selection or extend the canvas from a text prompt; model source to be decided)

### 3.7 Transform and warp
- [ ] **P1** Free transform (move, scale, rotate) with reference point
- [ ] **P1** Crop tool (with straighten; delete or hide cropped pixels)
- [ ] **P2** Skew, distort, perspective
- [ ] **P2** Warp (grid and preset warp shapes)
- [ ] **P2** Flip and rotate canvas or layer
- [ ] **P2** Perspective crop
- [ ] **P2** Content-aware scale (protect important areas during resize)
- [ ] **P3** Pin-based mesh warp (place pins, bend the image around them)
- [ ] **P3** Perspective warp (define planes, then change perspective)
- [ ] **P3** Pixel-push distortion tool (forward warp, twirl, pucker, bloat; face-aware shape controls)

### 3.8 Filters
- [ ] **P1** Gaussian blur
- [ ] **P1** Unsharp mask
- [ ] **P2** Non-destructive filters on embedded layers (editable, with mask and blend mode)
- [ ] **P2** Blur: box, motion, radial, surface, lens blur (with depth map), smart blur
- [ ] **P2** Blur gallery: field, iris, tilt-shift, path, spin blur
- [ ] **P2** Sharpen: smart sharpen, high pass
- [ ] **P2** Noise: add noise, reduce noise, median, dust and scratches
- [ ] **P2** Distort: displace, pinch, polar coordinates, ripple, shear, spherize, twirl, wave, zigzag
- [ ] **P2** Stylize: emboss, find edges, solarize, tiles, wind, oil paint
- [ ] **P2** Render: clouds, difference clouds, lens flare, lighting effects
- [ ] **P2** Pixelate: mosaic, crystallize, color halftone, pointillize
- [ ] **P2** Other: offset, maximum, minimum, custom kernel
- [ ] **P2** Lens correction filter (camera/lens profile, manual)
- [ ] **P2** Adaptive wide-angle (straighten curved lines from wide/fisheye lenses)
- [ ] **P3** Filter gallery (artistic, brush stroke, sketch, texture effects with stacked preview)
- [ ] **P3** AI neural filters (skin smoothing, colorize, style transfer, depth blur, super zoom, JPEG artifact removal)
- [ ] **P3** Plug-in filter API for third-party filters

### 3.9 Text
- [ ] **P2** Point text and paragraph (area) text
- [ ] **P2** Font family, style, size, color, alignment
- [ ] **P2** Character settings: tracking, kerning, leading, baseline shift, case, OpenType features
- [ ] **P2** Paragraph settings: alignment, indents, spacing, hyphenation
- [ ] **P2** Text on a path and inside a shape
- [ ] **P2** Warp text
- [ ] **P3** Character and paragraph styles
- [ ] **P3** Vertical text
- [ ] **P3** Variable font support

### 3.10 Vector shapes and paths
- [ ] **P2** Shape tools: rectangle (with corner radius), ellipse, polygon, star, line, custom shapes
- [ ] **P2** Shape layers with fill and stroke settings
- [ ] **P2** Pen tool and freeform pen
- [ ] **P2** Path selection and direct selection; add/delete/convert anchor points
- [ ] **P2** Paths panel: save, stroke path, fill path, path to selection, selection to path
- [ ] **P2** Path operations (combine, subtract, intersect, exclude)
- [ ] **P3** Curvature pen
- [ ] **P3** Import and export SVG shapes

### 3.11 Channels and color
- [ ] **P2** Channels panel (view per channel, alpha channels, spot channels)
- [ ] **P2** Apply image and calculations (blend channels)
- [ ] **P2** Convert between color modes and profiles
- [ ] **P2** ICC color management: assign profile, convert to profile, rendering intents
- [ ] **P2** Info panel (color values under the cursor, sampler points)
- [ ] **P3** Proof colors (soft-proof with gamut warning)
- [ ] **P3** Indexed color and duotone modes

### 3.12 History and automation
- [ ] **P1** Undo/redo with history panel
- [ ] **P2** History snapshots
- [ ] **P2** Record and play macros (recorded tool and menu steps)
- [ ] **P2** Batch processing (run a macro on a folder of files)
- [ ] **P3** Scripting API
- [ ] **P3** Variables / data-driven graphics (generate many versions from a data table)
- [ ] **P3** Image processor (batch convert and resize)
- [ ] **P3** Contact sheet generator

### 3.13 Other editor tools
- [ ] **P2** Move tool (auto-select layer, show transform controls)
- [ ] **P2** Measure tool (ruler, angle)
- [ ] **P2** Count tool and notes
- [ ] **P2** Slices for web export
- [ ] **P3** Vanishing point (clone and paint in perspective planes)
- [ ] **P3** 3D / video timeline (out of scope until the operator approves)

---

## 4. Integration Between Library/Develop and Editor

- [ ] **P1** Open a developed photo in the editor (develop settings applied, 16-bit)
- [ ] **P1** Save the editor result back to the library, stacked with the original
- [ ] **P2** Open several photos as layers in one document
- [ ] **P2** Open as an embedded non-destructive layer (develop settings stay editable)
- [ ] **P3** Send to merges (HDR, panorama) from the editor

---

## 5. Output

### 5.1 Export
- [x] **P1** Export to JPEG, PNG, TIFF
- [x] **P1** Size options: long edge, short edge, width/height, megapixels, percent
- [x] **P1** Quality and bit-depth options
- [x] **P1** Output color space (sRGB, Display P3, opRGB, ProPhoto RGB)
- [ ] **P2** Export to WebP, HEIF/HEIC, AVIF, JPEG XL, DNG, PSD
- [ ] **P2** File naming templates and destination folder rules
- [ ] **P2** Output sharpening (screen, matte paper, glossy paper)
- [ ] **P2** Metadata options (include all, copyright only, remove location)
- [ ] **P2** Watermarks (text and image)
- [ ] **P2** Export presets
- [ ] **P2** Background export queue with progress
- [ ] **P3** Post-export actions (open in another application, run a script)
- [ ] **P3** Quick export of layers or artboards as separate files

### 5.2 Print
- [ ] **P3** Print layouts: single photo, contact sheet, custom package
- [ ] **P3** Print sharpening, printer color management, and 16-bit output
- [ ] **P3** Print to a JPEG file

### 5.3 Other outputs
- [ ] **P3** Slideshow (transitions, music, export to video)
- [ ] **P3** Web gallery generation
- [ ] **P3** Book layout

---

## 6. Application Shell and Platform

- [ ] **P1** Main window with modules (Library, Develop, Editor) and fast module switching
- [ ] **P1** Keyboard shortcuts for all main actions
- [ ] **P1** Preferences (cache location, performance, defaults)
- [ ] **P1** GPU acceleration for display and processing
- [ ] **P2** Customizable panels and workspaces (dock, float, hide, save layouts)
- [ ] **P2** Customizable keyboard shortcuts
- [ ] **P2** Dark and light UI themes
- [ ] **P2** HiDPI / Retina display support
- [ ] **P2** Display color management (use the monitor ICC profile)
- [ ] **P2** HDR display output (view HDR photos and edits on HDR screens)
- [ ] **P2** Background task manager (imports, previews, exports)
- [ ] **P3** Command palette / search for menus and tools
- [ ] **P3** Secondary display window (loupe or grid on a second monitor)
- [ ] **P3** Plug-in / extension system
- [ ] **P3** Localization (multiple UI languages)
- [ ] **P3** Accessibility (screen reader labels, high-contrast mode, full keyboard navigation)
- [ ] **P3** Web version (runs in a browser; same core image engine; local files through browser file access)

---

## Out of Scope (until the operator approves)

- Cloud sync and online storage
- Mobile versions
- Multi-user collaboration
- 3D model editing
- Video editing
