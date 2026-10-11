// Test data uses simple casts for random values, and the sample document
// builder is long but linear.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::too_many_lines
)]

use super::*;

use expose_core::buffer::Rgba;
use expose_core::develop::CurvePoint;

use crate::adjust::{BrightnessContrast, Curves, HueSaturation, Levels, LevelsChannel};
use crate::blend::{BlendMode, BlendSpace};
use crate::document::BitDepth;
use crate::layer::{GroupIsolation, LayerContent, Locks, MaskFill, Placement};

/// A small deterministic random generator (xorshift64*).
struct Rng(u64);

impl Rng {
    fn next_u32(&mut self) -> u32 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 32) as u32
    }

    /// A value in `-2.0..6.0`, so that values below 0 and above 1 occur.
    fn value(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 / (1 << 24) as f32 * 8.0 - 2.0
    }
}

fn random_pixels(rng: &mut Rng, width: u32, height: u32) -> PixelLayer {
    let pixels: Vec<Rgba> = (0..width * height)
        .map(|_| [rng.value(), rng.value(), rng.value(), rng.value()])
        .collect();
    let image = WorkingImage::from_pixels(width, height, pixels).unwrap();
    PixelLayer::new(image, (0, 0)).unwrap()
}

fn random_mask(rng: &mut Rng, width: u32, height: u32, offset: (i32, i32), fill: MaskFill) -> Mask {
    let values = (0..width * height).map(|_| rng.value()).collect();
    Mask::new(
        MaskImage::from_data(width, height, values).unwrap(),
        offset,
        fill,
    )
}

fn add_pixel(doc: &mut Document, rng: &mut Rng, name: &str, at: Placement) -> LayerId {
    let layer = random_pixels(rng, 3, 2);
    doc.add_layer(name, LayerContent::Pixel(layer), at)
        .unwrap()
        .0
}

fn add(doc: &mut Document, name: &str, content: LayerContent, at: Placement) -> LayerId {
    doc.add_layer(name, content, at).unwrap().0
}

/// A document with every layer kind, nested groups, masks, offsets, all
/// blend modes, all adjustment kinds, and locks.
fn sample_document() -> Document {
    let mut rng = Rng(0x1234_5678_9ABC_DEF1);
    let mut spec = DocumentSpec::new(13, 9);
    spec.resolution_ppi = 72.5;
    spec.bit_depth = BitDepth::ThirtyTwo;
    spec.blend_space = BlendSpace::Linear;
    let mut doc = Document::new(spec).unwrap();

    // One pixel layer for each blend mode, with offsets also below 0.
    for (i, mode) in BlendMode::ALL.into_iter().enumerate() {
        let n = doc.layers().len();
        let id = add_pixel(&mut doc, &mut rng, &format!("Mode {i}"), Placement::root(n));
        doc.set_blend_mode(id, mode).unwrap();
        let k = i32::try_from(i).unwrap();
        doc.translate_layer(id, k - 7, 5 - k).unwrap();
    }

    // Isolated group with a pass-through group inside.
    let n = doc.layers().len();
    let outer = add(
        &mut doc,
        "Outer",
        LayerContent::Group(GroupIsolation::Isolated),
        Placement::root(n),
    );
    let masked = add_pixel(&mut doc, &mut rng, "Masked", Placement::new(Some(outer), 0));
    doc.set_mask(
        masked,
        Some(random_mask(&mut rng, 4, 3, (-2, 1), MaskFill::White)),
    )
    .unwrap();
    let inner = add(
        &mut doc,
        "Inner ✓ \"quoted\"",
        LayerContent::Group(GroupIsolation::PassThrough),
        Placement::new(Some(outer), 1),
    );
    let channel = LevelsChannel {
        input_black: 0.1,
        input_white: 0.9,
        gamma: 1.7,
        output_black: 0.05,
        output_white: 0.95,
    };
    let adjustments = [
        Adjustment::BrightnessContrast(BrightnessContrast {
            brightness: 0.3,
            contrast: -0.45,
        }),
        Adjustment::Levels(Levels {
            master: channel,
            red: LevelsChannel {
                gamma: 0.3,
                ..channel
            },
            green: LevelsChannel::default(),
            blue: LevelsChannel {
                input_black: 0.0,
                ..channel
            },
        }),
        Adjustment::Curves(Curves {
            master: vec![
                CurvePoint::new(0.0, 0.1),
                CurvePoint::new(0.3, 0.2),
                CurvePoint::new(1.0, 0.9),
            ],
            red: vec![CurvePoint::new(0.0, 0.0), CurvePoint::new(1.0, 1.0)],
            green: vec![
                CurvePoint::new(0.1, 0.0),
                CurvePoint::new(0.5, 0.6),
                CurvePoint::new(0.9, 1.0),
            ],
            blue: vec![CurvePoint::new(0.0, 1.0), CurvePoint::new(1.0, 0.0)],
        }),
        Adjustment::HueSaturation(HueSaturation {
            hue: -123.25,
            saturation: 0.5,
            lightness: -0.125,
        }),
    ];
    for (i, a) in adjustments.into_iter().enumerate() {
        let id = add(
            &mut doc,
            &format!("Adjustment {i}"),
            LayerContent::Adjustment(a),
            Placement::new(Some(inner), i),
        );
        doc.set_opacity(id, 0.25 * (i as f32 + 1.0)).unwrap();
    }
    let in_inner = add_pixel(&mut doc, &mut rng, "Deep", Placement::new(Some(inner), 0));
    let mut mask = random_mask(&mut rng, 2, 5, (3, -4), MaskFill::Black);
    mask.enabled = false;
    doc.set_mask(in_inner, Some(mask)).unwrap();
    doc.set_mask(
        outer,
        Some(random_mask(&mut rng, 13, 9, (0, 0), MaskFill::Black)),
    )
    .unwrap();
    let empty_group = add(
        &mut doc,
        "",
        LayerContent::Group(GroupIsolation::PassThrough),
        Placement::new(Some(inner), 0),
    );

    // Properties and locks (locks last, because they block changes).
    doc.set_opacity(masked, 0.37).unwrap();
    doc.set_fill_opacity(masked, 0.81).unwrap();
    doc.set_visible(in_inner, false).unwrap();
    doc.set_blend_mode(outer, BlendMode::Screen).unwrap();
    doc.set_locks(
        masked,
        Locks {
            transparency: true,
            position: true,
            ..Locks::NONE
        },
    )
    .unwrap();
    doc.set_locks(
        inner,
        Locks {
            pixels: true,
            all: true,
            ..Locks::NONE
        },
    )
    .unwrap();
    doc.set_locks(
        empty_group,
        Locks {
            all: true,
            ..Locks::NONE
        },
    )
    .unwrap();

    // A deleted layer makes the next ID larger than the largest ID + 1.
    let n = doc.layers().len();
    let gone = add_pixel(&mut doc, &mut rng, "Gone", Placement::root(n));
    doc.delete_layer(gone).unwrap();
    doc
}

fn to_bytes(doc: &Document) -> Vec<u8> {
    let mut out = Vec::new();
    write_document(doc, &mut out).unwrap();
    out
}

fn manifest_len(bytes: &[u8]) -> usize {
    let mut b = [0_u8; 8];
    b.copy_from_slice(&bytes[12..20]);
    usize::try_from(u64::from_le_bytes(b)).unwrap()
}

/// Makes a file from a manifest JSON text and blob bytes.
fn file_from_json(json: &str, blobs: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&VERSION_MAJOR.to_le_bytes());
    out.extend_from_slice(&VERSION_MINOR.to_le_bytes());
    out.extend_from_slice(&(json.len() as u64).to_le_bytes());
    out.extend_from_slice(json.as_bytes());
    out.extend_from_slice(blobs);
    out
}

#[test]
fn round_trip_gives_an_equal_document() {
    let doc = sample_document();
    let bytes = to_bytes(&doc);
    let back = read_document(bytes.as_slice()).unwrap();
    assert_eq!(back, doc);
    assert_eq!(back.next_id(), doc.next_id());
    assert_eq!(back.layer_count(), doc.layer_count());
    // A write of the loaded document gives the same bytes.
    assert_eq!(to_bytes(&back), bytes);
}

#[test]
fn loaded_document_gets_new_stamps() {
    let doc = sample_document();
    let back = read_document(to_bytes(&doc).as_slice()).unwrap();
    assert_ne!(back.revision(), doc.revision());
    let a = doc.layers();
    let b = back.layers();
    for (x, y) in a.iter().zip(b) {
        assert_eq!(x.id(), y.id());
        assert_ne!(x.pixel_version(), y.pixel_version());
        assert_ne!(x.mask_version(), y.mask_version());
    }
}

#[test]
fn round_trip_keeps_all_bits() {
    // Special values: negative zero, subnormals, infinities, NaN payloads.
    let specials = [
        -0.0_f32,
        f32::MIN_POSITIVE / 3.0,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::from_bits(0x7FC0_1234),
        f32::MAX,
        -1.0e-30,
        1.0e30,
    ];
    let pixels: Vec<Rgba> = specials
        .chunks_exact(4)
        .map(|c| [c[0], c[1], c[2], c[3]])
        .collect();
    let image = WorkingImage::from_pixels(2, 1, pixels).unwrap();
    let layer = PixelLayer::new(image, (i32::MIN, i32::MAX)).unwrap();
    let doc = Document::from_layer(DocumentSpec::new(1, 1), "Special", layer).unwrap();
    let back = read_document(to_bytes(&doc).as_slice()).unwrap();
    let got: Vec<u32> = back.layers()[0]
        .as_pixel()
        .unwrap()
        .image()
        .as_slice()
        .iter()
        .map(|v| v.to_bits())
        .collect();
    let want: Vec<u32> = specials.iter().map(|v| v.to_bits()).collect();
    assert_eq!(got, want);
    assert_eq!(
        back.layers()[0].as_pixel().unwrap().offset(),
        (i32::MIN, i32::MAX)
    );
}

#[test]
fn round_trip_of_float_properties_is_exact() {
    let mut rng = Rng(99);
    let mut doc = Document::new_empty(DocumentSpec::new(2, 2)).unwrap();
    for i in 0..200 {
        let id = add(
            &mut doc,
            "a",
            LayerContent::Adjustment(Adjustment::HueSaturation(HueSaturation {
                hue: (rng.value() - 2.0) * 45.0,
                saturation: rng.value() / 8.0,
                lightness: -rng.value() / 8.0 + 0.5,
            })),
            Placement::root(i),
        );
        let o = (rng.next_u32() as f32 / u32::MAX as f32).clamp(0.0, 1.0);
        doc.set_opacity(id, o).unwrap();
    }
    let back = read_document(to_bytes(&doc).as_slice()).unwrap();
    assert_eq!(back, doc);
}

#[test]
fn empty_document_round_trip() {
    let doc = Document::new_empty(DocumentSpec::new(30_000, 1)).unwrap();
    let back = read_document(to_bytes(&doc).as_slice()).unwrap();
    assert_eq!(back, doc);
}

#[test]
fn truncated_file_is_an_error() {
    let bytes = to_bytes(&sample_document());
    for len in 0..bytes.len() {
        let result = read_document(&bytes[..len]);
        assert!(result.is_err(), "prefix of {len} bytes was accepted");
    }
}

#[test]
fn truncated_blob_section_is_a_truncated_error() {
    let bytes = to_bytes(&sample_document());
    let err = read_document(&bytes[..bytes.len() - 1]).unwrap_err();
    assert_eq!(err, EditorError::File(FileError::Truncated));
}

#[test]
fn flipped_blob_byte_is_a_crc_error() {
    let bytes = to_bytes(&sample_document());
    let start = HEADER_LEN + manifest_len(&bytes);
    for at in [
        start,
        start + 1,
        usize::midpoint(start, bytes.len()),
        bytes.len() - 1,
    ] {
        let mut bad = bytes.clone();
        bad[at] ^= 0x10;
        let err = read_document(bad.as_slice()).unwrap_err();
        assert!(
            matches!(err, EditorError::File(FileError::Crc { .. })),
            "byte {at}: {err:?}"
        );
    }
}

#[test]
fn wrong_magic_is_rejected() {
    let mut bytes = to_bytes(&sample_document());
    bytes[1] = b'Y';
    assert_eq!(
        read_document(bytes.as_slice()).unwrap_err(),
        EditorError::File(FileError::BadMagic)
    );
    assert_eq!(
        read_document(&b"not a document at all"[..]).unwrap_err(),
        EditorError::File(FileError::BadMagic)
    );
}

#[test]
fn future_major_version_is_rejected_and_minor_is_accepted() {
    let doc = sample_document();
    let mut bytes = to_bytes(&doc);
    bytes[8..10].copy_from_slice(&2_u16.to_le_bytes());
    bytes[10..12].copy_from_slice(&7_u16.to_le_bytes());
    assert_eq!(
        read_document(bytes.as_slice()).unwrap_err(),
        EditorError::File(FileError::UnsupportedVersion { major: 2, minor: 7 })
    );
    bytes[8..10].copy_from_slice(&VERSION_MAJOR.to_le_bytes());
    assert_eq!(read_document(bytes.as_slice()).unwrap(), doc);
}

#[test]
fn manifest_length_limit() {
    let mut bytes = to_bytes(&sample_document());
    bytes[12..20].copy_from_slice(&(MAX_MANIFEST_BYTES + 1).to_le_bytes());
    assert_eq!(
        read_document(bytes.as_slice()).unwrap_err(),
        EditorError::File(FileError::ManifestTooLarge(MAX_MANIFEST_BYTES + 1))
    );
}

const DOC_OK: &str = r#""document":{"width":4,"height":4,"resolutionPpi":300,"colorMode":"rgb","bitDepth":16,"blendSpace":"perceptual"}"#;

fn layer_json(id: u64, content: &str) -> String {
    format!(
        r#"{{"id":{id},"name":"L","visible":true,"opacity":1,"fillOpacity":1,"blendMode":"normal","locks":{{}},"content":{content}}}"#
    )
}

#[test]
fn unknown_fields_are_ignored_and_next_id_is_above_the_max() {
    let layer = layer_json(
        41,
        r#"{"kind":"group","isolation":"isolated","children":[],"future":[1,2]}"#,
    );
    let json = format!(r#"{{{DOC_OK},"layers":[{layer}],"blobs":[],"extra":{{"a":1}}}}"#);
    let doc = read_document(file_from_json(&json, &[]).as_slice()).unwrap();
    assert_eq!(doc.layer_by_number(41).map(LayerId::get), Some(41));
    assert_eq!(doc.next_id(), 42);
}

#[test]
fn oversize_dimensions_are_rejected_before_allocation() {
    // Canvas too large.
    let json = r#"{"document":{"width":30001,"height":4,"resolutionPpi":300,"colorMode":"rgb","bitDepth":16,"blendSpace":"linear"},"layers":[],"blobs":[]}"#;
    assert!(matches!(
        read_document(file_from_json(json, &[]).as_slice()),
        Err(EditorError::TooLarge { .. })
    ));

    // Layer with too many pixels (no blob data follows).
    let blob = r#"{"offset":0,"compressedLength":10,"rawLength":6400000000,"crc32":0}"#;
    let layer = layer_json(
        1,
        r#"{"kind":"pixel","offset":[0,0],"width":20000,"height":20000,"blob":0}"#,
    );
    let json = format!(r#"{{{DOC_OK},"layers":[{layer}],"blobs":[{blob}]}}"#);
    assert!(matches!(
        read_document(file_from_json(&json, &[]).as_slice()),
        Err(EditorError::TooLarge { .. })
    ));

    // The largest permitted layer (4 GiB), but the blob is too short for
    // its raw length.
    let blob = r#"{"offset":0,"compressedLength":1000,"rawLength":4294967296,"crc32":0}"#;
    let layer = layer_json(
        1,
        r#"{"kind":"pixel","offset":[0,0],"width":16384,"height":16384,"blob":0}"#,
    );
    let json = format!(r#"{{{DOC_OK},"layers":[{layer}],"blobs":[{blob}]}}"#);
    assert!(matches!(
        read_document(file_from_json(&json, &[]).as_slice()),
        Err(EditorError::File(FileError::Blob { blob: 0, .. }))
    ));

    // Raw length does not agree with the layer size.
    let blob = r#"{"offset":0,"compressedLength":10,"rawLength":96,"crc32":0}"#;
    let mask =
        r#""mask":{"offset":[0,0],"width":5,"height":5,"enabled":true,"fill":"white","blob":0},"#;
    let layer = layer_json(
        1,
        r#"{"kind":"group","isolation":"isolated","children":[]}"#,
    )
    .replacen("\"content\"", &format!("{mask}\"content\""), 1);
    let json = format!(r#"{{{DOC_OK},"layers":[{layer}],"blobs":[{blob}]}}"#);
    assert!(matches!(
        read_document(file_from_json(&json, &[]).as_slice()),
        Err(EditorError::File(FileError::Blob { blob: 0, .. }))
    ));
}

#[test]
fn structural_errors_are_rejected() {
    let group = r#"{"kind":"group","isolation":"isolated","children":[]}"#;
    let blob = r#"{"offset":0,"compressedLength":10,"rawLength":256,"crc32":0}"#;
    let pixel = r#"{"kind":"pixel","offset":[0,0],"width":4,"height":4,"blob":0}"#;
    let cases: Vec<(String, &str)> = vec![
        (
            format!("[{},{}]", layer_json(3, group), layer_json(3, group)),
            "[]",
        ),
        (format!("[{}]", layer_json(0, group)), "[]"),
        (format!("[{}]", layer_json(1, pixel)), "[]"),
        (
            format!("[{},{}]", layer_json(1, pixel), layer_json(2, pixel)),
            "BLOB",
        ),
        (
            format!(
                "[{}]",
                layer_json(
                    1,
                    r#"{"kind":"adjustment","adjustment":{"type":"hueSaturation","hue":400,"saturation":0,"lightness":0}}"#
                )
            ),
            "[]",
        ),
        (
            format!(
                "[{}]",
                layer_json(
                    1,
                    r#"{"kind":"adjustment","adjustment":{"type":"curves","master":[[0,0]],"red":[[0,0],[1,1]],"green":[[0,0],[1,1]],"blue":[[0,0],[1,1]]}}"#
                )
            ),
            "[]",
        ),
        (
            format!(
                "[{}]",
                layer_json(1, group).replace("\"opacity\":1", "\"opacity\":1.5")
            ),
            "[]",
        ),
        (
            format!("[{}]", layer_json(1, group).replace("normal", "sparkle")),
            "[]",
        ),
    ];
    let blob_list = format!("[{blob}]");
    for (layers, blobs) in cases {
        let blobs = if blobs == "BLOB" {
            blob_list.as_str()
        } else {
            blobs
        };
        let json = format!(r#"{{{DOC_OK},"layers":{layers},"blobs":{blobs}}}"#);
        let result = read_document(file_from_json(&json, &[0; 64]).as_slice());
        assert!(result.is_err(), "accepted: {json}");
    }

    // Overlapping blobs.
    let doc = Document::new(DocumentSpec::new(2, 2)).unwrap();
    let mut doc2 = doc.clone();
    let id = doc2.layers()[0].id();
    doc2.set_mask(
        id,
        Some(Mask::new(
            MaskImage::new_filled(2, 2, 0.5).unwrap(),
            (0, 0),
            MaskFill::White,
        )),
    )
    .unwrap();
    let bytes = to_bytes(&doc2);
    let len = manifest_len(&bytes);
    let json = std::str::from_utf8(&bytes[HEADER_LEN..HEADER_LEN + len]).unwrap();
    let mut manifest: serde_json::Value = serde_json::from_str(json).unwrap();
    manifest["blobs"][1]["offset"] = serde_json::Value::from(1);
    let json = manifest.to_string();
    let bad = file_from_json(&json, &bytes[HEADER_LEN + len..]);
    assert_eq!(
        read_document(bad.as_slice()).unwrap_err(),
        EditorError::File(FileError::Invalid("blobs overlap"))
    );
}

#[test]
fn blob_gaps_and_unused_blobs_are_skipped() {
    let mut doc = Document::new(DocumentSpec::new(3, 2)).unwrap();
    let id = doc.layers()[0].id();
    doc.set_mask(
        id,
        Some(Mask::new(
            MaskImage::new_filled(3, 2, 0.25).unwrap(),
            (1, 1),
            MaskFill::Black,
        )),
    )
    .unwrap();
    let bytes = to_bytes(&doc);
    let len = manifest_len(&bytes);
    let json = std::str::from_utf8(&bytes[HEADER_LEN..HEADER_LEN + len]).unwrap();
    let mut manifest: serde_json::Value = serde_json::from_str(json).unwrap();
    // Put 5 junk bytes before each blob, and an unused blob at the end.
    let section = &bytes[HEADER_LEN + len..];
    let first =
        usize::try_from(manifest["blobs"][0]["compressedLength"].as_u64().unwrap()).unwrap();
    let mut blobs = Vec::new();
    blobs.extend_from_slice(&[7; 5]);
    blobs.extend_from_slice(&section[..first]);
    blobs.extend_from_slice(&[7; 5]);
    blobs.extend_from_slice(&section[first..]);
    manifest["blobs"][0]["offset"] = serde_json::Value::from(5);
    manifest["blobs"][1]["offset"] = serde_json::Value::from(first as u64 + 10);
    let unused = manifest["blobs"][0].clone();
    manifest["blobs"].as_array_mut().unwrap().push(unused);
    let file = file_from_json(&manifest.to_string(), &blobs);
    assert_eq!(read_document(file.as_slice()).unwrap(), doc);
}

#[test]
fn too_deep_groups_are_rejected_by_the_writer() {
    let mut doc = Document::new_empty(DocumentSpec::new(2, 2)).unwrap();
    let mut parent = None;
    for _ in 0..=MAX_GROUP_DEPTH {
        let id = add(
            &mut doc,
            "g",
            LayerContent::Group(GroupIsolation::PassThrough),
            Placement::new(parent, 0),
        );
        parent = Some(id);
    }
    // MAX_GROUP_DEPTH + 1 nested groups: the deepest is at depth
    // MAX_GROUP_DEPTH and has no children, so this is permitted.
    let back = read_document(to_bytes(&doc).as_slice()).unwrap();
    assert_eq!(back, doc);
    // One more level is not permitted.
    add(
        &mut doc,
        "a",
        LayerContent::Adjustment(Adjustment::Curves(Curves::default())),
        Placement::new(parent, 0),
    );
    let mut out = Vec::new();
    assert!(matches!(
        write_document(&doc, &mut out),
        Err(EditorError::File(FileError::Invalid(_)))
    ));
}

#[test]
fn max_depth_levels_adjustment_fits_the_parser_limit() {
    let mut doc = Document::new_empty(DocumentSpec::new(2, 2)).unwrap();
    let mut parent = None;
    for _ in 0..MAX_GROUP_DEPTH {
        let id = add(
            &mut doc,
            "g",
            LayerContent::Group(GroupIsolation::PassThrough),
            Placement::new(parent, 0),
        );
        parent = Some(id);
    }
    add(
        &mut doc,
        "c",
        LayerContent::Adjustment(Adjustment::Curves(Curves::default())),
        Placement::new(parent, 0),
    );
    let deep = add_pixel(&mut doc, &mut Rng(5), "p", Placement::new(parent, 0));
    doc.set_mask(
        deep,
        Some(Mask::new(
            MaskImage::new_filled(1, 1, 1.0).unwrap(),
            (0, 0),
            MaskFill::White,
        )),
    )
    .unwrap();
    let back = read_document(to_bytes(&doc).as_slice()).unwrap();
    assert_eq!(back, doc);
}

/// Write and read time of a 6000 x 4000 layer. Run with
/// `cargo test -p expose-editor --release -- --ignored --nocapture file_benchmark`.
#[test]
#[ignore = "benchmark"]
fn file_benchmark() {
    use std::time::Instant;
    let (w, h) = (6000_u32, 4000_u32);
    let mut rng = Rng(7);
    let noise: Vec<Rgba> = (0..w * h)
        .map(|_| {
            let v = (rng.next_u32() >> 8) as f32 / (1 << 24) as f32;
            [v, v * 0.5, v * 0.25, 1.0]
        })
        .collect();
    let smooth: Vec<Rgba> = (0..h)
        .flat_map(|y| {
            (0..w).map(move |x| {
                let r = x as f32 / w as f32;
                let g = y as f32 / h as f32;
                [r, g, (r + g) * 0.5, 1.0]
            })
        })
        .collect();
    for (name, pixels) in [("noise", noise), ("smooth", smooth)] {
        let image = WorkingImage::from_pixels(w, h, pixels).unwrap();
        let layer = PixelLayer::new(image, (0, 0)).unwrap();
        let doc = Document::from_layer(DocumentSpec::new(w, h), "Photo", layer).unwrap();
        let t = Instant::now();
        let bytes = to_bytes(&doc);
        let write = t.elapsed();
        let t = Instant::now();
        let back = read_document(bytes.as_slice()).unwrap();
        let read = t.elapsed();
        assert_eq!(back, doc);
        let raw = u64::from(w) * u64::from(h) * 16;
        eprintln!(
            "{name}: {} bytes ({:.1} % of {raw}), write {write:?}, read {read:?}",
            bytes.len(),
            bytes.len() as f64 * 100.0 / raw as f64
        );
    }
}
