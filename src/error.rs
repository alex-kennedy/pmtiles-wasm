use std::rc::Rc;
use std::string::FromUtf8Error;

use thiserror::Error;
use wasm_bindgen::prelude::*;

use crate::compression::Compression;

/// Result for PMTilesError. Returned by most functions, including to JS.
pub type PMTilesResult<T> = Result<T, PMTilesError>;

#[derive(Error, Debug, Clone)]
pub enum PMTilesError {
    /// Failed to decode a PMTiles directory.
    #[error("Error decoding directory: {0}")]
    DirectoryDecodeError(String),
    /// Failed to decode the PMTiles header.
    #[error("Error decoding header: {0}")]
    HeaderDecodeError(String),
    /// Error from the reqwest library.
    #[error("Error from reqwest: {0}")]
    ReqwestError(Rc<reqwest::Error>),
    /// PMTiles archive changes between successive requests, potentially making the header invalid.
    #[error("PMTiles file changed between (potentially cached) requests")]
    ContentChangedError,
    /// PMTiles archive is invalid in some way. For example, this could refer to inconsistencies
    /// between the header and the file size.
    #[error("The PMTiles archive is invalid: {0}")]
    InvalidArchiveError(String),
    /// Attempted to decompress tile or directory data using an unsupported compression type.
    /// Compression types are behind features, and may be disabled.
    #[error("Unsupported compression type: {0}")]
    UnsupportedCompression(Compression),
    /// Wrapped std::io::Error.
    #[error("IO error: {0}")]
    IOError(Rc<std::io::Error>),
    /// Wrapped utf-8 error.
    #[error("utf-8 error: {0}")]
    FromUtf8Error(#[from] FromUtf8Error),
    /// Tile ID or zoom level causes an overflow.
    #[error("Tile overflow error")]
    TileOverflowError,
    /// URL parse error.
    #[error("url parse error: {0}")]
    UrlParseError(#[from] url::ParseError),
    /// Unsupported status code from a server fetch, i.e. not 200 or 206.
    #[error("unsupported status code, expected 200 or 206, got {0}")]
    UnsupportedStatusCode(reqwest::StatusCode),
    /// The server answered a range request with the whole file, or without saying how long the
    /// response is.
    #[error(
        "server returned 200 without a Content-Length within the requested range, check that it supports HTTP range requests"
    )]
    ByteServingUnsupported,
    /// A custom request header has an invalid name or value, or cannot be overridden.
    #[error("invalid header: {0}")]
    InvalidHeaderError(String),
    /// Error thrown by a browser API.
    #[error("JavaScript error: {0}")]
    JsError(String),
    /// Failed to decode zstd-compressed data.
    #[cfg(feature = "zstd")]
    #[error("zstd decode error: {0}")]
    ZstdDecodeError(Rc<ruzstd::decoding::errors::FrameDecoderError>),
}

impl From<reqwest::Error> for PMTilesError {
    fn from(value: reqwest::Error) -> Self {
        PMTilesError::ReqwestError(Rc::new(value))
    }
}

impl From<std::io::Error> for PMTilesError {
    fn from(value: std::io::Error) -> Self {
        PMTilesError::IOError(Rc::new(value))
    }
}

#[cfg(feature = "zstd")]
impl From<ruzstd::decoding::errors::FrameDecoderError> for PMTilesError {
    fn from(value: ruzstd::decoding::errors::FrameDecoderError) -> Self {
        PMTilesError::ZstdDecodeError(Rc::new(value))
    }
}

impl From<JsValue> for PMTilesError {
    fn from(value: JsValue) -> Self {
        PMTilesError::JsError(format!("{:?}", value))
    }
}

impl From<PMTilesError> for JsValue {
    fn from(value: PMTilesError) -> Self {
        JsError::new(&value.to_string()).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::wasm_bindgen_test;

    #[wasm_bindgen_test]
    fn js_value_converts_to_js_error() {
        let error = PMTilesError::from(JsValue::from_str("boom"));

        assert!(matches!(&error, PMTilesError::JsError(message) if message.contains("boom")));
    }

    #[wasm_bindgen_test]
    fn error_converts_to_js_error_with_message() {
        let error = PMTilesError::ByteServingUnsupported;
        let message = error.to_string();

        let value = JsValue::from(error);

        let js_error = value.dyn_into::<js_sys::Error>().unwrap();
        assert_eq!(String::from(js_error.message()), message);
    }
}
