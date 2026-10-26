use crate::{PMTilesError, PMTilesResult};
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
pub async fn decompress(bytes: &[u8], compression: Compression) -> PMTilesResult<Vec<u8>> {
    match compression {
        Compression::Unknown => Ok(bytes.to_vec()),
        Compression::None => Ok(bytes.to_vec()),
        #[cfg(feature = "gzip")]
        Compression::Gzip => gunzip(bytes).await,
        #[cfg(feature = "brotli")]
        Compression::Brotli => read_all(brotli::Decompressor::new(bytes, 4096)),
        #[cfg(feature = "zstd")]
        Compression::Zstd => read_all(ruzstd::decoding::StreamingDecoder::new(bytes)?),
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

#[cfg(feature = "gzip")]
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(extends = web_sys::ReadableWritablePair)]
    type DecompressionStream;

    #[wasm_bindgen(constructor)]
    fn new(format: &str) -> DecompressionStream;
}

#[cfg(feature = "gzip")]
/// Decompresses gzip data with the browser's native `DecompressionStream`.
async fn gunzip(bytes: &[u8]) -> PMTilesResult<Vec<u8>> {
    let input =
        web_sys::Response::new_with_opt_buffer_source(Some(&js_sys::Uint8Array::from(bytes)))?;
    let body = input
        .body()
        .ok_or_else(|| PMTilesError::JsError("response has no body".to_string()))?;
    let decompressed = body.pipe_through(&DecompressionStream::new("gzip"));
    let output = web_sys::Response::new_with_opt_readable_stream(Some(&decompressed))?;
    let buffer = wasm_bindgen_futures::JsFuture::from(output.array_buffer()?).await?;
    Ok(js_sys::Uint8Array::new(&buffer).to_vec())
}

/// Simply reads the reader into a new vector.
#[cfg(any(feature = "brotli", feature = "zstd"))]
fn read_all<R: std::io::Read>(mut r: R) -> PMTilesResult<Vec<u8>> {
    let mut bytes = Vec::new();
    r.read_to_end(&mut bytes)?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::wasm_bindgen_test;

    #[cfg(feature = "gzip")]
    #[wasm_bindgen_test]
    async fn gunzips() {
        let bytes = include_bytes!("../testdata/test_fixture_1.pmtiles");

        let metadata = decompress(&bytes[152..399], Compression::Gzip)
            .await
            .unwrap();

        assert!(String::from_utf8(metadata).unwrap().starts_with('{'));
    }

    #[cfg(feature = "gzip")]
    #[wasm_bindgen_test]
    async fn gunzip_rejects_invalid_data() {
        let bytes = b"not gzip";

        let result = decompress(bytes, Compression::Gzip).await;

        assert!(matches!(result, Err(PMTilesError::JsError(_))));
    }
}
