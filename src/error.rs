use thiserror::Error;
use wasm_bindgen::prelude::*;

/// Result for PMTilesError. Returned by most functions, including to JS.
pub type PMTilesResult<T> = Result<T, PMTilesError>;

#[derive(Error, Debug)]
pub enum PMTilesError {
    /// Tile ID or zoom level causes an overflow.
    #[error("Tile overflow error")]
    TileOverflowError,
}

impl Into<JsValue> for PMTilesError {
    fn into(self) -> JsValue {
        JsError::new(&self.to_string()).into()
    }
}
