//! The calculator's exchange rates, engine side: the ECB's daily reference
//! rates fetched through the engine's HTTP client, cached under Compass's
//! cache directory, and refreshed on Refresh Exchange Rates (IPC v21).
//!
//! # On demand, not in the background
//!
//! Upstream loads the rates at start and checks them hourly. Compass fetches
//! only when a currency expression asks: the launcher evaluates, notices the
//! expression wanted a rate ([`compass_core::calculator::take_rates_wanted`]),
//! and sends [`compass_ipc::Request::ExchangeRates`]. That request reads the
//! cache file and, when the rates are missing or a day old, fetches them
//! before answering. Someone who never converts a currency never pays for
//! the cache read, the TLS stack or the timer.
//!
//! The rates come from the ECB rather than the Vicinae API
//! (`compass_core::exchange_rates`).

use std::path::PathBuf;

use compass_core::exchange_rates::{self, ExchangeRates, RateCache};
use compass_ipc::{ErrorKind, ExchangeRateTable, ProtocolError, Request, Response};

/// The largest rates file accepted. The ECB's is under two kilobytes.
const MAX_BYTES: u64 = 256 * 1024;

/// How long a failed on-demand fetch holds off the next one, so typing a
/// currency expression offline does not try the network on every request.
const RETRY_AFTER_FAILURE_SECS: i64 = 15 * 60;

/// Where the rates come from and what is held.
#[derive(Debug, Default)]
pub struct ExchangeRateService {
    /// The rates file's URL; `None` has no source, and every refresh fails.
    url: Option<String>,
    cache: tokio::sync::Mutex<Option<RateCache>>,
    cache_path: Option<PathBuf>,
    /// Held across an on-demand fetch, so requests that arrive together
    /// fetch once.
    fetching: tokio::sync::Mutex<()>,
    /// When the last on-demand fetch failed.
    failed_at: std::sync::Mutex<Option<i64>>,
    /// Whether rates are only fetched when asked to refresh; see
    /// [`exchange_rates::DISABLE_AUTO_REFRESH_ENV`].
    auto_refresh_disabled: bool,
}

/// Seconds since the epoch.
fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX)
        })
}

/// The wire form of `rates`.
#[must_use]
pub fn table(rates: &ExchangeRates) -> ExchangeRateTable {
    ExchangeRateTable {
        date: rates.date.clone(),
        fetched_at: rates.fetched_at,
        rates: rates
            .rates
            .iter()
            .map(|(code, rate)| (code.clone(), rate.to_string()))
            .collect(),
    }
}

impl ExchangeRateService {
    /// The ECB (or [`exchange_rates::URL_ENV`]'s URL), cached under
    /// `$XDG_CACHE_HOME/compass`.
    #[must_use]
    pub fn from_environment() -> Self {
        let mut service = Self::new(
            Some(exchange_rates::source_url(
                std::env::var(exchange_rates::URL_ENV).ok().as_deref(),
            )),
            compass_core::xdg_dirs::cache_home().map(|home| exchange_rates::cache_path(&home)),
        );
        // Anything but the empty string, as the C++'s
        // `isAutoRateRefreshDisabled` (which counts the empty string too).
        service.auto_refresh_disabled =
            compass_xdg::brand::env_var_os(exchange_rates::DISABLE_AUTO_REFRESH_ENV)
                .is_some_and(|value| !value.is_empty());
        service
    }

    /// Rates fetched from `url` and cached in `cache_path`. Nothing is read
    /// until they are first asked for.
    #[must_use]
    pub fn new(url: Option<String>, cache_path: Option<PathBuf>) -> Self {
        Self {
            url,
            cache_path,
            ..Self::default()
        }
    }

    async fn with_cache<T>(&self, f: impl FnOnce(&mut RateCache) -> T) -> T {
        let mut slot = self.cache.lock().await;
        let cache = match &mut *slot {
            Some(cache) => cache,
            None => {
                let path = self.cache_path.clone();
                let loaded = tokio::task::spawn_blocking(move || RateCache::load(path))
                    .await
                    .unwrap_or_default();
                slot.insert(loaded)
            }
        };
        f(cache)
    }

    /// The rates held: fetched, or read back from the cache.
    pub async fn current(&self) -> Option<ExchangeRates> {
        self.with_cache(|cache| cache.current().cloned()).await
    }

    /// The rates, fetched first when they are missing or stale, unless
    /// automatic fetching is off or a fetch failed in the last 15 minutes.
    /// A failed fetch answers with what is held.
    pub async fn wanted(&self) -> Option<ExchangeRates> {
        if self.auto_refresh_disabled || !self.due(now()).await {
            return self.current().await;
        }
        let _one = self.fetching.lock().await;
        // Whoever held the lock may have fetched already.
        if self.due(now()).await {
            match self.refresh().await {
                Ok(rates) => tracing::info!(date = %rates.date, "fetched the exchange rates"),
                Err(error) => {
                    tracing::warn!(%error, "could not fetch the exchange rates");
                    *self
                        .failed_at
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(now());
                }
            }
        }
        self.current().await
    }

    /// Whether an on-demand fetch should run at `now`.
    async fn due(&self, now: i64) -> bool {
        let failed_at = *self
            .failed_at
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if failed_at.is_some_and(|at| now - at < RETRY_AFTER_FAILURE_SECS) {
            return false;
        }
        self.with_cache(|cache| cache.needs_refresh(now)).await
    }

    /// Fetches the rates now, whatever their age. A failure keeps the rates
    /// held.
    ///
    /// # Errors
    ///
    /// Why the rates could not be fetched or read.
    pub async fn refresh(&self) -> Result<ExchangeRates, String> {
        let fetched = match &self.url {
            Some(url) => crate::stores::get(url.clone(), MAX_BYTES).await,
            None => Err("there is no exchange rate source".to_owned()),
        };
        let at = now();
        self.with_cache(move |cache| cache.refresh(move || fetched, at).cloned())
            .await
    }

    /// Answers [`Request::ExchangeRates`] and
    /// [`Request::RefreshExchangeRates`].
    pub async fn answer(&self, request: Request) -> Response {
        match request {
            Request::RefreshExchangeRates => match self.refresh().await {
                Ok(rates) => Response::ExchangeRates {
                    rates: Some(table(&rates)),
                },
                Err(reason) => Response::Error(ProtocolError::new(
                    ErrorKind::Internal,
                    format!("could not refresh the exchange rates: {reason}"),
                )),
            },
            _ => Response::ExchangeRates {
                rates: self.wanted().await.as_ref().map(table),
            },
        }
    }
}
