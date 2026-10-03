//! Baseline JPEG encoder (ITU-T T.81, JFIF).
//!
//! # Why a local encoder
//!
//! The JPEG encoder of the `image` crate always writes 4:4:4 (no chroma
//! subsampling option). Other pure-Rust encoders are not under the
//! permitted licences only. This encoder is small and uses only the public
//! JPEG standard: the example quantization and Huffman tables of Annex K.
//!
//! # Features
//!
//! - Baseline sequential DCT, 8-bit, 3 components (YCbCr, JFIF full range).
//! - Chroma subsampling 4:4:4, 4:2:2, or 4:2:0 (box filter for chroma).
//! - Quality 1..=100 with the usual linear scaling of the Annex K tables.
//! - An ICC profile in APP2 `ICC_PROFILE` segments (split if necessary).
//! - Edges: the image is extended by edge pixel repetition to a whole
//!   number of MCUs.
//!
//! The encoder uses the standard Huffman tables (not optimized tables).
//! Optimized tables would make files about 5 to 10 % smaller.

use expose_core::buffer::Image8;

use crate::encode::ChromaSubsampling;
use crate::error::{EncodeError, EncodeResult};

/// The largest width or height of a baseline JPEG.
pub(crate) const MAX_SIDE: u32 = 65_535;

/// Largest ICC data in one APP2 segment: 65 535 - 2 (length) - 14 (header).
const ICC_CHUNK: usize = 65_519;

/// Natural (row-major) index of each coefficient in zigzag order.
const ZIGZAG: [usize; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20,
    13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59,
    52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
];

/// Annex K.1 luminance quantization table (natural order).
const LUMA_Q: [u16; 64] = [
    16, 11, 10, 16, 24, 40, 51, 61, 12, 12, 14, 19, 26, 58, 60, 55, 14, 13, 16, 24, 40, 57, 69, 56,
    14, 17, 22, 29, 51, 87, 80, 62, 18, 22, 37, 56, 68, 109, 103, 77, 24, 35, 55, 64, 81, 104, 113,
    92, 49, 64, 78, 87, 103, 121, 120, 101, 72, 92, 95, 98, 112, 100, 103, 99,
];

/// Annex K.1 chrominance quantization table (natural order).
const CHROMA_Q: [u16; 64] = [
    17, 18, 24, 47, 99, 99, 99, 99, 18, 21, 26, 66, 99, 99, 99, 99, 24, 26, 56, 99, 99, 99, 99, 99,
    47, 66, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99,
    99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99,
];

/// Annex K.3 Huffman table specifications: code counts for lengths 1..=16,
/// then the symbol values.
const DC_LUMA_BITS: [u8; 16] = [0, 1, 5, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0];
const DC_CHROMA_BITS: [u8; 16] = [0, 3, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0];
const DC_VALUES: [u8; 12] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11];

const AC_LUMA_BITS: [u8; 16] = [0, 2, 1, 3, 3, 2, 4, 3, 5, 5, 4, 4, 0, 0, 1, 0x7d];
const AC_LUMA_VALUES: [u8; 162] = [
    0x01, 0x02, 0x03, 0x00, 0x04, 0x11, 0x05, 0x12, 0x21, 0x31, 0x41, 0x06, 0x13, 0x51, 0x61, 0x07,
    0x22, 0x71, 0x14, 0x32, 0x81, 0x91, 0xa1, 0x08, 0x23, 0x42, 0xb1, 0xc1, 0x15, 0x52, 0xd1, 0xf0,
    0x24, 0x33, 0x62, 0x72, 0x82, 0x09, 0x0a, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x25, 0x26, 0x27, 0x28,
    0x29, 0x2a, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3a, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49,
    0x4a, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5a, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69,
    0x6a, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7a, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89,
    0x8a, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9a, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa7,
    0xa8, 0xa9, 0xaa, 0xb2, 0xb3, 0xb4, 0xb5, 0xb6, 0xb7, 0xb8, 0xb9, 0xba, 0xc2, 0xc3, 0xc4, 0xc5,
    0xc6, 0xc7, 0xc8, 0xc9, 0xca, 0xd2, 0xd3, 0xd4, 0xd5, 0xd6, 0xd7, 0xd8, 0xd9, 0xda, 0xe1, 0xe2,
    0xe3, 0xe4, 0xe5, 0xe6, 0xe7, 0xe8, 0xe9, 0xea, 0xf1, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8,
    0xf9, 0xfa,
];

const AC_CHROMA_BITS: [u8; 16] = [0, 2, 1, 2, 4, 4, 3, 4, 7, 5, 4, 4, 0, 1, 2, 0x77];
const AC_CHROMA_VALUES: [u8; 162] = [
    0x00, 0x01, 0x02, 0x03, 0x11, 0x04, 0x05, 0x21, 0x31, 0x06, 0x12, 0x41, 0x51, 0x07, 0x61, 0x71,
    0x13, 0x22, 0x32, 0x81, 0x08, 0x14, 0x42, 0x91, 0xa1, 0xb1, 0xc1, 0x09, 0x23, 0x33, 0x52, 0xf0,
    0x15, 0x62, 0x72, 0xd1, 0x0a, 0x16, 0x24, 0x34, 0xe1, 0x25, 0xf1, 0x17, 0x18, 0x19, 0x1a, 0x26,
    0x27, 0x28, 0x29, 0x2a, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3a, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48,
    0x49, 0x4a, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5a, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68,
    0x69, 0x6a, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7a, 0x82, 0x83, 0x84, 0x85, 0x86, 0x87,
    0x88, 0x89, 0x8a, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9a, 0xa2, 0xa3, 0xa4, 0xa5,
    0xa6, 0xa7, 0xa8, 0xa9, 0xaa, 0xb2, 0xb3, 0xb4, 0xb5, 0xb6, 0xb7, 0xb8, 0xb9, 0xba, 0xc2, 0xc3,
    0xc4, 0xc5, 0xc6, 0xc7, 0xc8, 0xc9, 0xca, 0xd2, 0xd3, 0xd4, 0xd5, 0xd6, 0xd7, 0xd8, 0xd9, 0xda,
    0xe2, 0xe3, 0xe4, 0xe5, 0xe6, 0xe7, 0xe8, 0xe9, 0xea, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8,
    0xf9, 0xfa,
];

/// A Huffman code table: `(code, length)` for each symbol value.
struct HuffTable {
    codes: [(u16, u8); 256],
}

impl HuffTable {
    /// Generates the codes from a table specification (T.81 Annex C).
    fn new(bits: &[u8; 16], values: &[u8]) -> Self {
        let mut codes = [(0_u16, 0_u8); 256];
        let mut code: u16 = 0;
        let mut k = 0;
        for (i, &n) in bits.iter().enumerate() {
            // The length is i + 1, at most 16.
            #[allow(clippy::cast_possible_truncation, reason = "i is in 0..16")]
            let len = (i + 1) as u8;
            for _ in 0..n {
                if let Some(&v) = values.get(k) {
                    codes[usize::from(v)] = (code, len);
                }
                code = code.wrapping_add(1);
                k += 1;
            }
            code = code.wrapping_shl(1);
        }
        Self { codes }
    }
}

/// Bit writer with JPEG byte stuffing (0xFF is followed by 0x00).
struct BitWriter<'a> {
    out: &'a mut Vec<u8>,
    acc: u32,
    bits: u32,
}

impl<'a> BitWriter<'a> {
    fn new(out: &'a mut Vec<u8>) -> Self {
        Self {
            out,
            acc: 0,
            bits: 0,
        }
    }

    /// Writes the low `len` bits of `code` (`len` <= 16).
    #[inline]
    fn put(&mut self, code: u32, len: u32) {
        // bits < 8 before the call and len <= 16, so acc keeps at most 23
        // significant bits.
        self.acc = (self.acc << len) | (code & ((1 << len) - 1));
        self.bits += len;
        while self.bits >= 8 {
            self.bits -= 8;
            #[allow(clippy::cast_possible_truncation, reason = "masked to one byte")]
            let byte = ((self.acc >> self.bits) & 0xFF) as u8;
            self.out.push(byte);
            if byte == 0xFF {
                self.out.push(0);
            }
        }
        self.acc &= (1 << self.bits) - 1;
    }

    /// Pads the last byte with 1 bits.
    fn flush(&mut self) {
        if self.bits > 0 {
            let pad = 8 - self.bits;
            self.put((1 << pad) - 1, pad);
        }
    }
}

/// Returns the bit count (category) of `v` and its JPEG bit pattern.
#[inline]
fn category(v: i32) -> (u32, u32) {
    let mag = v.unsigned_abs();
    let n = 32 - mag.leading_zeros();
    // Negative values are stored as v - 1 in n bits (one's complement).
    #[allow(clippy::cast_sign_loss, reason = "the bit pattern is wanted")]
    let bits = if v < 0 { (v - 1) as u32 } else { v as u32 };
    (n, bits & ((1 << n) - 1))
}

/// Scales a base quantization table for `quality` (1..=100).
fn scaled_table(base: &[u16; 64], quality: u8) -> [u16; 64] {
    let q = u32::from(quality.clamp(1, 100));
    let scale = if q < 50 { 5000 / q } else { 200 - 2 * q };
    base.map(|b| {
        let v = (u32::from(b) * scale + 50) / 100;
        // Baseline JPEG with 8-bit tables: 1..=255.
        #[allow(clippy::cast_possible_truncation, reason = "clamped to 1..=255")]
        let v = v.clamp(1, 255) as u16;
        v
    })
}

/// The 8x8 forward DCT basis: `C[u][x] = c(u) / 2 * cos((2x + 1) u pi / 16)`.
fn dct_matrix() -> [[f32; 8]; 8] {
    let mut m = [[0.0_f32; 8]; 8];
    for (u, row) in m.iter_mut().enumerate() {
        let cu = if u == 0 {
            std::f64::consts::FRAC_1_SQRT_2
        } else {
            1.0
        };
        for (x, v) in row.iter_mut().enumerate() {
            #[allow(clippy::cast_precision_loss, reason = "small integers")]
            let angle = ((2 * x + 1) * u) as f64 * std::f64::consts::PI / 16.0;
            #[allow(clippy::cast_possible_truncation, reason = "f32 basis is sufficient")]
            {
                *v = (cu / 2.0 * angle.cos()) as f32;
            }
        }
    }
    m
}

/// Per-component state of the encoder.
struct Component<'t> {
    /// Reciprocal of the quantization table (natural order).
    inv_q: [f32; 64],
    dc: &'t HuffTable,
    ac: &'t HuffTable,
    pred: i32,
}

struct BlockCoder {
    dct: [[f32; 8]; 8],
}

impl BlockCoder {
    /// Transforms, quantizes, and entropy-codes one block of level-shifted
    /// samples (row-major).
    fn encode(&self, block: &[f32; 64], comp: &mut Component<'_>, w: &mut BitWriter<'_>) {
        // Rows: t[y][u] = sum_x C[u][x] * b[y][x].
        let mut t = [0.0_f32; 64];
        for y in 0..8 {
            let row = &block[y * 8..y * 8 + 8];
            for u in 0..8 {
                let c = &self.dct[u];
                let mut s = 0.0;
                for x in 0..8 {
                    s += c[x] * row[x];
                }
                t[y * 8 + u] = s;
            }
        }
        // Columns: F[v][u] = sum_y C[v][y] * t[y][u], then quantize.
        let mut q = [0_i32; 64];
        for v in 0..8 {
            let c = &self.dct[v];
            for u in 0..8 {
                let mut s = 0.0;
                for y in 0..8 {
                    s += c[y] * t[y * 8 + u];
                }
                let i = v * 8 + u;
                // Clamp to the ranges that baseline Huffman categories
                // can code (DC: 11 bits for the difference, AC: 10 bits).
                // With 8-bit input the values stay in range; the clamp
                // protects against float rounding at the extremes.
                #[allow(clippy::cast_possible_truncation, reason = "clamped to +-1024")]
                let qv = (s * comp.inv_q[i]).round().clamp(-1024.0, 1023.0) as i32;
                q[i] = qv;
            }
        }

        // DC difference.
        let dc = q[0];
        let diff = dc - comp.pred;
        comp.pred = dc;
        let (n, bits) = category(diff);
        let (code, len) = comp.dc.codes[n as usize];
        w.put(u32::from(code), u32::from(len));
        if n > 0 {
            w.put(bits, n);
        }

        // AC coefficients in zigzag order, with run lengths of zeros.
        let mut run = 0_u32;
        for &idx in &ZIGZAG[1..] {
            let v = q[idx];
            if v == 0 {
                run += 1;
                continue;
            }
            while run > 15 {
                let (code, len) = comp.ac.codes[0xF0];
                w.put(u32::from(code), u32::from(len));
                run -= 16;
            }
            let (n, bits) = category(v.clamp(-1023, 1023));
            let symbol = ((run << 4) | n) as usize;
            let (code, len) = comp.ac.codes[symbol];
            w.put(u32::from(code), u32::from(len));
            w.put(bits, n);
            run = 0;
        }
        if run > 0 {
            let (code, len) = comp.ac.codes[0x00];
            w.put(u32::from(code), u32::from(len));
        }
    }
}

fn segment(out: &mut Vec<u8>, marker: u8, payload: &[u8]) -> EncodeResult<()> {
    let len = u16::try_from(payload.len() + 2).map_err(|_| EncodeError::InvalidOption {
        name: "jpeg segment",
        reason: "segment is too large".to_string(),
    })?;
    out.extend_from_slice(&[0xFF, marker]);
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(payload);
    Ok(())
}

fn write_icc(out: &mut Vec<u8>, icc: &[u8]) -> EncodeResult<()> {
    if icc.is_empty() {
        return Ok(());
    }
    let count =
        u8::try_from(icc.len().div_ceil(ICC_CHUNK)).map_err(|_| EncodeError::InvalidOption {
            name: "icc profile",
            reason: "profile is too large for JPEG (more than 255 segments)".to_string(),
        })?;
    for (i, chunk) in icc.chunks(ICC_CHUNK).enumerate() {
        let mut p = Vec::with_capacity(14 + chunk.len());
        p.extend_from_slice(b"ICC_PROFILE\0");
        // i < count <= 255, so i + 1 fits in u8.
        p.push(u8::try_from(i + 1).unwrap_or(u8::MAX));
        p.push(count);
        p.extend_from_slice(chunk);
        segment(out, 0xE2, &p)?;
    }
    Ok(())
}

fn huffman_payload(class_id: u8, bits: &[u8; 16], values: &[u8], p: &mut Vec<u8>) {
    p.push(class_id);
    p.extend_from_slice(bits);
    p.extend_from_slice(values);
}

/// Converts one RGB pixel to level-shifted YCbCr (JFIF, BT.601 full range).
#[inline]
fn ycbcr(px: &[u8]) -> [f32; 3] {
    let (r, g, b) = (f32::from(px[0]), f32::from(px[1]), f32::from(px[2]));
    [
        0.299 * r + 0.587 * g + 0.114 * b - 128.0,
        -0.168_736 * r - 0.331_264 * g + 0.5 * b,
        0.5 * r - 0.418_688 * g - 0.081_312 * b,
    ]
}

/// Encodes an 8-bit RGB image as a baseline JPEG.
///
/// `image` must have the layout `Rgb`. `icc` is embedded if it is not
/// empty.
pub(crate) fn encode_jpeg(
    image: &Image8,
    quality: u8,
    subsampling: ChromaSubsampling,
    icc: &[u8],
) -> EncodeResult<Vec<u8>> {
    let (w, h) = (image.width(), image.height());
    if w > MAX_SIDE || h > MAX_SIDE {
        return Err(EncodeError::TooLarge {
            width: w,
            height: h,
            format: "JPEG",
            max: MAX_SIDE,
        });
    }
    if image.layout().channels() != 3 || image.layout().has_alpha() {
        return Err(EncodeError::Unsupported("JPEG input must be RGB"));
    }
    let (hs, vs) = match subsampling {
        ChromaSubsampling::Yuv444 => (1_usize, 1_usize),
        ChromaSubsampling::Yuv422 => (2, 1),
        ChromaSubsampling::Yuv420 => (2, 2),
    };
    let (wu, hu) = (w as usize, h as usize);
    let (mcu_w, mcu_h) = (8 * hs, 8 * vs);
    let geometry = Geometry {
        width: wu,
        height: hu,
        hs,
        vs,
        mcu_w,
        mcu_h,
        mcus_x: wu.div_ceil(mcu_w),
        mcus_y: hu.div_ceil(mcu_h),
    };
    let luma_q = scaled_table(&LUMA_Q, quality);
    let chroma_q = scaled_table(&CHROMA_Q, quality);

    let mut out = Vec::with_capacity(wu * hu / 4 + 1024);
    write_headers(&mut out, &geometry, &luma_q, &chroma_q, icc)?;
    write_scan(&mut out, image.as_slice(), &geometry, &luma_q, &chroma_q);
    out.extend_from_slice(&[0xFF, 0xD9]); // EOI
    Ok(out)
}

/// Sizes of the MCU grid.
struct Geometry {
    width: usize,
    height: usize,
    /// Horizontal and vertical luma sampling factors (1 or 2).
    hs: usize,
    vs: usize,
    /// MCU size in pixels.
    mcu_w: usize,
    mcu_h: usize,
    /// MCU count.
    mcus_x: usize,
    mcus_y: usize,
}

/// Writes all markers before the entropy-coded data.
fn write_headers(
    out: &mut Vec<u8>,
    g: &Geometry,
    luma_q: &[u16; 64],
    chroma_q: &[u16; 64],
    icc: &[u8],
) -> EncodeResult<()> {
    let too_large = || EncodeError::TooLarge {
        width: u32::try_from(g.width).unwrap_or(u32::MAX),
        height: u32::try_from(g.height).unwrap_or(u32::MAX),
        format: "JPEG",
        max: MAX_SIDE,
    };
    let w16 = u16::try_from(g.width).map_err(|_| too_large())?;
    let h16 = u16::try_from(g.height).map_err(|_| too_large())?;

    out.extend_from_slice(&[0xFF, 0xD8]); // SOI
    // APP0 JFIF 1.01, no density unit, aspect 1:1, no thumbnail.
    segment(
        out,
        0xE0,
        &[b'J', b'F', b'I', b'F', 0, 1, 1, 0, 0, 1, 0, 1, 0, 0],
    )?;
    write_icc(out, icc)?;

    // DQT: two 8-bit tables in zigzag order.
    let mut quant_tables = Vec::with_capacity(130);
    for (id, table) in [(0_u8, luma_q), (1, chroma_q)] {
        quant_tables.push(id);
        for &z in &ZIGZAG {
            // Values are clamped to 1..=255 in scaled_table.
            quant_tables.push(u8::try_from(table[z]).unwrap_or(u8::MAX));
        }
    }
    segment(out, 0xDB, &quant_tables)?;

    // SOF0: 8-bit precision, size, 3 components.
    #[allow(clippy::cast_possible_truncation, reason = "hs and vs are 1 or 2")]
    let luma_sampling = ((g.hs << 4) | g.vs) as u8;
    let mut sof = vec![8];
    sof.extend_from_slice(&h16.to_be_bytes());
    sof.extend_from_slice(&w16.to_be_bytes());
    sof.extend_from_slice(&[3, 1, luma_sampling, 0, 2, 0x11, 1, 3, 0x11, 1]);
    segment(out, 0xC0, &sof)?;

    // DHT: the four Annex K tables.
    let mut huff_tables = Vec::new();
    huffman_payload(0x00, &DC_LUMA_BITS, &DC_VALUES, &mut huff_tables);
    huffman_payload(0x10, &AC_LUMA_BITS, &AC_LUMA_VALUES, &mut huff_tables);
    huffman_payload(0x01, &DC_CHROMA_BITS, &DC_VALUES, &mut huff_tables);
    huffman_payload(0x11, &AC_CHROMA_BITS, &AC_CHROMA_VALUES, &mut huff_tables);
    segment(out, 0xC4, &huff_tables)?;

    // SOS: 3 components, full spectral range, no approximation.
    segment(out, 0xDA, &[3, 1, 0x00, 2, 0x11, 3, 0x11, 0, 63, 0])
}

/// Writes the entropy-coded data of all MCUs (interleaved scan).
///
/// `data` is interleaved 8-bit RGB with `g.width * g.height` pixels.
fn write_scan(
    out: &mut Vec<u8>,
    data: &[u8],
    g: &Geometry,
    luma_q: &[u16; 64],
    chroma_q: &[u16; 64],
) {
    let inv = |t: &[u16; 64]| t.map(|v| 1.0 / f32::from(v));
    let (wu, hu, hs, vs) = (g.width, g.height, g.hs, g.vs);
    let (mcu_w, mcu_h) = (g.mcu_w, g.mcu_h);
    let pw = g.mcus_x * mcu_w;
    let cw = pw / hs;
    let dc_luma = HuffTable::new(&DC_LUMA_BITS, &DC_VALUES);
    let ac_luma = HuffTable::new(&AC_LUMA_BITS, &AC_LUMA_VALUES);
    let dc_chroma = HuffTable::new(&DC_CHROMA_BITS, &DC_VALUES);
    let ac_chroma = HuffTable::new(&AC_CHROMA_BITS, &AC_CHROMA_VALUES);
    let mut comps = [
        Component {
            inv_q: inv(luma_q),
            dc: &dc_luma,
            ac: &ac_luma,
            pred: 0,
        },
        Component {
            inv_q: inv(chroma_q),
            dc: &dc_chroma,
            ac: &ac_chroma,
            pred: 0,
        },
        Component {
            inv_q: inv(chroma_q),
            dc: &dc_chroma,
            ac: &ac_chroma,
            pred: 0,
        },
    ];
    let coder = BlockCoder { dct: dct_matrix() };

    // One strip of MCUs at a time: full-resolution planes of `mcu_h` rows,
    // and subsampled chroma planes of 8 rows.
    let mut y_strip = vec![0.0_f32; mcu_h * pw];
    let mut blue_full = vec![0.0_f32; mcu_h * pw];
    let mut red_full = vec![0.0_f32; mcu_h * pw];
    let mut blue_strip = vec![0.0_f32; 8 * cw];
    let mut red_strip = vec![0.0_f32; 8 * cw];
    let row_len = wu * 3;
    let mut block = [0.0_f32; 64];
    let mut writer = BitWriter::new(out);

    for my in 0..g.mcus_y {
        // Fill the strip; rows and columns past the edge repeat the last.
        for sy in 0..mcu_h {
            let src_y = (my * mcu_h + sy).min(hu - 1);
            let src = &data[src_y * row_len..(src_y + 1) * row_len];
            for x in 0..pw {
                let sx = x.min(wu - 1);
                let [yv, cb, cr] = ycbcr(&src[sx * 3..sx * 3 + 3]);
                let i = sy * pw + x;
                y_strip[i] = yv;
                blue_full[i] = cb;
                red_full[i] = cr;
            }
        }
        // Subsample chroma with a box filter.
        #[allow(clippy::cast_precision_loss, reason = "hs * vs is at most 4")]
        let norm = 1.0 / (hs * vs) as f32;
        for cy in 0..8 {
            for cx in 0..cw {
                let (mut sb, mut sr) = (0.0, 0.0);
                for dy in 0..vs {
                    for dx in 0..hs {
                        let i = (cy * vs + dy) * pw + cx * hs + dx;
                        sb += blue_full[i];
                        sr += red_full[i];
                    }
                }
                blue_strip[cy * cw + cx] = sb * norm;
                red_strip[cy * cw + cx] = sr * norm;
            }
        }
        for mx in 0..g.mcus_x {
            for by in 0..vs {
                for bx in 0..hs {
                    let x0 = mx * mcu_w + bx * 8;
                    for r in 0..8 {
                        let s = (by * 8 + r) * pw + x0;
                        block[r * 8..r * 8 + 8].copy_from_slice(&y_strip[s..s + 8]);
                    }
                    coder.encode(&block, &mut comps[0], &mut writer);
                }
            }
            for (plane, comp) in [(&blue_strip, 1), (&red_strip, 2)] {
                let x0 = mx * 8;
                for r in 0..8 {
                    let s = r * cw + x0;
                    block[r * 8..r * 8 + 8].copy_from_slice(&plane[s..s + 8]);
                }
                coder.encode(&block, &mut comps[comp], &mut writer);
            }
        }
    }
    writer.flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn huffman_specs_are_complete() {
        let total = |b: &[u8; 16]| b.iter().map(|v| usize::from(*v)).sum::<usize>();
        assert_eq!(total(&DC_LUMA_BITS), DC_VALUES.len());
        assert_eq!(total(&DC_CHROMA_BITS), DC_VALUES.len());
        assert_eq!(total(&AC_LUMA_BITS), AC_LUMA_VALUES.len());
        assert_eq!(total(&AC_CHROMA_BITS), AC_CHROMA_VALUES.len());
        // Every symbol that the coder can emit has a code.
        for t in [
            HuffTable::new(&AC_LUMA_BITS, &AC_LUMA_VALUES),
            HuffTable::new(&AC_CHROMA_BITS, &AC_CHROMA_VALUES),
        ] {
            for run in 0..16_usize {
                for n in 1..=10_usize {
                    assert!(t.codes[(run << 4) | n].1 > 0, "run {run} size {n}");
                }
            }
            assert!(t.codes[0x00].1 > 0 && t.codes[0xF0].1 > 0);
        }
    }

    #[test]
    fn categories() {
        assert_eq!(category(0), (0, 0));
        assert_eq!(category(1), (1, 1));
        assert_eq!(category(-1), (1, 0));
        assert_eq!(category(3), (2, 3));
        assert_eq!(category(-3), (2, 0));
        assert_eq!(category(-2047), (11, 0));
        assert_eq!(category(1023), (10, 1023));
    }

    #[test]
    fn quality_scaling() {
        assert!(scaled_table(&LUMA_Q, 100).iter().all(|v| *v == 1));
        assert_eq!(scaled_table(&LUMA_Q, 50), LUMA_Q);
        assert!(scaled_table(&LUMA_Q, 1).iter().all(|v| *v <= 255));
    }

    #[test]
    fn byte_stuffing() {
        let mut out = Vec::new();
        let mut w = BitWriter::new(&mut out);
        w.put(0xFF, 8);
        w.put(0x1, 1);
        w.flush();
        assert_eq!(out, vec![0xFF, 0x00, 0xFF, 0x00]);
    }
}
