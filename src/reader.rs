use futures_util::future::{FutureExt, LocalBoxFuture, Shared};
use reqwest::Url;
use std::cell::RefCell;
use std::num::NonZeroUsize;
use std::rc::Rc;
use wasm_bindgen::prelude::*;

use crate::TileCoordinate;
use crate::cache::DirectoryCache;
use crate::compression::{Compression, decompress};
use crate::directory::{Directory, DirectoryEntry};
use crate::error::{PMTilesError, PMTilesResult};
use crate::fetch_source::{FetchOptions, FetchSource};
use crate::header::Header;
use crate::source::Source;
use crate::tile_id::TileID;

/// The header is at the start of the archive, and the root directory MUST be contained in the first
/// 16,384 bytes (16 KiB).
const HEADER_AND_ROOT_MAX_SIZE: u64 = 16_384;

/// The spec recommends directories have only two levels (the root and the leaves), but writers can
/// do whatever they wish. Let's use an arbitrary limit on the number of directories visited, root
/// included, to prevent loops.
const DIRECTORY_DEPTH_LIMIT: usize = 8;

/// The maximum number of directories (root and leaves) to keep in memory. Matches the JS client.
const DIRECTORY_CACHE_CAPACITY: NonZeroUsize = NonZeroUsize::new(100).unwrap();

/// One version of the archive, as identified by its ETag. When the archive changes, the reader
/// replaces this wholesale, and the old version's cached directories go with it.
struct Archive {
    /// PMTiles archive header.
    header: Header,
    /// ETag (hash), if set.
    etag: Option<String>,
    /// Root directory reference.
    root: Rc<Directory>,
    // Directories storage.
    directories: DirectoryCache,
}

impl Archive {
    /// Fetches the header and root directory. Owns its source, so a reload can be shared between
    /// requests.
    async fn load<S: Source>(source: Rc<S>) -> PMTilesResult<Archive> {
        let (header_bytes, etag) = source.fetch(0, HEADER_AND_ROOT_MAX_SIZE, None).await?;
        if header_bytes.len() < 127 {
            return Err(PMTilesError::HeaderDecodeError(format!(
                "header too short, got {} bytes, want 127 bytes",
                header_bytes.len()
            )));
        }

        let header = Header::decode(header_bytes[0..127].try_into().unwrap())?;
        let root = decode_root_directory(&header, &header_bytes).await?;
        Ok(Archive {
            header,
            etag,
            root: Rc::new(root),
            directories: DirectoryCache::new(DIRECTORY_CACHE_CAPACITY),
        })
    }
}

/// An archive that may be reloading.
type SharedArchive = Shared<LocalBoxFuture<'static, PMTilesResult<Rc<Archive>>>>;

/// Reads tiles and metadata from a remote PMTiles archive.
///
/// If the archive changes on the server, requests in flight fail with `ContentChangedError`. Each
/// public method then reloads the header and root directory, and retries once.
#[wasm_bindgen]
pub struct PMTilesReader {
    reader: Reader<FetchSource>,
}

#[wasm_bindgen]
impl PMTilesReader {
    pub async fn new(url: &str) -> PMTilesResult<Self> {
        Self::with_options(url, FetchOptions::default()).await
    }

    /// Opens the archive, sending every request with the given headers and credentials.
    pub async fn with_options(url: &str, options: FetchOptions) -> PMTilesResult<Self> {
        let source = Rc::new(FetchSource::new(Url::parse(url)?, options));
        Ok(Self {
            reader: Reader::new(source).await?,
        })
    }

    /// Replaces the headers and credentials used for all subsequent requests, e.g. to refresh an
    /// auth token.
    pub fn set_fetch_options(&self, options: FetchOptions) {
        self.reader.source.set_options(options);
    }

    #[wasm_bindgen(getter)]
    pub fn header(&self) -> Header {
        self.reader.header()
    }

    /// Returns the tile's (decompressed) bytes, or None if the archive does not contain the tile.
    pub async fn get_tile(&self, tile_id: TileID) -> PMTilesResult<Option<Vec<u8>>> {
        self.reader.get_tile(tile_id).await
    }

    /// Returns the tile's (decompressed) bytes, or None if the archive does not contain the tile.
    pub async fn get_tile_zxy(&self, z: u8, x: u32, y: u32) -> PMTilesResult<Option<Vec<u8>>> {
        self.reader.get_tile_zxy(z, x, y).await
    }

    pub async fn fetch_metadata_raw(&self) -> PMTilesResult<Vec<u8>> {
        self.reader.fetch_metadata_raw().await
    }

    pub async fn fetch_metadata(&self) -> PMTilesResult<String> {
        self.reader.fetch_metadata().await
    }
}

/// The reader proper, generic over where the archive's bytes come from.
struct Reader<S> {
    source: Rc<S>,
    archive: RefCell<Rc<Archive>>,
    reloading: RefCell<Option<SharedArchive>>,
}

impl<S: Source + 'static> Reader<S> {
    async fn new(source: Rc<S>) -> PMTilesResult<Self> {
        let archive = Archive::load(source.clone()).await?;
        Ok(Self {
            source,
            archive: RefCell::new(Rc::new(archive)),
            reloading: RefCell::new(None),
        })
    }

    fn header(&self) -> Header {
        self.archive().header
    }

    async fn get_tile(&self, tile_id: TileID) -> PMTilesResult<Option<Vec<u8>>> {
        self.with_reload(async |archive| self.tile(archive, tile_id).await)
            .await
    }

    async fn get_tile_zxy(&self, z: u8, x: u32, y: u32) -> PMTilesResult<Option<Vec<u8>>> {
        let tile_id = TileCoordinate::new(z, x, y)?;
        let header = self.header();
        if z < header.min_zoom || z > header.max_zoom {
            return Ok(None);
        }
        self.get_tile(tile_id.into()).await
    }

    async fn fetch_metadata_raw(&self) -> PMTilesResult<Vec<u8>> {
        self.with_reload(async |archive| self.metadata_bytes(archive).await)
            .await
    }

    async fn fetch_metadata(&self) -> PMTilesResult<String> {
        self.with_reload(async |archive| {
            let bytes = self.metadata_bytes(archive).await?;
            let decompressed = decompress(&bytes, archive.header.internal_compression).await?;
            Ok(String::from_utf8(decompressed)?)
        })
        .await
    }

    /// The current version of the archive. The `RefCell` borrow ends immediately, so callers can
    /// hold the result across awaits.
    fn archive(&self) -> Rc<Archive> {
        self.archive.borrow().clone()
    }

    /// Runs `f` against the current archive. If the archive turns out to have changed, reloads it
    /// and runs `f` once more.
    async fn with_reload<T>(
        &self,
        f: impl AsyncFn(&Archive) -> PMTilesResult<T>,
    ) -> PMTilesResult<T> {
        let archive = self.archive();
        match f(&archive).await {
            Err(PMTilesError::ContentChangedError) => {
                let archive = self.reload(&archive).await?;
                f(&archive).await
            }
            result => result,
        }
    }

    /// Replaces `stale` with a freshly loaded archive. Concurrent callers share one reload, and a
    /// caller arriving after the reload has finished gets its result without reloading again.
    async fn reload(&self, stale: &Rc<Archive>) -> PMTilesResult<Rc<Archive>> {
        let current = self.archive();
        if !Rc::ptr_eq(&current, stale) {
            return Ok(current);
        }

        let pending = self.reloading.borrow().clone();
        let reload = match pending {
            Some(reload) => reload,
            None => {
                let reload = Archive::load(self.source.clone())
                    .map(|result| result.map(Rc::new))
                    .boxed_local()
                    .shared();
                *self.reloading.borrow_mut() = Some(reload.clone());
                reload
            }
        };

        let result = reload.clone().await;

        let mut reloading = self.reloading.borrow_mut();
        if reloading.as_ref().is_some_and(|r| r.ptr_eq(&reload)) {
            *reloading = None;
            if let Ok(archive) = &result {
                *self.archive.borrow_mut() = archive.clone();
            }
        }
        result
    }

    async fn tile(&self, archive: &Archive, tile_id: TileID) -> PMTilesResult<Option<Vec<u8>>> {
        let mut directory = archive.root.clone();
        for _ in 0..DIRECTORY_DEPTH_LIMIT {
            let Some(entry) = directory.get_entry(tile_id) else {
                return Ok(None);
            };
            if !entry.is_leaf_directory() {
                return Ok(Some(self.tile_data(archive, entry).await?));
            }
            let offset = absolute_offset(archive.header.leaf_directories_offset, entry)?;
            directory = self.leaf_directory(archive, offset, entry.length).await?;
        }
        Err(PMTilesError::InvalidArchiveError(format!(
            "tile {} is nested more than {} directories deep",
            tile_id.id(),
            DIRECTORY_DEPTH_LIMIT
        )))
    }

    /// Returns the leaf directory at the given absolute byte range, from the cache if possible.
    async fn leaf_directory(
        &self,
        archive: &Archive,
        offset: u64,
        length: u64,
    ) -> PMTilesResult<Rc<Directory>> {
        let fetch = fetch_directory(
            self.source.clone(),
            offset,
            length,
            archive.header.internal_compression,
            archive.etag.clone(),
        );
        archive
            .directories
            .get_or_fetch(offset, length, fetch)
            .await
    }

    async fn tile_data(
        &self,
        archive: &Archive,
        directory_entry: &DirectoryEntry,
    ) -> PMTilesResult<Vec<u8>> {
        let (bytes, _) = self
            .source
            .fetch(
                absolute_offset(archive.header.tile_data_offset, directory_entry)?,
                directory_entry.length,
                archive.etag.as_deref(),
            )
            .await?;
        decompress(&bytes, archive.header.tile_compression).await
    }

    async fn metadata_bytes(&self, archive: &Archive) -> PMTilesResult<Vec<u8>> {
        let (bytes, _) = self
            .source
            .fetch(
                archive.header.metadata_offset,
                archive.header.metadata_length,
                archive.etag.as_deref(),
            )
            .await?;
        Ok(bytes.to_vec())
    }
}

/// Fetches and decodes the directory at the given absolute byte range. Owns all of its inputs, so
/// the cache can hold the fetch beyond the lifetime of any one request.
async fn fetch_directory<S: Source>(
    source: Rc<S>,
    offset: u64,
    length: u64,
    compression: Compression,
    etag: Option<String>,
) -> PMTilesResult<Directory> {
    let (bytes, _) = source.fetch(offset, length, etag.as_deref()).await?;
    let decompressed_bytes = decompress(&bytes, compression).await?;
    Directory::try_from(decompressed_bytes.as_slice())
}

/// Converts a directory entry's offset, which is relative to the start of its section, into an
/// absolute offset in the archive.
fn absolute_offset(section_offset: u64, directory_entry: &DirectoryEntry) -> PMTilesResult<u64> {
    section_offset
        .checked_add(directory_entry.offset)
        .ok_or_else(|| {
            PMTilesError::InvalidArchiveError(format!(
                "entry offset {} overflows section offset {}",
                directory_entry.offset, section_offset
            ))
        })
}

async fn decode_root_directory(header: &Header, bytes: &[u8]) -> PMTilesResult<Directory> {
    let start = usize::try_from(header.root_directory_offset).map_err(|e| {
        PMTilesError::InvalidArchiveError(format!("root_directory_offset too large: {}", e))
    })?;
    let length = usize::try_from(header.root_directory_length).map_err(|e| {
        PMTilesError::InvalidArchiveError(format!("root_directory_length too large: {}", e))
    })?;
    let bytes = start
        .checked_add(length)
        .and_then(|end| bytes.get(start..end))
        .ok_or_else(|| {
            PMTilesError::InvalidArchiveError("root directory out of bounds".to_string())
        })?;
    let bytes = decompress(bytes, header.internal_compression).await?;
    Directory::try_from(bytes.as_slice())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use integer_encoding::VarIntWriter;
    use std::cell::Cell;
    use wasm_bindgen_test::wasm_bindgen_test;

    struct MemorySource {
        bytes: Vec<u8>,
        etag: RefCell<Option<String>>,
        requests: Cell<usize>,
    }

    impl MemorySource {
        fn new(bytes: &[u8], etag: Option<&str>) -> Rc<Self> {
            Rc::new(Self {
                bytes: bytes.to_vec(),
                etag: RefCell::new(etag.map(str::to_string)),
                requests: Cell::new(0),
            })
        }
    }

    impl Source for MemorySource {
        async fn fetch(
            &self,
            offset: u64,
            length: u64,
            etag: Option<&str>,
        ) -> PMTilesResult<(Bytes, Option<String>)> {
            self.requests.set(self.requests.get() + 1);
            let current = self.etag.borrow().clone();
            if let (Some(old), Some(new)) = (etag, current.as_deref()) {
                if old != new {
                    return Err(PMTilesError::ContentChangedError);
                }
            }
            let start = (offset as usize).min(self.bytes.len());
            let end = ((offset + length) as usize).min(self.bytes.len());
            Ok((Bytes::copy_from_slice(&self.bytes[start..end]), current))
        }
    }

    fn serialize_directory(entries: &[DirectoryEntry]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.write_varint(entries.len()).unwrap();
        let mut last_tile_id = 0;
        for entry in entries {
            bytes
                .write_varint(entry.tile_id.id() - last_tile_id)
                .unwrap();
            last_tile_id = entry.tile_id.id();
        }
        for entry in entries {
            bytes.write_varint(entry.run_length).unwrap();
        }
        for entry in entries {
            bytes.write_varint(entry.length).unwrap();
        }
        for entry in entries {
            bytes.write_varint(entry.offset + 1).unwrap();
        }
        bytes
    }

    #[wasm_bindgen_test]
    async fn loads_header_and_root_directory() {
        let source = MemorySource::new(include_bytes!("../testdata/test_fixture_1.pmtiles"), None);

        let reader = Reader::new(source).await.unwrap();

        let root = reader.archive().root.clone();
        let entry = root.get_entry(TileID::new(0).unwrap()).unwrap();
        assert_eq!(reader.header().root_directory_offset, 127);
        assert_eq!(entry.offset, 0);
        assert_eq!(entry.length, 69);
        assert_eq!(entry.run_length, 1);
    }

    #[wasm_bindgen_test(unsupported = futures_test::test)]
    async fn empty_archive_fails_to_load() {
        let source = MemorySource::new(include_bytes!("../testdata/empty.pmtiles"), None);

        let result = Reader::new(source).await;

        assert!(matches!(result, Err(PMTilesError::HeaderDecodeError(_))));
    }

    #[wasm_bindgen_test(unsupported = futures_test::test)]
    async fn invalid_magic_number_fails_to_load() {
        let source = MemorySource::new(include_bytes!("../testdata/invalid.pmtiles"), None);

        let result = Reader::new(source).await;

        assert!(matches!(result, Err(PMTilesError::HeaderDecodeError(_))));
    }

    #[wasm_bindgen_test(unsupported = futures_test::test)]
    async fn future_spec_version_fails_to_load() {
        let source = MemorySource::new(include_bytes!("../testdata/invalid_v4.pmtiles"), None);

        let result = Reader::new(source).await;

        assert!(matches!(result, Err(PMTilesError::HeaderDecodeError(_))));
    }

    #[wasm_bindgen_test]
    async fn fetches_metadata() {
        let source = MemorySource::new(include_bytes!("../testdata/test_fixture_1.pmtiles"), None);
        let reader = Reader::new(source).await.unwrap();

        let metadata = reader.fetch_metadata().await.unwrap();

        assert!(metadata.contains(r#""name": "test_fixture_1.pmtiles""#));
    }

    #[wasm_bindgen_test]
    async fn fetches_tile() {
        let source = MemorySource::new(include_bytes!("../testdata/test_fixture_1.pmtiles"), None);
        let reader = Reader::new(source).await.unwrap();

        let tile = reader.get_tile_zxy(0, 0, 0).await.unwrap();

        assert!(tile.unwrap().starts_with(&[0x1a, 0x2f, 0x78, 0x02]));
    }

    #[wasm_bindgen_test]
    async fn tile_outside_zoom_range_is_none() {
        let source = MemorySource::new(include_bytes!("../testdata/test_fixture_1.pmtiles"), None);
        let reader = Reader::new(source.clone()).await.unwrap();

        let tile = reader.get_tile_zxy(1, 0, 0).await.unwrap();

        assert_eq!(tile, None);
        assert_eq!(source.requests.get(), 1);
    }

    #[wasm_bindgen_test]
    async fn etag_change_reloads_archive() {
        let source = MemorySource::new(
            include_bytes!("../testdata/test_fixture_1.pmtiles"),
            Some("etag_1"),
        );
        let reader = Reader::new(source.clone()).await.unwrap();
        reader.get_tile_zxy(0, 0, 0).await.unwrap();
        let requests_before_change = source.requests.get();
        *source.etag.borrow_mut() = Some("etag_2".to_string());

        let tile = reader.get_tile_zxy(0, 0, 0).await.unwrap();

        assert!(tile.is_some());
        assert_eq!(requests_before_change, 2);
        assert_eq!(source.requests.get(), 5);
        assert_eq!(reader.archive().etag.as_deref(), Some("etag_2"));
    }

    #[wasm_bindgen_test]
    async fn unchanged_etag_does_not_reload_archive() {
        let source = MemorySource::new(
            include_bytes!("../testdata/test_fixture_1.pmtiles"),
            Some("etag_1"),
        );
        let reader = Reader::new(source.clone()).await.unwrap();
        reader.get_tile_zxy(0, 0, 0).await.unwrap();

        let tile = reader.get_tile_zxy(0, 0, 0).await.unwrap();

        assert!(tile.is_some());
        assert_eq!(source.requests.get(), 3);
    }

    #[wasm_bindgen_test(unsupported = futures_test::test)]
    async fn fetches_tiles_through_leaf_directories() {
        let entries: Vec<DirectoryEntry> = (0..100_000)
            .step_by(2)
            .map(|i| DirectoryEntry {
                tile_id: TileID::new(i).unwrap(),
                offset: i,
                length: 1,
                run_length: 2,
            })
            .collect();
        let mut root_entries = Vec::new();
        let mut leaves = Vec::new();
        for chunk in entries.chunks(1000) {
            let leaf = serialize_directory(chunk);
            root_entries.push(DirectoryEntry {
                tile_id: chunk[0].tile_id,
                offset: leaves.len() as u64,
                length: leaf.len() as u64,
                run_length: 0,
            });
            leaves.extend_from_slice(&leaf);
        }
        let root = serialize_directory(&root_entries);
        let tile_data: Vec<u8> = (0..100_000).map(|i| (i % 251) as u8).collect();
        let root_offset = 127u64;
        let leaves_offset = root_offset + root.len() as u64;
        let tile_data_offset = leaves_offset + leaves.len() as u64;
        let mut archive = Vec::new();
        archive.extend_from_slice(b"PMTiles");
        archive.push(3);
        for value in [
            root_offset,
            root.len() as u64,
            root_offset,
            0,
            leaves_offset,
            leaves.len() as u64,
            tile_data_offset,
            tile_data.len() as u64,
            100_000,
            50_000,
            50_000,
        ] {
            archive.extend_from_slice(&value.to_le_bytes());
        }
        archive.extend_from_slice(&[0, 1, 1, 1, 0, 10]);
        archive.extend_from_slice(&[0; 25]);
        archive.extend_from_slice(&root);
        archive.extend_from_slice(&leaves);
        archive.extend_from_slice(&tile_data);
        let source = MemorySource::new(&archive, None);
        let reader = Reader::new(source).await.unwrap();

        let mut tiles = Vec::new();
        for i in (0..100_000).step_by(31) {
            let coordinate = TileCoordinate::from_tile_id(TileID::new(i).unwrap());
            let tile = reader
                .get_tile_zxy(coordinate.z(), coordinate.x(), coordinate.y())
                .await
                .unwrap();
            tiles.push((i, tile));
        }

        assert!(root.len() as u64 <= HEADER_AND_ROOT_MAX_SIZE - root_offset);
        for (i, tile) in tiles {
            assert_eq!(tile, Some(vec![((i - i % 2) % 251) as u8]), "tile {}", i);
        }
    }
}
