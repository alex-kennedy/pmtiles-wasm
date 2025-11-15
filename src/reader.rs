use bytes::Buf;
use futures_util::future::{FutureExt, LocalBoxFuture, Shared};
use reqwest::Url;
use std::cell::RefCell;
use std::io::Cursor;
use std::num::NonZeroUsize;
use std::rc::Rc;
use wasm_bindgen::prelude::*;

use crate::TileCoordinate;
use crate::cache::DirectoryCache;
use crate::compression::{Compression, decompress};
use crate::directory::{Directory, DirectoryEntry};
use crate::error::{PMTilesError, PMTilesResult};
use crate::fetch_source::FetchSource;
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
    async fn load(source: Rc<FetchSource>) -> PMTilesResult<Archive> {
        let (header_bytes, etag) = source.fetch(0, HEADER_AND_ROOT_MAX_SIZE, None).await?;
        if header_bytes.len() < 127 {
            return Err(PMTilesError::HeaderDecodeError(format!(
                "header too short, got {} bytes, want 127 bytes",
                header_bytes.len()
            )));
        }

        let header = Header::decode(header_bytes[0..127].try_into().unwrap())?;
        let root = decode_root_directory(&header, &header_bytes)?;
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
    source: Rc<FetchSource>,
    archive: RefCell<Rc<Archive>>,
    reloading: RefCell<Option<SharedArchive>>,
}

#[wasm_bindgen]
impl PMTilesReader {
    pub async fn new(url: &str) -> PMTilesResult<Self> {
        let source = Rc::new(FetchSource::new(Url::parse(url)?));
        let archive = Archive::load(source.clone()).await?;
        Ok(Self {
            source,
            archive: RefCell::new(Rc::new(archive)),
            reloading: RefCell::new(None),
        })
    }

    #[wasm_bindgen(getter)]
    pub fn header(&self) -> Header {
        self.archive().header
    }

    /// Returns the tile's (decompressed) bytes, or None if the archive does not contain the tile.
    pub async fn get_tile(&self, tile_id: TileID) -> PMTilesResult<Option<Vec<u8>>> {
        self.with_reload(async |archive| self.tile(archive, tile_id).await)
            .await
    }

    /// Returns the tile's (decompressed) bytes, or None if the archive does not contain the tile.
    pub async fn get_tile_zxy(&self, z: u8, x: u32, y: u32) -> PMTilesResult<Option<Vec<u8>>> {
        let tile_id = TileCoordinate::new(z, x, y)?;
        let header = self.header();
        if z < header.min_zoom || z > header.max_zoom {
            return Ok(None);
        }
        self.get_tile(tile_id.into()).await
    }

    pub async fn fetch_metadata_raw(&self) -> PMTilesResult<Vec<u8>> {
        self.with_reload(async |archive| self.metadata_bytes(archive).await)
            .await
    }

    pub async fn fetch_metadata(&self) -> PMTilesResult<String> {
        self.with_reload(async |archive| {
            let bytes = self.metadata_bytes(archive).await?;
            let decompressed = decompress(Cursor::new(bytes), archive.header.internal_compression)?;
            Ok(String::from_utf8(decompressed)?)
        })
        .await
    }
}

impl PMTilesReader {
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

        // As with the directory cache, no RefCell borrow may be held across an await.
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

        // The first waiter to finish installs the new archive, or clears a failed reload so that a
        // later request can try again.
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
        decompress(bytes.reader(), archive.header.tile_compression)
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
async fn fetch_directory(
    source: Rc<FetchSource>,
    offset: u64,
    length: u64,
    compression: Compression,
    etag: Option<String>,
) -> PMTilesResult<Directory> {
    let (bytes, _) = source.fetch(offset, length, etag.as_deref()).await?;
    let decompressed_bytes = decompress(bytes.reader(), compression)?;
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

fn decode_root_directory(header: &Header, bytes: &[u8]) -> PMTilesResult<Directory> {
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
    let bytes = decompress(bytes, header.internal_compression)?;
    Directory::try_from(bytes.as_slice())
}
