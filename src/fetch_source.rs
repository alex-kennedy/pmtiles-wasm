use std::cell::Cell;

use bytes::Bytes;
use reqwest::header::HeaderValue;
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

        #[cfg(target_arch = "wasm32")]
        if self.must_reload.get() {
            request = request.fetch_cache_reload();
        }

        let response = request.send().await?;
        let new_etag = response.headers().get(header::ETAG).and_then(strong_etag);
        if content_changed(offset, response.status(), etag, new_etag.as_deref()) {
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

/// Weak ETags do not guarantee content is unchanged, so should not be used as a key.
fn strong_etag(value: &HeaderValue) -> Option<String> {
    value
        .to_str()
        .ok()
        .filter(|etag| !etag.starts_with("W/"))
        .map(|etag| etag.to_string())
}

fn content_changed(
    offset: u64,
    status: StatusCode,
    etag: Option<&str>,
    new_etag: Option<&str>,
) -> bool {
    let etag_changed = match (etag, new_etag) {
        (Some(old), Some(new)) => old != new,
        _ => false,
    };
    let range_gone = offset > 0 && status == StatusCode::RANGE_NOT_SATISFIABLE;
    etag_changed || range_gone
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::wasm_bindgen_test;

    #[wasm_bindgen_test(unsupported = test)]
    fn strong_etag_is_kept() {
        let value = HeaderValue::from_static("\"abc\"");

        let etag = strong_etag(&value);

        assert_eq!(etag.as_deref(), Some("\"abc\""));
    }

    #[wasm_bindgen_test(unsupported = test)]
    fn weak_etag_is_ignored() {
        let value = HeaderValue::from_static("W/weak_etag");

        let etag = strong_etag(&value);

        assert_eq!(etag, None);
    }

    #[wasm_bindgen_test(unsupported = test)]
    fn changed_etag_is_content_changed() {
        let changed = content_changed(
            100,
            StatusCode::PARTIAL_CONTENT,
            Some("etag_1"),
            Some("etag_2"),
        );

        assert!(changed);
    }

    #[wasm_bindgen_test(unsupported = test)]
    fn same_etag_is_not_content_changed() {
        let changed = content_changed(
            100,
            StatusCode::PARTIAL_CONTENT,
            Some("etag_1"),
            Some("etag_1"),
        );

        assert!(!changed);
    }

    #[wasm_bindgen_test(unsupported = test)]
    fn missing_etag_is_not_content_changed() {
        let unknown_before =
            content_changed(100, StatusCode::PARTIAL_CONTENT, None, Some("etag_2"));
        let unknown_after = content_changed(100, StatusCode::PARTIAL_CONTENT, Some("etag_1"), None);

        assert!(!unknown_before);
        assert!(!unknown_after);
    }

    #[wasm_bindgen_test(unsupported = test)]
    fn range_not_satisfiable_is_content_changed() {
        let changed = content_changed(100, StatusCode::RANGE_NOT_SATISFIABLE, None, None);

        assert!(changed);
    }

    #[wasm_bindgen_test(unsupported = test)]
    fn range_not_satisfiable_at_start_is_not_content_changed() {
        let changed = content_changed(0, StatusCode::RANGE_NOT_SATISFIABLE, None, None);

        assert!(!changed);
    }
}
