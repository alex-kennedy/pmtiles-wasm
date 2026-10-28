# pmtiles-wasm

A WebAssembly client for reading [PMTiles](https://pmtiles.io) v3 archives over HTTP range requests.
It runs anywhere that provides `fetch` and `DecompressionStream`: browsers, Cloudflare Workers, Deno
and Node 18+.

## Building

```sh
wasm-pack build --target web
```

This writes the package to `pkg/`. Import it and call the default export once to load the module:

```js
import init, { PMTilesReader } from "./pkg/pmtiles_wasm.js";

await init();
```

The examples below assume this setup.

## Reading a tile

```js
const reader = await PMTilesReader.new("https://example.com/tiles.pmtiles");

const tile = await reader.get_tile_zxy(3, 4, 2);
if (tile !== undefined) {
  console.log(tile.byteLength);
}
```

`get_tile_zxy` resolves to the tile's decompressed bytes, or `undefined` if the archive does not
contain that tile. Errors, such as an invalid URL or a failed request, reject the promise.

## Tile IDs and coordinates

Tiles can also be addressed by their PMTiles tile ID, which is a `bigint`:

```js
import { TileCoordinate, TileID } from "./pkg/pmtiles_wasm.js";

const tileId = TileID.new(42n);
const tile = await reader.get_tile(tileId);

const coordinate = TileCoordinate.from_tile_id(tileId);
console.log(coordinate.z(), coordinate.x(), coordinate.y());
```

`TileID.new` and `TileCoordinate.new` throw if the value is out of range.

## Header and metadata

```js
import { TileType } from "./pkg/pmtiles_wasm.js";

const header = reader.header;
console.log(header.min_zoom, header.max_zoom);
console.log(header.center_position.lat, header.center_position.lng);
console.log(header.tile_type === TileType.Mvt);

const metadata = JSON.parse(await reader.fetch_metadata());
console.log(metadata.name);
```

`fetch_metadata_raw` returns the decompressed metadata as bytes if you want to parse it yourself.

## Authentication

Custom headers and credentials are set with `FetchOptions` and sent with every request:

```js
import { Credentials, FetchOptions, PMTilesReader } from "./pkg/pmtiles_wasm.js";

const options = new FetchOptions();
options.set_header("Authorization", "Bearer <token>");
options.set_credentials(Credentials.Include);

const reader = await PMTilesReader.with_options("https://example.com/tiles.pmtiles", options);
```

To rotate a token on an open reader, replace the options. Later requests use the new ones:

```js
const refreshed = new FetchOptions();
refreshed.set_header("Authorization", `Bearer ${newToken}`);

reader.set_fetch_options(refreshed);
```

The `Range` header is managed by the reader and cannot be set.

## Archives that change

If the archive is replaced on the server, the reader notices through the `ETag` header, reloads the
header and root directory, and retries the request once.

## Server requirements

The server must support HTTP range requests. For cross-origin archives it should also expose the
`ETag` header through CORS (`Access-Control-Expose-Headers: ETag`), otherwise change detection is
disabled.
Servers that reply `200` to a range request are accepted only when the response is no longer than
the requested range.

## Cargo features

| Feature  | Default | Description                                                           |
| -------- | ------- | --------------------------------------------------------------------- |
| `gzip`   | yes     | gzip decompression through the host's `DecompressionStream`.          |
| `brotli` | no      | Brotli decompression, compiled into the module.                       |
| `zstd`   | no      | zstd decompression, compiled into the module.                         |

Archives that use a compression whose feature is disabled fail with an unsupported compression
error.

```sh
wasm-pack build --target web -- --features brotli,zstd
```

## License

MIT
