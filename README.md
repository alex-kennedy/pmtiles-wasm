# PMTiles WebAssembly

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
