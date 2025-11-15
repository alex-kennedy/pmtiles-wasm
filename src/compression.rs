use crate::{PMTilesError, PMTilesResult};
use std::io::Read;
use wasm_bindgen::prelude::*;

/// Compression types for tile or directory data.
///
/// See <https://github.com/protomaps/PMTiles/blob/main/spec/v3/spec.md#33-compression>.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[wasm_bindgen]
pub enum Compression {
    /// Unknown compression.
    Unknown,

    /// No compression.
    None,

    /// gzip compression.
    Gzip,

    /// Brotli compression.
    Brotli,

    /// zstd compression.
    Zstd,
}

/// Decompress data according to a valid compression option for PMTiles. If the compression setting
/// is unknown, the raw data is returned and the caller will need to manage decompression, if
/// necessary.
pub fn decompress<R: Read>(r: R, compression: Compression) -> PMTilesResult<Vec<u8>> {
    match compression {
        Compression::Unknown => read_all(r),
        Compression::None => read_all(r),
        #[cfg(feature = "gzip")]
        Compression::Gzip => read_all(flate2::read::GzDecoder::new(r)),
        #[cfg(feature = "brotli")]
        Compression::Brotli => read_all(brotli::Decompressor::new(r, 4096)),
        #[cfg(feature = "zstd")]
        Compression::Zstd => read_all(ruzstd::decoding::StreamingDecoder::new(r)?),
        #[allow(unreachable_patterns)]
        _ => Err(PMTilesError::UnsupportedCompression(compression)),
    }
}

impl std::fmt::Display for Compression {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        std::fmt::Debug::fmt(self, f)
    }
}

/// Converts the compression byte into a compression enum value according to the PMTiles spec.
///
/// See <https://github.com/protomaps/PMTiles/blob/main/spec/v3/spec.md#33-compression>.
impl TryFrom<u8> for Compression {
    type Error = PMTilesError;

    fn try_from(value: u8) -> PMTilesResult<Self> {
        match value {
            0 => Ok(Compression::Unknown),
            1 => Ok(Compression::None),
            2 => Ok(Compression::Gzip),
            3 => Ok(Compression::Brotli),
            4 => Ok(Compression::Zstd),
            _ => Err(PMTilesError::HeaderDecodeError(format!(
                "invalid compression value {}",
                value
            ))),
        }
    }
}

/// Simply reads the reader into a new vector.
fn read_all<R: Read>(mut r: R) -> PMTilesResult<Vec<u8>> {
    let mut bytes = Vec::new();
    r.read_to_end(&mut bytes)?;
    Ok(bytes)
}
