#![cfg(target_arch = "wasm32")]

use pmtiles_wasm::{Compression, FetchOptions, PMTilesError, PMTilesReader, TileType};
use wasm_bindgen_test::{wasm_bindgen_test, wasm_bindgen_test_configure};

wasm_bindgen_test_configure!(run_in_browser);

const FIRENZE: &[u8] = include_bytes!("../testdata/spec/protomaps(vector)ODbL_firenze.pmtiles");
const INVALID: &[u8] = include_bytes!("../testdata/invalid.pmtiles");

fn archive_url(bytes: &[u8]) -> String {
    let parts = js_sys::Array::of1(&js_sys::Uint8Array::from(bytes));
    let blob = web_sys::Blob::new_with_u8_array_sequence(&parts).unwrap();
    web_sys::Url::create_object_url_with_blob(&blob).unwrap()
}

async fn open(bytes: &[u8]) -> PMTilesReader {
    PMTilesReader::new(&archive_url(bytes)).await.unwrap()
}

#[wasm_bindgen_test]
async fn vector_archive_header() {
    let reader = open(FIRENZE).await;

    let header = reader.header();

    assert_eq!((header.min_zoom, header.max_zoom), (0, 15));
    assert_eq!(header.tile_type, TileType::Mvt);
    assert_eq!(header.internal_compression, Compression::Gzip);
    assert_eq!(header.tile_compression, Compression::Gzip);
    assert_eq!(header.number_of_addressed_tiles, 92);
}

#[wasm_bindgen_test]
async fn vector_archive_tiles_are_decompressed() {
    let reader = open(FIRENZE).await;

    let world = reader.get_tile_zxy(0, 0, 0).await.unwrap().unwrap();
    let street = reader
        .get_tile_zxy(15, 17408, 11944)
        .await
        .unwrap()
        .unwrap();

    assert!(!world.starts_with(&[0x1f, 0x8b]));
    assert!(!street.starts_with(&[0x1f, 0x8b]));
    assert_eq!(world[0], 0x1a);
    assert_eq!(street[0], 0x1a);
}

#[wasm_bindgen_test]
async fn vector_archive_missing_tiles_are_none() {
    let reader = open(FIRENZE).await;

    let outside_bounds = reader.get_tile_zxy(15, 0, 0).await.unwrap();
    let beyond_max_zoom = reader.get_tile_zxy(16, 34816, 23888).await.unwrap();

    assert_eq!(outside_bounds, None);
    assert_eq!(beyond_max_zoom, None);
}

#[wasm_bindgen_test]
async fn vector_archive_metadata() {
    let reader = open(FIRENZE).await;

    let metadata = reader.fetch_metadata().await.unwrap();

    assert!(metadata.starts_with(r#"{"vector_layers":"#));
}

#[wasm_bindgen_test]
async fn invalid_archive_fails_to_open() {
    let result = PMTilesReader::new(&archive_url(INVALID)).await;

    assert!(matches!(result, Err(PMTilesError::HeaderDecodeError(_))));
}

#[wasm_bindgen_test]
async fn invalid_url_fails_to_open() {
    let result = PMTilesReader::new("not a url").await;

    assert!(matches!(result, Err(PMTilesError::UrlParseError(_))));
}
#[wasm_bindgen_test]
async fn opens_with_custom_options() {
    let mut options = FetchOptions::new();
    options.set_header("X-Custom", "value").unwrap();

    let reader = PMTilesReader::with_options(&archive_url(FIRENZE), options)
        .await
        .unwrap();

    assert!(reader.get_tile_zxy(0, 0, 0).await.unwrap().is_some());
}
