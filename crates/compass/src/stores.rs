//! The extension stores, engine side: fetching the listings, building the
//! launcher's rows and detail pages, installing and uninstalling, and
//! installing what `extensions.installed` declares ([`Stores::reconcile`]).
//!
//! Ports the HTTP halves of `VicinaeStoreService` and `RaycastStoreService`
//! and the install and uninstall actions of the two store views. What the
//! listings mean is `compass_core::{extension_store, raycast_store}`; how a
//! bundle lands on disk is `compass_core::store_bundle`; what a declaration
//! means is `compass_core::declared_extensions`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use compass_core::declared_extensions::{self, Step, StoreExtension};
use compass_core::store_bundle::Marker;
use compass_core::store_listing::{Listing, Store};
use compass_core::{extension_store, raycast_store, raycast_store_view, store_bundle};
use compass_ipc::{StoreDetail, StoreEntry, StoreKind};

/// How long one store request may take, connection to last byte.
const TIMEOUT: Duration = Duration::from_secs(60);

/// The largest listing accepted, in bytes. The Vicinae store's whole list
/// and a Raycast page are each well under a megabyte.
const MAX_LISTING_BYTES: u64 = 16 * 1024 * 1024;

/// How long the Vicinae store's list is reused before it is fetched again.
///
/// The C++ asks with `PreferCache`, so a list fetched once is served from
/// its disk cache for as long as that keeps it. A few minutes keeps the
/// browse-then-install round trip off the network without letting a
/// long-running engine show last week's store.
const LIST_TTL: Duration = Duration::from_secs(10 * 60);

/// The platform name the stores filter on, `platform::extensionPlatform`.
#[must_use]
pub const fn platform() -> &'static str {
    match std::env::consts::OS.as_bytes() {
        b"macos" => "macos",
        b"windows" => "windows",
        _ => "linux",
    }
}

const fn store(kind: StoreKind) -> Store {
    match kind {
        StoreKind::Vicinae => Store::Vicinae,
        StoreKind::Raycast => Store::Raycast,
    }
}

const fn kind(store: Store) -> StoreKind {
    match store {
        Store::Vicinae => StoreKind::Vicinae,
        Store::Raycast => StoreKind::Raycast,
    }
}

pub(crate) fn agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        use ureq::tls::{RootCerts, TlsConfig, TlsProvider};
        ureq::Agent::config_builder()
            .timeout_global(Some(TIMEOUT))
            .user_agent(format!("compass/{}", env!("CARGO_PKG_VERSION")))
            .tls_config(
                TlsConfig::builder()
                    .provider(TlsProvider::NativeTls)
                    .root_certs(RootCerts::PlatformVerifier)
                    .build(),
            )
            .build()
            .into()
    })
}

/// GETs `url`, at most `limit` bytes of it. Blocking.
fn get_blocking(url: &str, limit: u64) -> Result<Vec<u8>, String> {
    let mut response = agent().get(url).call().map_err(|err| err.to_string())?;
    response
        .body_mut()
        .with_config()
        .limit(limit)
        .read_to_vec()
        .map_err(|err| err.to_string())
}

/// GETs `url` on the blocking pool.
pub(crate) async fn get(url: String, limit: u64) -> Result<Vec<u8>, String> {
    tokio::task::spawn_blocking(move || get_blocking(&url, limit))
        .await
        .unwrap_or_else(|err| Err(format!("the request failed: {err}")))
}

async fn get_json<T: serde::de::DeserializeOwned>(url: String) -> Result<T, String> {
    let bytes = get(url, MAX_LISTING_BYTES).await?;
    serde_json::from_slice(&bytes).map_err(|err| format!("the store answered unexpectedly: {err}"))
}

/// The stores' state: their addresses and what has been fetched.
#[derive(Debug)]
pub struct Stores {
    vicinae_base: String,
    raycast_base: String,
    platform: &'static str,
    vicinae: tokio::sync::Mutex<Option<(Instant, Arc<Vec<extension_store::Extension>>)>>,
    raycast: tokio::sync::Mutex<raycast_store::RaycastStoreService>,
}

impl Default for Stores {
    fn default() -> Self {
        Self::from_environment()
    }
}

impl Stores {
    /// The real stores, or the ones `COMPASS_VICINAE_API_URL` (formerly
    /// `VICINAE_API_URL`) and `COMPASS_RAYCAST_API_URL` name.
    #[must_use]
    pub fn from_environment() -> Self {
        Self::new(
            compass_core::telemetry::api_base_url(
                compass_xdg::brand::env_var_os_or(
                    compass_core::telemetry::API_URL_ENV,
                    compass_core::telemetry::LEGACY_API_URL_ENV,
                )
                .and_then(|value| value.into_string().ok())
                .as_deref(),
            ),
            raycast_store::api_base_url(std::env::var(raycast_store::API_URL_ENV).ok().as_deref()),
        )
    }

    /// Stores at these base URLs.
    #[must_use]
    pub fn new(vicinae_base: String, raycast_base: String) -> Self {
        Self {
            vicinae_base: vicinae_base.trim_end_matches('/').to_owned(),
            raycast_base: raycast_base.trim_end_matches('/').to_owned(),
            platform: platform(),
            vicinae: tokio::sync::Mutex::default(),
            raycast: tokio::sync::Mutex::default(),
        }
    }

    /// The Vicinae store's whole list, `fetchAll`, post-processed.
    async fn vicinae_all(&self) -> Result<Arc<Vec<extension_store::Extension>>, String> {
        let mut cached = self.vicinae.lock().await;
        if let Some((at, list)) = cached.as_ref()
            && at.elapsed() < LIST_TTL
        {
            return Ok(Arc::clone(list));
        }
        let mut response: extension_store::ListResponse = get_json(format!(
            "{}{}",
            self.vicinae_base,
            extension_store::fetch_all_path()
        ))
        .await?;
        extension_store::post_process(&mut response, self.platform);
        let list = Arc::new(response.extensions);
        *cached = Some((Instant::now(), Arc::clone(&list)));
        Ok(list)
    }

    /// The Raycast store's first page, cached for the session.
    async fn raycast_page(&self) -> Result<Vec<raycast_store::Extension>, String> {
        let options = raycast_store::ListPaginationOptions::default();
        if let Some(page) = self.raycast.lock().await.cached_page(options.page) {
            return Ok(page.to_vec());
        }
        let response: raycast_store::ListApiResponse = get_json(format!(
            "{}{}",
            self.raycast_base,
            raycast_store::list_path(options)
        ))
        .await?;
        Ok(self
            .raycast
            .lock()
            .await
            .store_page(options.page, response.data, self.platform)
            .to_vec())
    }

    async fn raycast_search(&self, query: &str) -> Result<Vec<raycast_store::Extension>, String> {
        let response: raycast_store::ListApiResponse = get_json(format!(
            "{}{}",
            self.raycast_base,
            raycast_store::search_path(query)
        ))
        .await?;
        let mut extensions = response.data;
        raycast_store::post_process(&mut extensions, self.platform);
        Ok(extensions)
    }

    async fn raycast_extension(
        &self,
        owner: &str,
        name: &str,
    ) -> Result<raycast_store::Extension, String> {
        let mut extension: raycast_store::Extension = get_json(format!(
            "{}{}",
            self.raycast_base,
            raycast_store::extension_path(owner, name)
        ))
        .await?;
        raycast_store::post_process_extension(&mut extension);
        Ok(extension)
    }

    /// The compatibility sheet, fetched once where there is one. A failed
    /// fetch is an empty sheet and is tried again next time, as `fetchCompat`.
    async fn raycast_compat(&self) -> raycast_store::CompatMap {
        if !self.raycast.lock().await.should_fetch_compat(self.platform) {
            return self.raycast.lock().await.compat().clone();
        }
        let url = format!("{}{}", self.vicinae_base, raycast_store::COMPAT_PATH);
        match get_json::<raycast_store::CompatMap>(url).await {
            Ok(compat) => {
                self.raycast.lock().await.set_compat(compat.clone());
                compat
            }
            Err(err) => {
                tracing::warn!(%err, "Raycast compatibility sheet not fetched");
                self.raycast.lock().await.compat_fetch_failed()
            }
        }
    }

    /// A store's rows for `query`, with the heading over them.
    ///
    /// # Errors
    ///
    /// The sentence the launcher shows: the C++ toast's, and why.
    pub async fn browse(&self, kind: StoreKind, query: &str) -> Result<Listed, String> {
        let installed = installed().await;
        match kind {
            StoreKind::Vicinae => {
                let all = self.vicinae_all().await.map_err(|err| {
                    format!("{}: {err}", compass_core::store_listing::FETCH_FAILED)
                })?;
                let entries = extension_store::filter(&all, query)
                    .into_iter()
                    .map(|extension| entry(&Listing::from_vicinae(extension), &installed, None))
                    .collect();
                Ok(Listed {
                    heading: raycast_store_view::LIST_HEADING.to_owned(),
                    entries,
                })
            }
            StoreKind::Raycast => {
                let searching = raycast_store_view::text_starts_search(query.trim());
                let (fetched, compat) = if searching {
                    tokio::join!(self.raycast_search(query.trim()), self.raycast_compat())
                } else {
                    tokio::join!(self.raycast_page(), self.raycast_compat())
                };
                let failed = if searching {
                    compass_core::store_listing::SEARCH_FAILED
                } else {
                    compass_core::store_listing::FETCH_FAILED
                };
                let extensions = fetched.map_err(|err| format!("{failed}: {err}"))?;
                let sheet = raycast_store::has_compat_sheet(self.platform);
                let entries = extensions
                    .iter()
                    .map(|extension| {
                        let tier =
                            raycast_store_view::row_compat_tier(sheet, compat.get(&extension.name));
                        entry(
                            &Listing::from_raycast(extension, None),
                            &installed,
                            tier.map(|tier| u8::try_from(tier.as_i32()).unwrap_or(3)),
                        )
                    })
                    .collect();
                Ok(Listed {
                    heading: if searching {
                        raycast_store_view::SEARCH_HEADING
                    } else {
                        raycast_store_view::LIST_HEADING
                    }
                    .to_owned(),
                    entries,
                })
            }
        }
    }

    /// One extension as the store describes it now, by the handle the store
    /// files it under ([`Listing::owner`]) and its name.
    async fn listing(
        &self,
        kind: StoreKind,
        owner: &str,
        name: &str,
    ) -> Result<(Listing, Option<u8>, Option<raycast_store_view::Alert>), String> {
        match kind {
            StoreKind::Vicinae => {
                let all = self.vicinae_all().await.map_err(|err| {
                    format!("Could not fetch extension data from the store: {err}")
                })?;
                let extension = extension_store::find(&all, owner, name).ok_or_else(|| {
                    format!("The extension \"{owner}/{name}\" could not be found in the store.")
                })?;
                Ok((Listing::from_vicinae(extension), None, None))
            }
            StoreKind::Raycast => {
                let (extension, compat) =
                    tokio::join!(self.raycast_extension(owner, name), self.raycast_compat());
                let extension = extension.map_err(|err| {
                    let (_, message) = raycast_store_view::extension_load_failure(owner, name);
                    format!("{message} ({err})")
                })?;
                let updated = (extension.updated_at > 0)
                    .then(|| jiff::Timestamp::from_second(extension.updated_at).ok())
                    .flatten()
                    .map(|at| at.strftime("%Y-%m-%d").to_string());
                let sheet = raycast_store::has_compat_sheet(self.platform);
                let tier = raycast_store_view::row_compat_tier(sheet, compat.get(&extension.name))
                    .map(|tier| u8::try_from(tier.as_i32()).unwrap_or(3));
                let alert = compass_core::store_listing::raycast_alert(
                    &compat,
                    &extension.name,
                    self.platform,
                );
                Ok((Listing::from_raycast(&extension, updated), tier, alert))
            }
        }
    }

    /// One extension's detail page, with its README when it can be fetched.
    ///
    /// # Errors
    ///
    /// The sentence the launcher shows when the store does not have it or
    /// cannot be reached.
    pub async fn detail(
        &self,
        kind: StoreKind,
        owner: &str,
        name: &str,
    ) -> Result<StoreDetail, String> {
        let (listing, tier, alert) = self.listing(kind, owner, name).await?;
        let readme = match &listing.readme_url {
            Some(url) => {
                let url = compass_core::store_listing::readme_source_url(url);
                match get(url.clone(), compass_core::store_listing::MAX_README_BYTES).await {
                    Ok(bytes) => Some(html_images_as_markdown(&absolute_readme_images(
                        &String::from_utf8_lossy(&bytes),
                        &url,
                    ))),
                    Err(err) => {
                        tracing::debug!(%err, "README not fetched");
                        None
                    }
                }
            }
            None => None,
        };
        let installed = installed().await;
        Ok(StoreDetail {
            entry: entry(&listing, &installed, tier),
            markdown: compass_core::store_listing::detail_markdown(
                &listing,
                alert.as_ref(),
                readme.as_deref(),
            ),
            screenshots: listing.screenshots.clone(),
            readme_url: listing.readme_url.clone(),
            source_url: listing.source_url.clone(),
            store_url: listing.store_url.clone(),
        })
    }

    /// Downloads and installs an extension, answering its id and title.
    ///
    /// # Errors
    ///
    /// The sentence the launcher shows: the C++'s "Failed to download
    /// extension" or "Failed to extract extension archive", and why.
    pub async fn install(
        &self,
        kind: StoreKind,
        owner: &str,
        name: &str,
    ) -> Result<(String, String), String> {
        let (listing, _, _) = self.listing(kind, owner, name).await?;
        let id = store(kind).extension_id(&listing.name);
        if !store_bundle::is_safe_id(&id) {
            return Err(format!(
                "\"{id}\" is not an extension id Compass can install"
            ));
        }
        let url = listing.download_url.trim();
        if !(url.starts_with("https://") || url.starts_with("http://")) {
            return Err("Failed to download extension: the store gave no download address".into());
        }
        let archive = get(url.to_owned(), store_bundle::LIMITS.max_archive_bytes)
            .await
            .map_err(|err| format!("Failed to download extension: {err}"))?;
        let directory = compass_core::manifest::registry::local_directory()
            .ok_or("Installing an extension needs a data directory, and $XDG_DATA_HOME and $HOME are unset")?;
        let marker = Marker {
            store: store(kind).key().to_owned(),
            author: listing.owner.clone(),
            name: listing.name.clone(),
            version: listing.version.clone(),
        };
        let installed_id = id.clone();
        tokio::task::spawn_blocking(move || {
            store_bundle::install(
                &directory,
                &installed_id,
                &archive,
                &marker,
                &store_bundle::LIMITS,
            )
        })
        .await
        .map_err(|err| format!("the install task failed: {err}"))?
        .map_err(|err| format!("Failed to extract extension archive: {err}"))?;
        tracing::info!(%id, "extension installed");
        Ok((id, listing.title))
    }

    /// Installs what `entries` (`extensions.installed`) declares and is not
    /// installed, one store install at a time, and says what became of each
    /// entry. Never uninstalls anything; see
    /// [`compass_core::declared_extensions`] for why.
    ///
    /// A `dry_run` says what would be installed without asking the network.
    pub async fn reconcile(&self, entries: &[String], dry_run: bool) -> Vec<Reconciled> {
        let installed = installed().await;
        let steps = declared_extensions::plan(
            entries,
            |id| installed.contains_key(id),
            compass_core::raycast_overrides::Manifest::shipped(),
        );
        let mut reconciled = Vec::with_capacity(steps.len());
        for step in steps {
            let (entry, outcome) = match step {
                Step::Present { entry, id } => (entry, Outcome::Present { id }),
                Step::Unresolvable { entry, reason } => (entry, Outcome::Unresolvable(reason)),
                Step::Install { entry, extension } if dry_run => {
                    (entry, Outcome::WouldInstall { id: extension.id() })
                }
                Step::Install { entry, extension } => {
                    let outcome = match self.install_declared(&extension).await {
                        Ok((id, title)) => Outcome::Installed { id, title },
                        Err(reason) => Outcome::Failed(reason),
                    };
                    (entry, outcome)
                }
            };
            reconciled.push(Reconciled { entry, outcome });
        }
        reconciled
    }

    async fn install_declared(
        &self,
        extension: &StoreExtension,
    ) -> Result<(String, String), String> {
        let kind = kind(extension.store);
        let owner = match &extension.owner {
            Some(owner) => owner.clone(),
            None => self.owner_of(extension.store, &extension.name).await?,
        };
        self.install(kind, &owner, &extension.name).await
    }

    /// The handle the store files the extension `name` under, for an entry
    /// that gave only the id it installs under. Two owners with the same
    /// name would install to the same id, so that is refused rather than
    /// guessed.
    async fn owner_of(&self, store: Store, name: &str) -> Result<String, String> {
        let mut owners: Vec<String> = match store {
            Store::Vicinae => self
                .vicinae_all()
                .await
                .map_err(|err| format!("{}: {err}", compass_core::store_listing::FETCH_FAILED))?
                .iter()
                .filter(|extension| extension.name == name)
                .map(|extension| extension.author.handle.clone())
                .collect(),
            Store::Raycast => self
                .raycast_search(name)
                .await
                .map_err(|err| format!("{}: {err}", compass_core::store_listing::SEARCH_FAILED))?
                .iter()
                .filter(|extension| extension.name == name)
                .map(|extension| extension.owner_handle().to_owned())
                .collect(),
        };
        owners.sort_unstable();
        owners.dedup();
        let key = store.key();
        let store = store.name();
        match <[String; 1]>::try_from(owners) {
            Ok([owner]) => Ok(owner),
            Err(owners) if owners.is_empty() => {
                Err(format!("The {store} store has no extension named {name}."))
            }
            Err(_) => Err(format!(
                "The {store} store has more than one extension named {name}; write it as {key}:<owner>/{name}."
            )),
        }
    }
}

/// What reconciling did with one entry of `extensions.installed`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reconciled {
    /// The entry as written.
    pub entry: String,
    /// What became of it.
    pub outcome: Outcome,
}

/// What became of one declared extension.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// It was installed already, under `id`.
    Present {
        /// The id it is installed under.
        id: String,
    },
    /// It has just been installed.
    Installed {
        /// The id it was installed under.
        id: String,
        /// Its title.
        title: String,
    },
    /// A dry run: it would be installed under `id`.
    WouldInstall {
        /// The id it would be installed under.
        id: String,
    },
    /// Nothing can install it; trying again will not help until the entry
    /// changes.
    Unresolvable(String),
    /// The install failed: the store could not be reached, did not have it,
    /// or served a bundle that was refused. Worth trying again later.
    Failed(String),
}

impl Outcome {
    /// Whether this is a problem to report.
    #[must_use]
    pub const fn is_problem(&self) -> bool {
        matches!(self, Self::Unresolvable(_) | Self::Failed(_))
    }
}

/// A store's rows and their heading.
#[derive(Debug)]
pub struct Listed {
    /// `Extensions` or `Results`.
    pub heading: String,
    /// The rows.
    pub entries: Vec<StoreEntry>,
}

/// What is installed now, by id, with each one's marker when it has one.
async fn installed() -> HashMap<String, Option<Marker>> {
    tokio::task::spawn_blocking(|| {
        compass_core::manifest::registry::scan(&compass_core::manifest::registry::search_paths())
            .extensions
            .into_iter()
            .map(|manifest| (manifest.id, store_bundle::read_marker(&manifest.path)))
            .collect()
    })
    .await
    .unwrap_or_default()
}

fn entry(
    listing: &Listing,
    installed: &HashMap<String, Option<Marker>>,
    compat: Option<u8>,
) -> StoreEntry {
    let marker = installed.get(&listing.id);
    StoreEntry {
        id: listing.id.clone(),
        name: listing.name.clone(),
        author: listing.author.clone(),
        author_name: listing.author_name.clone(),
        title: listing.title.clone(),
        description: listing.description.clone(),
        icon_light: listing.icon_light.clone(),
        icon_dark: listing.icon_dark.clone(),
        downloads: listing.downloads.clone(),
        installed: marker.is_some(),
        update_available: store_bundle::update_available(
            marker.and_then(Option::as_ref),
            &listing.version,
        ),
        compat,
        author_avatar: listing.author_avatar.clone(),
        owner: listing.owner.clone(),
    }
}

/// A README with its relative image links made absolute against `base`
/// (the URL it was fetched from), so the detail page can fetch them: the
/// destinations of Markdown images, found with `pulldown-cmark`, and the
/// `src` of HTML `<img>` tags. Absolute links, and anything `base` cannot
/// resolve, are left as they are.
#[must_use]
pub fn absolute_readme_images(readme: &str, base: &str) -> String {
    use pulldown_cmark::{Event, Parser, Tag};
    let Ok(base) = url::Url::parse(base) else {
        return readme.to_owned();
    };
    let resolve = |link: &str| -> Option<String> {
        if link.is_empty() || url::Url::parse(link).is_ok() || link.starts_with('#') {
            return None;
        }
        base.join(link).ok().map(|url| url.to_string())
    };
    // (start, end, replacement) for each relative destination.
    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    for (event, range) in Parser::new(readme).into_offset_iter() {
        if let Event::Start(Tag::Image { dest_url, .. }) = event
            && let Some(absolute) = resolve(&dest_url)
            && let Some(at) = readme[range.clone()].rfind(dest_url.as_ref())
        {
            let start = range.start + at;
            edits.push((start, start + dest_url.len(), absolute));
        }
    }
    let mut search = 0;
    while let Some(found) = readme[search..].find("<img") {
        let tag_start = search + found;
        let tag_end = readme[tag_start..]
            .find('>')
            .map_or(readme.len(), |end| tag_start + end);
        let tag = &readme[tag_start..tag_end];
        for quote in ['"', '\''] {
            let key = format!("src={quote}");
            if let Some(at) = tag.find(&key) {
                let value_start = tag_start + at + key.len();
                if let Some(len) = readme[value_start..tag_end].find(quote)
                    && let Some(absolute) = resolve(&readme[value_start..value_start + len])
                {
                    edits.push((value_start, value_start + len, absolute));
                }
                break;
            }
        }
        search = tag_end.max(tag_start + 4);
    }
    edits.sort_by_key(|edit| edit.0);
    edits.dedup_by_key(|edit| edit.0);
    let mut out = String::with_capacity(readme.len());
    let mut cursor = 0;
    for (start, end, replacement) in edits {
        if start < cursor {
            continue;
        }
        out.push_str(&readme[cursor..start]);
        out.push_str(&replacement);
        cursor = end;
    }
    out.push_str(&readme[cursor..]);
    out
}

/// A Markdown document's `<img>` tags turned into Markdown images, so the
/// launcher's renderer, which draws Markdown and not HTML, shows them.
#[must_use]
pub fn html_images_as_markdown(readme: &str) -> String {
    let mut out = String::with_capacity(readme.len());
    let mut rest = readme;
    while let Some(found) = rest.find("<img") {
        out.push_str(&rest[..found]);
        let tag = &rest[found..];
        let end = tag.find('>').map_or(tag.len(), |end| end + 1);
        let attribute = |name: &str| {
            ['"', '\''].iter().find_map(|quote| {
                let key = format!("{name}={quote}");
                let at = tag[..end].find(&key)? + key.len();
                let len = tag[at..end].find(*quote)?;
                Some(tag[at..at + len].to_owned())
            })
        };
        match attribute("src") {
            Some(src) => {
                let alt = attribute("alt").unwrap_or_default();
                out.push_str(&format!("\n\n![{alt}]({src})\n\n"));
            }
            None => out.push_str(&tag[..end]),
        }
        rest = &tag[end..];
    }
    out.push_str(rest);
    out
}

/// The installed extension `id`'s directory, if one is installed.
#[must_use]
pub fn installed_directory(id: &str) -> Option<PathBuf> {
    compass_core::manifest::registry::scan(&compass_core::manifest::registry::search_paths())
        .extensions
        .into_iter()
        .find(|manifest| manifest.id == id)
        .map(|manifest| manifest.path)
}

/// Whether `url` is one [`compass_ipc::Request::OpenUrl`] opens: `http` or
/// `https`, with a host.
#[must_use]
pub fn is_openable_url(url: &str) -> bool {
    url::Url::parse(url).is_ok_and(|parsed| {
        matches!(parsed.scheme(), "http" | "https") && parsed.host_str().is_some()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_web_urls_are_opened() {
        assert!(is_openable_url("https://github.com/vicinaehq/extensions"));
        assert!(is_openable_url("http://127.0.0.1:8080/x"));
        assert!(!is_openable_url("file:///etc/passwd"));
        assert!(!is_openable_url("javascript:alert(1)"));
        assert!(!is_openable_url("vicinae://launch/core/store"));
        assert!(!is_openable_url("not a url"));
    }

    #[test]
    fn a_readme_s_relative_images_are_made_absolute_and_html_ones_drawable() {
        let base = "https://raw.githubusercontent.com/o/r/main/extensions/clock/README.md";
        let readme = "# Clock\n\n![shot](media/one.png) and ![abs](https://x.test/a.png)\n\n\
                      See [docs](docs/x.md).\n\n<img src=\"./media/two.png\" alt=\"two\" width=\"300\">\n";
        let fixed = absolute_readme_images(readme, base);
        assert!(
            fixed.contains(
                "![shot](https://raw.githubusercontent.com/o/r/main/extensions/clock/media/one.png)"
            ),
            "{fixed}"
        );
        assert!(
            fixed.contains("![abs](https://x.test/a.png)"),
            "absolute kept"
        );
        assert!(fixed.contains("[docs](docs/x.md)"), "links are not images");
        assert!(
            fixed.contains(
                "src=\"https://raw.githubusercontent.com/o/r/main/extensions/clock/media/two.png\""
            ),
            "{fixed}"
        );
        let drawable = html_images_as_markdown(&fixed);
        assert!(
            drawable.contains(
                "![two](https://raw.githubusercontent.com/o/r/main/extensions/clock/media/two.png)"
            ),
            "{drawable}"
        );
        assert!(!drawable.contains("<img"));
        assert_eq!(absolute_readme_images("x", "not a url"), "x");
    }

    #[test]
    fn the_platform_is_one_the_stores_know() {
        assert!(matches!(platform(), "linux" | "macos" | "windows"));
    }
}
