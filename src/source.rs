use bytes::Bytes;

use crate::PMTilesResult;

pub trait Source {
    /// Fetches `length` bytes starting at `offset`, returning them along with the archive's ETag,
    /// if it has one.
    ///
    /// `etag` is the ETag the caller last saw for the archive. If the archive has since changed,
    /// this fails with `PMTilesError::ContentChangedError`.
    fn fetch(
        &self,
        offset: u64,
        length: u64,
        etag: Option<&str>,
    ) -> impl std::future::Future<Output = PMTilesResult<(Bytes, Option<String>)>>;
}
