mod cache;
mod compression;
mod directory;
mod error;
mod fetch_source;
mod header;
mod reader;
mod source;
mod tile_id;

pub use compression::*;
pub use error::*;
pub use reader::PMTilesReader;
pub use source::*;
pub use tile_id::*;

#[cfg(test)]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

pub fn set_panic_hook() {
    // When the `console_error_panic_hook` feature is enabled, we can call the
    // `set_panic_hook` function at least once during initialization, and then
    // we will get better error messages if our code ever panics.
    //
    // For more details see
    // https://github.com/rustwasm/console_error_panic_hook#readme
    #[cfg(feature = "console_error_panic_hook")]
    console_error_panic_hook::set_once();
}
