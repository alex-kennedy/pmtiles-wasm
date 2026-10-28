# PMTiles WebAssembly

[![Tests](https://github.com/alex-kennedy/pmtiles-wasm/actions/workflows/test.yml/badge.svg?branch=main)](https://github.com/alex-kennedy/pmtiles-wasm/actions/workflows/test.yml)
[![Crates.io](https://img.shields.io/crates/v/pmtiles-wasm.svg)](https://crates.io/crates/pmtiles-wasm)

A Rust client library that compiles to WebAssembly for reading 
[PMTiles](https://pmtiles.io) v3 archives over HTTP range requests.

## Example

```rust
use pmtiles_wasm::{PMTilesReader, PMTilesResult};

async fn load_tile() -> PMTilesResult<Option<Vec<u8>>> {
    let reader = PMTilesReader::new("https://example.com/tiles.pmtiles").await?;

    reader.get_tile_zxy(3, 4, 2).await
}
```

## Cargo features

| Feature  | Default | Description                                                  |
| -------- | ------- | ------------------------------------------------------------ |
| `gzip`   | yes     | gzip decompression through the host's `DecompressionStream`. |
| `brotli` | no      | Brotli decompression, compiled into the module.              |
| `zstd`   | no      | zstd decompression, compiled into the module.                |

Archives that use a compression whose feature is disabled fail with an 
unsupported compression error.
