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
    use rstest::rstest;
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

    #[rstest]
    #[test_attr(wasm_bindgen_test(unsupported = test))]
    #[case(0, Compression::Unknown)]
    #[case(1, Compression::None)]
    #[case(2, Compression::Gzip)]
    #[case(3, Compression::Brotli)]
    #[case(4, Compression::Zstd)]
    fn compression_from_byte(#[case] value: u8, #[case] want: Compression) {
        let got = Compression::try_from(value).unwrap();

        assert_eq!(got, want);
    }

    #[wasm_bindgen_test(unsupported = test)]
    fn compression_from_invalid_byte_fails() {
        let result = Compression::try_from(5);

        assert!(matches!(result, Err(PMTilesError::HeaderDecodeError(_))));
    }

    #[rstest]
    #[test_attr(wasm_bindgen_test(unsupported = futures_test::test))]
    #[case(Compression::Unknown)]
    #[case(Compression::None)]
    async fn uncompressed_data_is_returned_as_is(#[case] compression: Compression) {
        let bytes = b"tile data";

        let got = decompress(bytes, compression).await.unwrap();

        assert_eq!(got, bytes);
    }

    #[cfg(not(feature = "gzip"))]
    #[wasm_bindgen_test(unsupported = futures_test::test)]
    async fn gzip_is_unsupported_without_feature() {
        let result = decompress(b"data", Compression::Gzip).await;

        assert!(matches!(
            result,
            Err(PMTilesError::UnsupportedCompression(Compression::Gzip))
        ));
    }

    #[cfg(not(feature = "brotli"))]
    #[wasm_bindgen_test(unsupported = futures_test::test)]
    async fn brotli_is_unsupported_without_feature() {
        let result = decompress(b"data", Compression::Brotli).await;

        assert!(matches!(
            result,
            Err(PMTilesError::UnsupportedCompression(Compression::Brotli))
        ));
    }

    #[cfg(not(feature = "zstd"))]
    #[wasm_bindgen_test(unsupported = futures_test::test)]
    async fn zstd_is_unsupported_without_feature() {
        let result = decompress(b"data", Compression::Zstd).await;

        assert!(matches!(
            result,
            Err(PMTilesError::UnsupportedCompression(Compression::Zstd))
        ));
    }

    #[cfg(feature = "brotli")]
    #[wasm_bindgen_test(unsupported = futures_test::test)]
    async fn brotli_round_trips() {
        use std::io::Write;
        let mut compressed = Vec::new();
        brotli::CompressorWriter::new(&mut compressed, 4096, 5, 22)
            .write_all(b"tile data")
            .unwrap();

        let got = decompress(&compressed, Compression::Brotli).await.unwrap();

        assert_eq!(got, b"tile data");
    }

    #[cfg(feature = "brotli")]
    #[wasm_bindgen_test(unsupported = futures_test::test)]
    async fn brotli_rejects_invalid_data() {
        let result = decompress(b"not brotli", Compression::Brotli).await;

        assert!(matches!(result, Err(PMTilesError::IOError(_))));
    }

    #[cfg(feature = "zstd")]
    #[wasm_bindgen_test(unsupported = futures_test::test)]
    async fn zstd_round_trips() {
        let compressed = ruzstd::encoding::compress_to_vec(
            b"tile data".as_slice(),
            ruzstd::encoding::CompressionLevel::Fastest,
        );

        let got = decompress(&compressed, Compression::Zstd).await.unwrap();

        assert_eq!(got, b"tile data");
    }

    #[cfg(feature = "zstd")]
    #[wasm_bindgen_test(unsupported = futures_test::test)]
    async fn zstd_rejects_invalid_data() {
        let result = decompress(b"not zstd", Compression::Zstd).await;

        assert!(matches!(result, Err(PMTilesError::ZstdDecodeError(_))));
    }
}
