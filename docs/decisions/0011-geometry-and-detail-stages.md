# 0011 — Geometry and detail stages

- **Status:** Accepted
- **Date:** 2026-10-04
- **Decided by:** Claude (implementation details; the operator approved the start of the work)

## Context

The develop pipeline had only per-pixel stages (ADR 0005, 0009). Crop, straighten, flip, and sharpening settings existed in `DevelopSettings`, but no stage applied them. Export and thumbnails ignored them. The work is split into tasks: CPU geometry (PR #24), CPU sharpening (PR #25), GPU kernels with several inputs and any output size (PR #26), the pipeline connection with GPU kernels, and the shell connection.

## Decisions

### Stage order

Full develop: **geometry → white balance and exposure → tone zones and contrast → tone curve → sharpening → output encode**.

- Geometry is first. It works on linear source data, and a crop makes the image smaller before the other stages.
- Sharpening is last. It works on the toned image, so the halo strength does not change when the tone sliders change.

### Geometry (`expose-develop/src/geometry.rs`)

- Order: flip, then rotate by `angle` (degrees, clockwise on the screen) about the image center, then crop.
- The crop frame is a `W x H` frame with the same center as the image. `Crop::rect` is normalized to this frame. The output size is `round(rect.width * W) x round(rect.height * H)`, at least 1 x 1.
- The per-pixel mapping is one affine transform (output pixel center to source position). It is computed in `f64` and stored as `f32`, so the GPU kernel uses the same numbers.
- Angle 0: integer copy (crop and flips are bit-exact). Other angles: bicubic Catmull-Rom (`a = -0.5`) with clamp-to-edge taps.
- A sample point outside the source gives transparent black. The pipeline does not change the rect. The crop tool must keep the rect inside the rotated image with `max_inscribed_rect` and `rect_is_inside`.

### Sharpening (`expose-develop/src/sharpen.rs`)

- An unsharp mask on luminance only, so it adds no color fringes. RGB is scaled by `Y' / Y`.
- The detail is found in the sRGB-encoded luminance, an approximately perceptual scale.
- Four passes, one GPU kernel each: encode, horizontal blur, vertical blur, combine.
- The blur sigma is `radius * pixel_scale`. `pixel_scale` is the number of output pixels for each full-resolution source pixel, so a downscaled preview looks like the full-size result. A sigma below 0.25 makes the stage a no-op. The sigma is limited to 5 (16 weights per side).
- `detail` limits large differences softly (halo suppression). `masking` makes an edge mask from the gradient of the blurred luminance. The constants are public in the module.

### GPU kernels with several inputs (`expose-gpu`)

- `run_kernel_multi`: 1 to 4 inputs at bindings 0, 3, 4, 5. Bindings 0, 1, 2 keep their meaning, so old kernels work unchanged. The output can have any size.
- The 12 layouts (3 output formats x 4 input counts) are made when the context is made. The pipeline cache key is (kernel name, output format, input count).
- Each call is one queue submit. The queue keeps the order, so a pass can read the output of the pass before it.

### Pipeline API (PR #29)

- `Pipeline::new` does not know the image size. Size-dependent parts are built per call: `geometry(w, h)`, `sharpener(pixel_scale)`, `output_size(w, h)`.
- `develop_cpu` / `develop_gpu` run all stages. The old `apply_cpu`, `render_cpu`, and `render_gpu` run the tone stages only.
- `is_identity()` is true only when no stage of any kind is active. `tone_is_identity()` is the per-pixel part.
- `set_apply_geometry(false)` turns geometry off, so the crop tool can show the full image while the user drags.
- GPU intermediates are `rgba32float` images (only the red channel is used for the luminance passes, because `expose-gpu` has no single-channel format). `DevelopTargets` makes them again only when a size changes.
- Export sharpens at source resolution, before the resize (the radius is in source pixels).

### Shell connection (PR #31)

- `preview_render` has an optional `applyGeometry` (default `true`). The frame header carries the real output size, which is smaller than the session size when a crop is set. `longEdge` still sets the size of the pipeline input.
- Each preview session keeps the full-resolution size, so the sharpening pixel scale is correct. The display encoder is made again only when the frame size changes.
- The thumbnail format version is 2. Old cached thumbnails of cropped or sharpened photos had the same settings key, so they must be made again.
- Export develops at source resolution, then resizes. The output size and the 200 MP limit use the cropped size.

## Consequences

- The preview frame size changes when the crop changes. The UI presenter fits each frame again, so it accepts frames of a new size.
- PR #38 keeps cropped previews sharp: `longEdge` is the frame size, and the engine enlarges the develop input by the crop factor up to a 4096 px session source (ADR 0008 update). Thumbnails of cropped photos develop on up to 4096 px (thumbnail format version 3). Crops tighter than about 60 % of the long edge still show slightly enlarged.
- The crop UI (PRs #34, #39) shows the full image with one CSS matrix (flip, rotate, zoom-out to fit) and keeps the rect inside the rotated image with TS ports of `max_inscribed_rect` and `rect_is_inside`.
- Each GPU pass still makes a new parameter buffer and bind group. Buffer reuse is in the `gpu-api` backlog.
