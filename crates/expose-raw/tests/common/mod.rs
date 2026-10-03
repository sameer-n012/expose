//! Test helpers: a small little-endian TIFF writer, a synthetic DNG
//! builder, and an EXIF block builder.
#![allow(dead_code, reason = "each test file uses a part of the helpers")]
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    reason = "test data is small"
)]
#![allow(clippy::unwrap_used, reason = "test helpers")]

/// A TIFF field value.
#[derive(Debug, Clone)]
pub enum Val {
    Byte(Vec<u8>),
    Ascii(String),
    Short(Vec<u16>),
    Long(Vec<u32>),
    Rational(Vec<(u32, u32)>),
    SRational(Vec<(i32, i32)>),
    Undefined(Vec<u8>),
    /// A pointer to a nested IFD (type LONG).
    Ifd(Box<Ifd>),
    /// Image data; the entry gets the offset of the data (type LONG).
    Data(Vec<u8>),
}

/// An IFD: tag and value pairs.
#[derive(Debug, Clone, Default)]
pub struct Ifd(pub Vec<(u16, Val)>);

impl Ifd {
    pub fn with(mut self, tag: u16, v: Val) -> Self {
        self.0.push((tag, v));
        self
    }
}

fn encode(v: &Val) -> (u16, u32, Vec<u8>) {
    let mut b = Vec::new();
    match v {
        Val::Byte(x) => {
            b.extend_from_slice(x);
            (1, x.len() as u32, b)
        }
        Val::Ascii(s) => {
            b.extend_from_slice(s.as_bytes());
            b.push(0);
            (2, b.len() as u32, b)
        }
        Val::Short(x) => {
            x.iter().for_each(|v| b.extend_from_slice(&v.to_le_bytes()));
            (3, x.len() as u32, b)
        }
        Val::Long(x) => {
            x.iter().for_each(|v| b.extend_from_slice(&v.to_le_bytes()));
            (4, x.len() as u32, b)
        }
        Val::Rational(x) => {
            for (n, d) in x {
                b.extend_from_slice(&n.to_le_bytes());
                b.extend_from_slice(&d.to_le_bytes());
            }
            (5, x.len() as u32, b)
        }
        Val::SRational(x) => {
            for (n, d) in x {
                b.extend_from_slice(&n.to_le_bytes());
                b.extend_from_slice(&d.to_le_bytes());
            }
            (10, x.len() as u32, b)
        }
        Val::Undefined(x) => {
            b.extend_from_slice(x);
            (7, x.len() as u32, b)
        }
        Val::Ifd(_) | Val::Data(_) => (4, 1, vec![0; 4]),
    }
}

fn align(buf: &mut Vec<u8>) {
    if buf.len() % 2 == 1 {
        buf.push(0);
    }
}

/// Writes `ifd` at the end of `buf` and returns its offset. `base` is the
/// position of the TIFF header in `buf` (offsets are relative to it).
fn write_ifd(buf: &mut Vec<u8>, base: usize, ifd: &Ifd) -> u32 {
    align(buf);
    let mut entries = ifd.0.clone();
    entries.sort_by_key(|(t, _)| *t);
    let pos = buf.len();
    let n = entries.len();
    buf.resize(pos + 2 + 12 * n + 4, 0);
    buf[pos..pos + 2].copy_from_slice(&(n as u16).to_le_bytes());
    for (i, (tag, v)) in entries.iter().enumerate() {
        let (typ, count, bytes) = encode(v);
        let field: [u8; 4] = match v {
            Val::Ifd(sub) => write_ifd(buf, base, sub).to_le_bytes(),
            Val::Data(d) => {
                align(buf);
                let off = (buf.len() - base) as u32;
                buf.extend_from_slice(d);
                off.to_le_bytes()
            }
            _ if bytes.len() <= 4 => {
                let mut f = [0_u8; 4];
                f[..bytes.len()].copy_from_slice(&bytes);
                f
            }
            _ => {
                align(buf);
                let off = (buf.len() - base) as u32;
                buf.extend_from_slice(&bytes);
                off.to_le_bytes()
            }
        };
        let e = pos + 2 + 12 * i;
        buf[e..e + 2].copy_from_slice(&tag.to_le_bytes());
        buf[e + 2..e + 4].copy_from_slice(&typ.to_le_bytes());
        buf[e + 4..e + 8].copy_from_slice(&count.to_le_bytes());
        buf[e + 8..e + 12].copy_from_slice(&field);
    }
    (pos - base) as u32
}

/// Writes a little-endian TIFF file with one IFD (and its nested IFDs).
pub fn tiff(ifd0: &Ifd) -> Vec<u8> {
    let mut buf = b"II*\0\0\0\0\0".to_vec();
    let off = write_ifd(&mut buf, 0, ifd0);
    buf[4..8].copy_from_slice(&off.to_le_bytes());
    buf
}

/// Rational approximation of `v` with denominator 10000.
pub fn srat(v: f64) -> (i32, i32) {
    ((v * 10000.0).round() as i32, 10000)
}

/// Unsigned rational approximation of `v` with denominator 10000.
pub fn rat(v: f64) -> (u32, u32) {
    ((v * 10000.0).round() as u32, 10000)
}

/// XYZ (D65) to linear sRGB, row-major: a "camera" that sees linear sRGB.
pub fn xyz_to_srgb() -> [f64; 9] {
    let m = expose_core::color::ColorSpaceId::Srgb
        .definition()
        .to_xyz()
        .unwrap()
        .inverse()
        .unwrap();
    let r = m.rows;
    [
        r[0][0], r[0][1], r[0][2], r[1][0], r[1][1], r[1][2], r[2][0], r[2][1], r[2][2],
    ]
}

/// A synthetic DNG description.
#[derive(Debug, Clone)]
pub struct DngSpec {
    pub width: u32,
    pub height: u32,
    /// CFA pattern colors (0 = R, 1 = G, 2 = B), row-major 2x2.
    pub pattern: [u8; 4],
    pub black: u16,
    pub white: u16,
    pub bits: u16,
    pub neutral: [f64; 3],
    pub color_matrix: [f64; 9],
    pub orientation: u16,
    /// Top, left, bottom, right.
    pub active_area: Option<[u32; 4]>,
    /// Origin (x, y) and size (w, h) relative to the active area.
    pub crop: Option<([u32; 2], [u32; 2])>,
    pub samples: Vec<u16>,
    pub make: &'static str,
    pub model: &'static str,
    pub iso: Option<u16>,
}

impl DngSpec {
    /// A spec with all cells set by `f(color, x, y)`.
    pub fn new(width: u32, height: u32, pattern: [u8; 4], f: impl Fn(u8, u32, u32) -> u16) -> Self {
        let mut samples = Vec::new();
        for y in 0..height {
            for x in 0..width {
                let c = pattern[((y % 2) * 2 + x % 2) as usize];
                samples.push(f(c, x, y));
            }
        }
        Self {
            width,
            height,
            pattern,
            black: 0,
            white: 65535,
            bits: 16,
            neutral: [1.0, 1.0, 1.0],
            color_matrix: xyz_to_srgb(),
            orientation: 1,
            active_area: None,
            crop: None,
            samples,
            make: "Synthetic",
            model: "Test Sensor",
            iso: None,
        }
    }

    /// Builds the DNG file: an uncompressed 16-bit CFA image in IFD0.
    pub fn build(&self) -> Vec<u8> {
        let data: Vec<u8> = self.samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        let mut ifd = Ifd::default()
            .with(254, Val::Long(vec![0]))
            .with(256, Val::Long(vec![self.width]))
            .with(257, Val::Long(vec![self.height]))
            .with(258, Val::Short(vec![self.bits]))
            .with(259, Val::Short(vec![1]))
            .with(262, Val::Short(vec![32803]))
            .with(271, Val::Ascii(self.make.into()))
            .with(272, Val::Ascii(self.model.into()))
            .with(273, Val::Data(data.clone()))
            .with(274, Val::Short(vec![self.orientation]))
            .with(277, Val::Short(vec![1]))
            .with(278, Val::Long(vec![self.height]))
            .with(279, Val::Long(vec![data.len() as u32]))
            .with(284, Val::Short(vec![1]))
            .with(33421, Val::Short(vec![2, 2]))
            .with(33422, Val::Byte(self.pattern.to_vec()))
            .with(50706, Val::Byte(vec![1, 4, 0, 0]))
            .with(50707, Val::Byte(vec![1, 1, 0, 0]))
            .with(50708, Val::Ascii(format!("{} {}", self.make, self.model)))
            .with(50710, Val::Byte(vec![0, 1, 2]))
            .with(50711, Val::Short(vec![1]))
            .with(50713, Val::Short(vec![1, 1]))
            .with(50714, Val::Long(vec![u32::from(self.black)]))
            .with(50717, Val::Long(vec![u32::from(self.white)]))
            .with(
                50721,
                Val::SRational(self.color_matrix.iter().map(|v| srat(*v)).collect()),
            )
            .with(
                50728,
                Val::Rational(self.neutral.iter().map(|v| rat(*v)).collect()),
            )
            .with(50778, Val::Short(vec![21]));
        if let Some(a) = self.active_area {
            ifd = ifd.with(50829, Val::Long(a.to_vec()));
        }
        if let Some((o, s)) = self.crop {
            ifd = ifd
                .with(50719, Val::Long(o.to_vec()))
                .with(50720, Val::Long(s.to_vec()));
        }
        if let Some(iso) = self.iso {
            let exif = Ifd::default()
                .with(0x8827, Val::Short(vec![iso]))
                .with(0x9003, Val::Ascii("2025:01:02 03:04:05".into()));
            ifd = ifd.with(0x8769, Val::Ifd(Box::new(exif)));
        }
        tiff(&ifd)
    }
}

/// Mutates `data` for fuzz-style tests: truncations and byte changes at
/// many positions. Calls `f` for each variant.
pub fn for_each_mutation(data: &[u8], step: usize, mut f: impl FnMut(&[u8])) {
    let step = step.max(1);
    for n in (0..data.len()).step_by(step) {
        f(&data[..n]);
    }
    let mut copy = data.to_vec();
    for i in (0..data.len()).step_by(step) {
        for x in [0xFF_u8, 0x80, 0x01] {
            let orig = copy[i];
            copy[i] ^= x;
            f(&copy);
            copy[i] = orig;
        }
    }
}
