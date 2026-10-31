use std::cell::{Cell, RefCell};

use bytes::Bytes;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest::{Client, RequestBuilder, StatusCode, Url, header};
use wasm_bindgen::prelude::*;

use crate::{PMTilesError, PMTilesResult, Source};

/// Whether the browser sends cookies and HTTP auth with each request.
///
/// See <https://developer.mozilla.org/en-US/docs/Web/API/RequestInit#credentials>.
#[wasm_bindgen]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Credentials {
    Omit,
    SameOrigin,
    Include,
}

/// Options for every HTTP request made to a remote archive.
#[wasm_bindgen]
#[derive(Debug, Clone, Default)]
pub struct FetchOptions {
    headers: HeaderMap,
    credentials: Option<Credentials>,
}

#[wasm_bindgen]
impl FetchOptions {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets a header sent with every request, replacing any previous value. The reader sets
    /// `Range` itself, so it cannot be set here.
    pub fn set_header(&mut self, name: &str, value: &str) -> PMTilesResult<()> {
        let name = HeaderName::try_from(name)
            .map_err(|e| PMTilesError::InvalidHeaderError(e.to_string()))?;
        if name == header::RANGE {
            return Err(PMTilesError::InvalidHeaderError(
                "Range is set by the reader".to_string(),
            ));
        }
        let value = HeaderValue::try_from(value)
            .map_err(|e| PMTilesError::InvalidHeaderError(e.to_string()))?;
        self.headers.insert(name, value);
        Ok(())
    }

    /// Sets whether the browser sends cookies and HTTP auth. Defaults to the browser's own default,
    /// `SameOrigin`.
    pub fn set_credentials(&mut self, credentials: Credentials) {
        self.credentials = Some(credentials);
    }
}

pub struct FetchSource {
    client: Client,
    url: Url,
    options: RefCell<FetchOptions>,
    /// Set once the archive has been seen to change. From then on, requests bypass the browser's
    /// HTTP cache, which may still hold ranges of the old archive.
    must_reload: Cell<bool>,
    no_store: bool,
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

        let mut response = self
            .request(offset, length, self.must_reload.get())?
            .send()
            .await?;
        let mut length = length;

        if offset == 0 && response.status() == StatusCode::RANGE_NOT_SATISFIABLE {
            length = unsatisfied_range_length(response.headers().get(header::CONTENT_RANGE))
                .ok_or(PMTilesError::UnsupportedStatusCode(response.status()))?;
            if length == 0 {
                return Ok((Bytes::new(), None));
            }
            response = self.request(0, length, true)?.send().await?;
        }

        let new_etag = response.headers().get(header::ETAG).and_then(strong_etag);
        if content_changed(offset, response.status(), etag, new_etag.as_deref()) {
            self.must_reload.set(true);
            return Err(PMTilesError::ContentChangedError);
        }

        let response = response.error_for_status()?;
        check_byte_serving(response.status(), response.content_length(), length)?;

        let bytes = response.bytes().await?;
        Ok((bytes, new_etag))
    }
}

impl FetchSource {
    pub fn new(url: Url, options: FetchOptions) -> Self {
        Self {
            client: reqwest::Client::new(),
            url: url,
            options: RefCell::new(options),
            must_reload: Cell::new(false),
            no_store: is_chromium(&user_agent()),
        }
    }

    /// Replaces the options used for all subsequent requests.
    pub fn set_options(&self, options: FetchOptions) {
        *self.options.borrow_mut() = options;
    }

    fn request(&self, offset: u64, length: u64, reload: bool) -> PMTilesResult<RequestBuilder> {
        let end = offset.checked_add(length - 1).ok_or_else(|| {
            PMTilesError::InvalidArchiveError(format!(
                "range of {} bytes at offset {} overflows",
                length, offset
            ))
        })?;
        let options = self.options.borrow();
        let request = self
            .client
            .get(self.url.clone())
            .headers(options.headers.clone())
            .header(header::RANGE, format!("bytes={}-{}", offset, end));
        let cache = match (reload, self.no_store) {
            (true, _) => CacheMode::Reload,
            (false, true) => CacheMode::NoStore,
            (false, false) => CacheMode::Default,
        };
        Ok(with_browser_options(request, options.credentials, cache))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CacheMode {
    Default,
    NoStore,
    Reload,
}

#[cfg(target_arch = "wasm32")]
fn user_agent() -> String {
    js_sys::Reflect::get(&js_sys::global(), &"navigator".into())
        .and_then(|navigator| js_sys::Reflect::get(&navigator, &"userAgent".into()))
        .ok()
        .and_then(|user_agent| user_agent.as_string())
        .unwrap_or_default()
}

#[cfg(not(target_arch = "wasm32"))]
fn user_agent() -> String {
    String::new()
}

fn is_chromium(user_agent: &str) -> bool {
    ["Chrome", "Chromium", "Edg", "OPR", "Brave"]
        .iter()
        .any(|name| user_agent.contains(name))
}

#[cfg(target_arch = "wasm32")]
fn with_browser_options(
    request: RequestBuilder,
    credentials: Option<Credentials>,
    cache: CacheMode,
) -> RequestBuilder {
    let request = match credentials {
        Some(Credentials::Omit) => request.fetch_credentials_omit(),
        Some(Credentials::SameOrigin) => request.fetch_credentials_same_origin(),
        Some(Credentials::Include) => request.fetch_credentials_include(),
        None => request,
    };
    match cache {
        CacheMode::Default => request,
        CacheMode::NoStore => request.fetch_cache_no_store(),
        CacheMode::Reload => request.fetch_cache_reload(),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn with_browser_options(
    request: RequestBuilder,
    _credentials: Option<Credentials>,
    _cache: CacheMode,
) -> RequestBuilder {
    request
}

/// Weak ETags do not guarantee content is unchanged, so should not be used as a key.
fn strong_etag(value: &HeaderValue) -> Option<String> {
    value
        .to_str()
        .ok()
        .filter(|etag| !etag.starts_with("W/"))
        .map(|etag| etag.to_string())
}

/// The archive's full length, from the `Content-Range: bytes */<length>` of a 416 response.
fn unsatisfied_range_length(content_range: Option<&HeaderValue>) -> Option<u64> {
    content_range?
        .to_str()
        .ok()?
        .strip_prefix("bytes */")?
        .parse()
        .ok()
}

/// Some servers answer range requests with 200 rather than 206. That is fine as long as they sent
/// only the requested range, rather than the whole file.
fn check_byte_serving(
    status: StatusCode,
    content_length: Option<u64>,
    length: u64,
) -> PMTilesResult<()> {
    match status {
        StatusCode::PARTIAL_CONTENT => Ok(()),
        StatusCode::OK if content_length.is_some_and(|l| l <= length) => Ok(()),
        StatusCode::OK => Err(PMTilesError::ByteServingUnsupported),
        status => Err(PMTilesError::UnsupportedStatusCode(status)),
    }
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

    #[wasm_bindgen_test(unsupported = test)]
    fn unsatisfied_range_gives_archive_length() {
        let value = HeaderValue::from_static("bytes */1234");

        let length = unsatisfied_range_length(Some(&value));

        assert_eq!(length, Some(1234));
    }

    #[wasm_bindgen_test(unsupported = test)]
    fn malformed_unsatisfied_range_gives_no_length() {
        let satisfied = HeaderValue::from_static("bytes 0-99/1234");
        let not_a_number = HeaderValue::from_static("bytes */abc");

        let satisfied_length = unsatisfied_range_length(Some(&satisfied));
        let not_a_number_length = unsatisfied_range_length(Some(&not_a_number));
        let missing_length = unsatisfied_range_length(None);

        assert_eq!(satisfied_length, None);
        assert_eq!(not_a_number_length, None);
        assert_eq!(missing_length, None);
    }

    #[wasm_bindgen_test(unsupported = test)]
    fn partial_content_is_byte_serving() {
        let result = check_byte_serving(StatusCode::PARTIAL_CONTENT, None, 100);

        assert!(result.is_ok());
    }

    #[wasm_bindgen_test(unsupported = test)]
    fn ok_within_requested_length_is_byte_serving() {
        let exact = check_byte_serving(StatusCode::OK, Some(100), 100);
        let shorter = check_byte_serving(StatusCode::OK, Some(50), 100);

        assert!(exact.is_ok());
        assert!(shorter.is_ok());
    }

    #[wasm_bindgen_test(unsupported = test)]
    fn ok_beyond_requested_length_is_not_byte_serving() {
        let result = check_byte_serving(StatusCode::OK, Some(101), 100);

        assert!(matches!(result, Err(PMTilesError::ByteServingUnsupported)));
    }

    #[wasm_bindgen_test(unsupported = test)]
    fn ok_without_content_length_is_not_byte_serving() {
        let result = check_byte_serving(StatusCode::OK, None, 100);

        assert!(matches!(result, Err(PMTilesError::ByteServingUnsupported)));
    }

    #[wasm_bindgen_test(unsupported = test)]
    fn other_success_status_is_unsupported() {
        let result = check_byte_serving(StatusCode::NO_CONTENT, None, 100);

        assert!(matches!(
            result,
            Err(PMTilesError::UnsupportedStatusCode(StatusCode::NO_CONTENT))
        ));
    }

    #[wasm_bindgen_test(unsupported = test)]
    fn chrome_on_windows_is_chromium() {
        let user_agent = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

        assert!(is_chromium(user_agent));
    }

    #[wasm_bindgen_test(unsupported = test)]
    fn chrome_on_mac_is_chromium() {
        let user_agent = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

        assert!(is_chromium(user_agent));
    }

    #[wasm_bindgen_test(unsupported = test)]
    fn firefox_is_not_chromium() {
        let user_agent = "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:121.0) Gecko/20100101 Firefox/121.0";

        assert!(!is_chromium(user_agent));
    }

    #[wasm_bindgen_test(unsupported = test)]
    fn custom_header_is_set() {
        let mut options = FetchOptions::new();

        options.set_header("Authorization", "Bearer token").unwrap();

        assert_eq!(
            options.headers.get(header::AUTHORIZATION).unwrap(),
            "Bearer token"
        );
    }

    #[wasm_bindgen_test(unsupported = test)]
    fn custom_header_replaces_previous_value() {
        let mut options = FetchOptions::new();
        options.set_header("Authorization", "Bearer old").unwrap();

        options.set_header("authorization", "Bearer new").unwrap();

        assert_eq!(
            options
                .headers
                .get_all(header::AUTHORIZATION)
                .iter()
                .collect::<Vec<_>>(),
            vec!["Bearer new"]
        );
    }

    #[wasm_bindgen_test(unsupported = test)]
    fn range_header_cannot_be_set() {
        let mut options = FetchOptions::new();

        let result = options.set_header("Range", "bytes=0-1");

        assert!(matches!(result, Err(PMTilesError::InvalidHeaderError(_))));
    }

    #[wasm_bindgen_test(unsupported = test)]
    fn invalid_header_is_rejected() {
        let mut options = FetchOptions::new();

        let bad_name = options.set_header("bad name", "value");
        let bad_value = options.set_header("X-Custom", "bad\nvalue");

        assert!(matches!(bad_name, Err(PMTilesError::InvalidHeaderError(_))));
        assert!(matches!(
            bad_value,
            Err(PMTilesError::InvalidHeaderError(_))
        ));
    }

    fn source(options: FetchOptions) -> FetchSource {
        FetchSource::new(
            Url::parse("https://example.com/tiles.pmtiles").unwrap(),
            options,
        )
    }

    #[wasm_bindgen_test(unsupported = test)]
    fn request_sets_inclusive_range() {
        let source = source(FetchOptions::new());

        let request = source.request(10, 5, false).unwrap().build().unwrap();

        assert_eq!(request.headers().get(header::RANGE).unwrap(), "bytes=10-14");
    }

    #[wasm_bindgen_test(unsupported = test)]
    fn request_sends_custom_headers() {
        let mut options = FetchOptions::new();
        options.set_header("Authorization", "Bearer token").unwrap();
        let source = source(options);

        let request = source.request(0, 1, false).unwrap().build().unwrap();

        assert_eq!(
            request.headers().get(header::AUTHORIZATION).unwrap(),
            "Bearer token"
        );
    }

    #[wasm_bindgen_test(unsupported = test)]
    fn set_options_replaces_custom_headers() {
        let mut old = FetchOptions::new();
        old.set_header("Authorization", "Bearer old").unwrap();
        let source = source(old);
        let mut new = FetchOptions::new();
        new.set_header("X-Custom", "value").unwrap();

        source.set_options(new);

        let request = source.request(0, 1, false).unwrap().build().unwrap();
        assert!(request.headers().get(header::AUTHORIZATION).is_none());
        assert_eq!(request.headers().get("X-Custom").unwrap(), "value");
    }

    #[wasm_bindgen_test(unsupported = test)]
    fn request_range_overflow_is_rejected() {
        let source = source(FetchOptions::new());

        let result = source.request(u64::MAX, 2, false);

        assert!(matches!(result, Err(PMTilesError::InvalidArchiveError(_))));
    }
}
