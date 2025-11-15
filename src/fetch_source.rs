use std::cell::Cell;

use bytes::Bytes;
use reqwest::{Client, StatusCode, Url, header};

use crate::{PMTilesError, PMTilesResult, Source};

pub struct FetchSource {
    client: Client,
    url: Url,
    /// Set once the archive has been seen to change. From then on, requests bypass the browser's
    /// HTTP cache, which may still hold ranges of the old archive.
    must_reload: Cell<bool>,
}

impl Source for FetchSource {
    async fn fetch(
        &self,
        offset: u64,
        length: u64,
        etag: Option<&str>,
    ) -> PMTilesResult<(Bytes, Option<String>)> {
        if length == 0 {
            return Ok((Bytes::new(), None));
        }

        let range = format!("bytes={}-{}", offset, offset + length - 1);
        #[allow(unused_mut)]
        let mut request = self
            .client
            .get(self.url.clone())
            .header(header::RANGE, range);

        // `fetch_cache_reload` only exists in reqwest's wasm backend. This gate is just so that
        // `cargo test` builds natively.
        #[cfg(target_arch = "wasm32")]
        if self.must_reload.get() {
            request = request.fetch_cache_reload();
        }

        let response = request.send().await?;
        let new_etag = response
            .headers()
            .get(header::ETAG)
            .and_then(|etag| etag.to_str().ok())
            // Weak ETags do not guarantee content is unchanged, so should not be used as a key.
            .filter(|t| !t.starts_with("W/"))
            .map(|etag| etag.to_string());

        let etag_changed = match (etag, new_etag.as_deref()) {
            (Some(old), Some(new)) => old != new,
            _ => false,
        };
        let range_gone = offset > 0 && response.status() == StatusCode::RANGE_NOT_SATISFIABLE;
        if etag_changed || range_gone {
            self.must_reload.set(true);
            return Err(PMTilesError::ContentChangedError);
        }

        let response = response.error_for_status()?;
        if response.status() != StatusCode::PARTIAL_CONTENT {
            return Err(PMTilesError::UnsupportedStatusCode(response.status()));
        }

        let bytes = response.bytes().await?;
        Ok((bytes, new_etag))
    }
}

impl FetchSource {
    pub fn new(url: Url) -> Self {
        Self {
            client: reqwest::Client::new(),
            url: url,
            must_reload: Cell::new(false),
        }
    }
}
