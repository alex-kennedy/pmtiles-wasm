use std::cell::RefCell;
use std::future::Future;
use std::num::NonZeroUsize;
use std::rc::Rc;

use futures_util::future::{FutureExt, LocalBoxFuture, Shared};
use lru::LruCache;

use crate::directory::Directory;
use crate::error::PMTilesResult;

/// A directory fetch that may still be in flight, i.e. the equivalent of a JS
/// `Promise<Directory>`. Cloning it gives another handle to the same fetch, and every handle can be
/// awaited independently. The fetch itself only runs once.
type SharedDirectory = Shared<LocalBoxFuture<'static, PMTilesResult<Rc<Directory>>>>;

/// An LRU cache of directories, keyed by their absolute `(offset, length)` in the archive, which
/// also deduplicates concurrent fetches: if several tile requests miss on the same directory at
/// once, only one fetch is made and all of them await it.
///
/// Failed fetches are not cached, so a transient network error does not poison a directory.
pub(crate) struct DirectoryCache {
    entries: RefCell<LruCache<(u64, u64), SharedDirectory>>,
}

impl DirectoryCache {
    /// Creates a cache holding at most `capacity` directories, including in-flight fetches.
    pub(crate) fn new(capacity: NonZeroUsize) -> Self {
        Self {
            entries: RefCell::new(LruCache::new(capacity)),
        }
    }

    /// Returns the directory at the given absolute byte range, running `fetch` to obtain it on a
    /// miss. If a fetch for the same range is already in flight, this waits for that fetch instead.
    ///
    /// Futures do nothing until polled, so on a hit `fetch` is simply dropped without running.
    pub(crate) async fn get_or_fetch(
        &self,
        offset: u64,
        length: u64,
        fetch: impl Future<Output = PMTilesResult<Directory>> + 'static,
    ) -> PMTilesResult<Rc<Directory>> {
        let key = (offset, length);

        let cached = self.entries.borrow_mut().get(&key).cloned();
        let shared = match cached {
            Some(shared) => shared,
            None => {
                let shared = fetch
                    .map(|result| result.map(Rc::new))
                    .boxed_local()
                    .shared();
                self.entries.borrow_mut().put(key, shared.clone());
                shared
            }
        };

        let result = shared.clone().await;
        if result.is_err() {
            self.remove_if_same(key, &shared);
        }
        result
    }

    /// Removes the entry for `key`, but only if it is still `shared`. Every waiter on a failed fetch
    /// calls this, and by then the entry may have been evicted and replaced by a newer fetch, which
    /// must be left alone.
    fn remove_if_same(&self, key: (u64, u64), shared: &SharedDirectory) {
        let mut entries = self.entries.borrow_mut();
        if entries
            .peek(&key)
            .is_some_and(|cached| cached.ptr_eq(shared))
        {
            entries.pop(&key);
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::error::PMTilesError;
    use wasm_bindgen_test::wasm_bindgen_test;

    use std::cell::Cell;
    use std::future::poll_fn;
    use std::pin::pin;
    use std::task::{Context, Poll, Waker};

    fn directory() -> Directory {
        Directory::try_from([1u8, 0, 1, 1, 1].as_slice()).unwrap()
    }

    #[wasm_bindgen_test(unsupported = test)]
    fn hit_returns_cached_directory() {
        let cache = DirectoryCache::new(NonZeroUsize::new(10).unwrap());
        let first = cache
            .get_or_fetch(0, 10, async { Ok(directory()) })
            .now_or_never()
            .unwrap()
            .unwrap();

        let second = cache
            .get_or_fetch(0, 10, async { panic!("should not fetch") })
            .now_or_never()
            .unwrap()
            .unwrap();

        assert!(Rc::ptr_eq(&first, &second));
    }

    #[wasm_bindgen_test(unsupported = test)]
    fn concurrent_misses_share_one_fetch() {
        let cache = DirectoryCache::new(NonZeroUsize::new(10).unwrap());
        let mut cx = Context::from_waker(Waker::noop());
        let open = Rc::new(Cell::new(false));
        let gate = open.clone();
        let fetch = async move {
            poll_fn(|_| match gate.get() {
                true => Poll::Ready(()),
                false => Poll::Pending,
            })
            .await;
            Ok(directory())
        };
        let mut first_request = pin!(cache.get_or_fetch(0, 10, fetch));
        let mut second_request =
            pin!(cache.get_or_fetch(0, 10, async { panic!("should not fetch") }));
        let _ = first_request.as_mut().poll(&mut cx);
        let _ = second_request.as_mut().poll(&mut cx);

        open.set(true);
        let first = first_request.as_mut().poll(&mut cx);
        let second = second_request.as_mut().poll(&mut cx);

        assert!(matches!(
            (&first, &second),
            (Poll::Ready(Ok(first)), Poll::Ready(Ok(second))) if Rc::ptr_eq(first, second)
        ));
    }

    #[wasm_bindgen_test(unsupported = test)]
    fn failures_are_not_cached() {
        let cache = DirectoryCache::new(NonZeroUsize::new(10).unwrap());
        let _ = cache
            .get_or_fetch(0, 10, async {
                Err(PMTilesError::DirectoryDecodeError("boom".to_string()))
            })
            .now_or_never();

        let retried = cache
            .get_or_fetch(0, 10, async { Ok(directory()) })
            .now_or_never()
            .unwrap();

        assert!(retried.is_ok());
    }

    #[wasm_bindgen_test(unsupported = test)]
    fn least_recently_used_is_evicted() {
        let cache = DirectoryCache::new(NonZeroUsize::new(2).unwrap());
        let zero = cache
            .get_or_fetch(0, 10, async { Ok(directory()) })
            .now_or_never()
            .unwrap()
            .unwrap();
        let one = cache
            .get_or_fetch(1, 10, async { Ok(directory()) })
            .now_or_never()
            .unwrap()
            .unwrap();
        let _ = cache
            .get_or_fetch(0, 10, async { panic!("should not fetch") })
            .now_or_never();
        let _ = cache
            .get_or_fetch(2, 10, async { Ok(directory()) })
            .now_or_never();

        let zero_again = cache
            .get_or_fetch(0, 10, async { panic!("should not fetch") })
            .now_or_never()
            .unwrap()
            .unwrap();
        let one_again = cache
            .get_or_fetch(1, 10, async { Ok(directory()) })
            .now_or_never()
            .unwrap()
            .unwrap();

        assert!(Rc::ptr_eq(&zero, &zero_again));
        assert!(!Rc::ptr_eq(&one, &one_again));
    }

    #[wasm_bindgen_test(unsupported = test)]
    fn failure_does_not_remove_newer_fetch() {
        let cache = DirectoryCache::new(NonZeroUsize::new(1).unwrap());
        let mut cx = Context::from_waker(Waker::noop());
        let open = Rc::new(Cell::new(false));
        let gate = open.clone();
        let failing_fetch = async move {
            poll_fn(|_| match gate.get() {
                true => Poll::Ready(()),
                false => Poll::Pending,
            })
            .await;
            Err(PMTilesError::DirectoryDecodeError("boom".to_string()))
        };
        let mut failing_request = pin!(cache.get_or_fetch(0, 10, failing_fetch));
        let _ = failing_request.as_mut().poll(&mut cx);
        let _ = cache
            .get_or_fetch(1, 10, async { Ok(directory()) })
            .now_or_never();
        let newer = cache
            .get_or_fetch(0, 10, async { Ok(directory()) })
            .now_or_never()
            .unwrap()
            .unwrap();
        open.set(true);
        let _ = failing_request.as_mut().poll(&mut cx);

        let cached = cache
            .get_or_fetch(0, 10, async { panic!("should not fetch") })
            .now_or_never()
            .unwrap()
            .unwrap();

        assert!(Rc::ptr_eq(&newer, &cached));
    }

    #[wasm_bindgen_test(unsupported = test)]
    fn dropped_request_does_not_cancel_shared_fetch() {
        let cache = DirectoryCache::new(NonZeroUsize::new(10).unwrap());
        let mut cx = Context::from_waker(Waker::noop());
        let open = Rc::new(Cell::new(false));
        let gate = open.clone();
        let fetch = async move {
            poll_fn(|_| match gate.get() {
                true => Poll::Ready(()),
                false => Poll::Pending,
            })
            .await;
            Ok(directory())
        };
        let mut first_request = Box::pin(cache.get_or_fetch(0, 10, fetch));
        let mut second_request =
            pin!(cache.get_or_fetch(0, 10, async { panic!("should not fetch") }));
        let _ = first_request.as_mut().poll(&mut cx);
        let _ = second_request.as_mut().poll(&mut cx);
        drop(first_request);

        open.set(true);
        let second = second_request.as_mut().poll(&mut cx);

        assert!(matches!(second, Poll::Ready(Ok(_))));
    }

    #[wasm_bindgen_test(unsupported = test)]
    fn fetch_survives_all_requests_being_dropped() {
        let cache = DirectoryCache::new(NonZeroUsize::new(10).unwrap());
        let mut cx = Context::from_waker(Waker::noop());
        let open = Rc::new(Cell::new(false));
        let gate = open.clone();
        let fetch = async move {
            poll_fn(|_| match gate.get() {
                true => Poll::Ready(()),
                false => Poll::Pending,
            })
            .await;
            Ok(directory())
        };
        let mut dropped_request = Box::pin(cache.get_or_fetch(0, 10, fetch));
        let _ = dropped_request.as_mut().poll(&mut cx);
        drop(dropped_request);
        open.set(true);

        let later = cache
            .get_or_fetch(0, 10, async { panic!("should not fetch") })
            .now_or_never();

        assert!(matches!(later, Some(Ok(_))));
    }

    #[wasm_bindgen_test(unsupported = test)]
    fn completed_fetch_survives_request_being_dropped() {
        let cache = DirectoryCache::new(NonZeroUsize::new(10).unwrap());
        let first = cache
            .get_or_fetch(0, 10, async { Ok(directory()) })
            .now_or_never()
            .unwrap()
            .unwrap();
        drop(first);

        let second = cache
            .get_or_fetch(0, 10, async { panic!("should not fetch") })
            .now_or_never();

        assert!(matches!(second, Some(Ok(_))));
    }
}
