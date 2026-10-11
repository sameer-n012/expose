//! Blob encode and decode: `f32` values, byte shuffle, raw deflate, CRC-32.
//!
//! The byte shuffle writes byte 0 (the least significant byte of the
//! little-endian `f32`) of all values, then byte 1 of all values, then
//! byte 2, then byte 3. Similar values then give long runs of similar
//! bytes (sign and exponent), which deflate compresses well.

use std::io::Read;

use miniz_oxide::deflate::core::{
    CompressionStrategy, CompressorOxide, TDEFLFlush, TDEFLStatus, compress_to_output,
};
use miniz_oxide::inflate::stream::{InflateState, inflate};
use miniz_oxide::{DataFormat, MZError, MZFlush, MZStatus};

use super::FileError;

/// Number of bytes in one `f32`.
pub(super) const VALUE_BYTES: usize = 4;

/// Size of the work buffers.
const CHUNK: usize = 1 << 16;

/// The deflate level of the writer (1 = fastest, 9 = smallest).
///
/// Level 1 keeps the write time of a 24 MP layer near 2 s in a release
/// build. The low mantissa bytes of photo data are close to noise, so
/// higher levels give only a small decrease in size for a large increase
/// in time.
const LEVEL: u8 = 1;

/// A compressed blob.
pub(super) struct EncodedBlob {
    /// The stored bytes (raw deflate).
    pub bytes: Vec<u8>,
    /// The number of bytes before compression.
    pub raw_length: u64,
    /// CRC-32 of `bytes`.
    pub crc32: u32,
}

/// Shuffles and compresses `values`.
pub(super) fn encode(values: &[f32]) -> Result<EncodedBlob, FileError> {
    let mut compressor =
        CompressorOxide::with_params(DataFormat::Raw, LEVEL, CompressionStrategy::Default, 15);
    let mut out = Vec::new();
    let mut shuffled = vec![0_u8; CHUNK];
    for plane in 0..VALUE_BYTES {
        for chunk in values.chunks(CHUNK) {
            for (dst, v) in shuffled.iter_mut().zip(chunk) {
                *dst = v.to_le_bytes()[plane];
            }
            deflate_chunk(&mut compressor, &shuffled[..chunk.len()], &mut out, false)?;
        }
    }
    deflate_chunk(&mut compressor, &[], &mut out, true)?;
    let raw_length = u64::try_from(values.len())
        .ok()
        .and_then(|n| n.checked_mul(VALUE_BYTES as u64))
        .ok_or(FileError::Invalid("blob is too large"))?;
    let crc32 = crc32fast::hash(&out);
    Ok(EncodedBlob {
        bytes: out,
        raw_length,
        crc32,
    })
}

/// Gives one input chunk to the compressor and appends the output to `out`.
fn deflate_chunk(
    compressor: &mut CompressorOxide,
    input: &[u8],
    out: &mut Vec<u8>,
    finish: bool,
) -> Result<(), FileError> {
    let flush = if finish {
        TDEFLFlush::Finish
    } else {
        TDEFLFlush::None
    };
    let (status, consumed) = compress_to_output(compressor, input, flush, |bytes| {
        out.extend_from_slice(bytes);
        true
    });
    let expected = if finish {
        TDEFLStatus::Done
    } else {
        TDEFLStatus::Okay
    };
    if status != expected || consumed != input.len() {
        return Err(FileError::Invalid("deflate compressor failed"));
    }
    Ok(())
}

/// Reads `compressed_length` stored bytes of blob `index` from `input`,
/// checks them, and decodes them into `dst`.
///
/// `dst.len() * 4` must be the raw length of the blob (the caller checks
/// this against the blob table). The function reads exactly
/// `compressed_length` bytes, also when the data is not valid, so that the
/// CRC check covers all bytes. A CRC error has priority over a deflate
/// error.
pub(super) fn decode<R: Read>(
    input: &mut R,
    index: usize,
    compressed_length: u64,
    expected_crc: u32,
    dst: &mut [f32],
) -> Result<(), FileError> {
    let value_count = dst.len();
    let bytes: &mut [u8] = bytemuck::cast_slice_mut(dst);
    let raw_length = bytes.len();
    let mut state = InflateState::new_boxed(DataFormat::Raw);
    let mut hasher = crc32fast::Hasher::new();
    let mut in_buf = vec![0_u8; CHUNK];
    let mut out_buf = vec![0_u8; CHUNK];
    let mut remaining = compressed_length;
    let mut pos = 0_usize;
    let mut ended = false;
    let mut failure: Option<&'static str> = None;

    while remaining > 0 {
        // CHUNK fits in u64, and the minimum is not more than CHUNK, so it
        // fits in usize.
        let want = usize::try_from(remaining.min(CHUNK as u64)).unwrap_or(CHUNK);
        input
            .read_exact(&mut in_buf[..want])
            .map_err(FileError::from)?;
        remaining -= want as u64;
        hasher.update(&in_buf[..want]);
        if failure.is_some() {
            continue;
        }
        if ended {
            failure = Some("data after the end of the deflate stream");
            continue;
        }
        let mut data = &in_buf[..want];
        loop {
            let r = inflate(&mut state, data, &mut out_buf, MZFlush::None);
            data = &data[r.bytes_consumed..];
            let out = &out_buf[..r.bytes_written];
            if out.len() > raw_length - pos {
                failure = Some("more data than the raw length");
                break;
            }
            scatter(bytes, value_count, pos, out);
            pos += out.len();
            match r.status {
                Ok(MZStatus::StreamEnd) => {
                    ended = true;
                    if !data.is_empty() {
                        failure = Some("data after the end of the deflate stream");
                    }
                    break;
                }
                Ok(_) => {
                    let progress = r.bytes_consumed > 0 || r.bytes_written > 0;
                    let pending = r.bytes_written == out_buf.len();
                    if data.is_empty() && !pending {
                        break;
                    }
                    if !progress {
                        failure = Some("deflate stream makes no progress");
                        break;
                    }
                }
                // No progress without more input.
                Err(MZError::Buf) if data.is_empty() => break,
                Err(_) => {
                    failure = Some("deflate stream is not valid");
                    break;
                }
            }
        }
    }

    if hasher.finalize() != expected_crc {
        return Err(FileError::Crc { blob: index });
    }
    if let Some(reason) = failure {
        return Err(FileError::Blob {
            blob: index,
            reason,
        });
    }
    if !ended {
        return Err(FileError::Blob {
            blob: index,
            reason: "deflate stream ends early",
        });
    }
    if pos != raw_length {
        return Err(FileError::Blob {
            blob: index,
            reason: "less data than the raw length",
        });
    }
    #[cfg(target_endian = "big")]
    for v in dst.iter_mut() {
        *v = f32::from_bits(u32::from_le(v.to_bits()));
    }
    Ok(())
}

/// Writes shuffled stream bytes `pos..pos + data.len()` to their place in
/// the little-endian value bytes. The caller makes sure that the end is not
/// after `bytes.len()`.
fn scatter(bytes: &mut [u8], value_count: usize, mut pos: usize, mut data: &[u8]) {
    while !data.is_empty() {
        let plane = pos / value_count;
        let first = pos % value_count;
        let run = data.len().min(value_count - first);
        let dst = &mut bytes[first * VALUE_BYTES..(first + run) * VALUE_BYTES];
        for (value, &b) in dst.chunks_exact_mut(VALUE_BYTES).zip(&data[..run]) {
            value[plane] = b;
        }
        pos += run;
        data = &data[run..];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_decode_round_trip() {
        let values: Vec<f32> = (0..100_000_u32)
            .map(|i| f32::from_bits(i.wrapping_mul(2_654_435_761)))
            .collect();
        let blob = encode(&values).unwrap();
        assert_eq!(blob.raw_length, 400_000);
        let mut out = vec![0.0_f32; values.len()];
        decode(
            &mut blob.bytes.as_slice(),
            0,
            blob.bytes.len() as u64,
            blob.crc32,
            &mut out,
        )
        .unwrap();
        let a: Vec<u32> = values.iter().map(|v| v.to_bits()).collect();
        let b: Vec<u32> = out.iter().map(|v| v.to_bits()).collect();
        assert_eq!(a, b);
    }

    #[test]
    fn decode_rejects_wrong_raw_length() {
        let values = vec![0.25_f32; 1000];
        let blob = encode(&values).unwrap();
        for len in [999, 1001] {
            let mut out = vec![0.0_f32; len];
            let err = decode(
                &mut blob.bytes.as_slice(),
                3,
                blob.bytes.len() as u64,
                blob.crc32,
                &mut out,
            )
            .unwrap_err();
            assert!(matches!(err, FileError::Blob { blob: 3, .. }), "{err:?}");
        }
    }
}
