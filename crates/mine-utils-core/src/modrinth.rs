//! Modrinth discovery: search, project and version metadata.
//!
//! The HTTP transport is injectable so tests never touch the network. The
//! production transport is [`ReqwestTransport`]; tests supply a mock from
//! `tests/modrinth_client.rs`. No request is made before the Discover tab
//! opens: commands only call into this module from the search page.

use std::io::Read;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

/// Search and metadata requests go here. Downloads come from the CDN.
pub const API_BASE: &str = "https://api.modrinth.com";
/// The only host a pack file may be downloaded from.
pub const CDN_HOST: &str = "cdn.modrinth.com";
/// Hosts a redirect may legally land on. Anything else is rejected.
pub const ALLOWED_HOSTS: &[&str] = &["api.modrinth.com", "cdn.modrinth.com"];
/// The API answers are small; anything bigger is treated as hostile.
pub const MAX_API_BODY: usize = 4 * 1024 * 1024;
/// A downloaded pack file may not exceed 500 MB.
pub const MAX_DOWNLOAD_BYTES: u64 = 500 * 1024 * 1024;
/// Retries for 429/5xx before giving up.
pub const MAX_RETRIES: u32 = 3;
/// Redirect hops followed before giving up.
pub const MAX_REDIRECTS: u32 = 5;
/// Seconds before a request is abandoned.
pub const REQUEST_TIMEOUT_SECS: u64 = 15;

/// ` Fn(seconds) ` run before each retry. Production sleeps; tests pass a
/// no-op and count attempts instead.
pub type SleepFn = Arc<dyn Fn(u64) + Send + Sync>;

/// What the app reports for a Modrinth failure. The `code` is the wire code
/// the frontend switches on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModrinthError {
    /// The network is unreachable or TLS/DNS failed. The UI shows offline.
    Offline { reason: String },
    /// The API answered with a non-2xx, non-retried status.
    Status { status: u16, url: String },
    /// Retries for 429/5xx were exhausted.
    RetryExhausted { status: u16, url: String },
    /// A redirect pointed at a host outside [`ALLOWED_HOSTS`].
    Redirect { host: String },
    /// Too many redirect hops.
    TooManyRedirects { url: String },
    /// The response body exceeded [`MAX_API_BODY`].
    BodyTooLarge { url: String },
    /// The body was not the JSON shape expected.
    Parse { reason: String },
    /// The URL was malformed.
    BadUrl { url: String, reason: String },
}

impl ModrinthError {
    pub fn code(&self) -> &'static str {
        match self {
            ModrinthError::Offline { .. } => "modrinth_offline",
            ModrinthError::Status { .. } => "modrinth_status",
            ModrinthError::RetryExhausted { .. } => "modrinth_status",
            ModrinthError::Redirect { .. } => "modrinth_redirect",
            ModrinthError::TooManyRedirects { .. } => "modrinth_redirect",
            ModrinthError::BodyTooLarge { .. } => "modrinth_body",
            ModrinthError::Parse { .. } => "modrinth_parse",
            ModrinthError::BadUrl { .. } => "modrinth_url",
        }
    }
}

impl std::fmt::Display for ModrinthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ModrinthError::Offline { reason } => {
                write!(f, "Modrinth is unreachable ({reason}); check the network")
            }
            ModrinthError::Status { status, url } => {
                write!(f, "Modrinth request failed with status {status} for {url}")
            }
            ModrinthError::RetryExhausted { status, url } => {
                write!(
                    f,
                    "Modrinth request to {url} still fails with status {status} after retries"
                )
            }
            ModrinthError::Redirect { host } => {
                write!(f, "Modrinth redirected to unexpected host {host}; refused")
            }
            ModrinthError::TooManyRedirects { url } => {
                write!(f, "Modrinth redirected too many times for {url}; refused")
            }
            ModrinthError::BodyTooLarge { url } => {
                write!(
                    f,
                    "Modrinth response for {url} exceeded the size cap; refused"
                )
            }
            ModrinthError::Parse { reason } => {
                write!(f, "Modrinth response could not be read: {reason}")
            }
            ModrinthError::BadUrl { url, reason } => {
                write!(f, "bad Modrinth URL {url}: {reason}")
            }
        }
    }
}

impl std::error::Error for ModrinthError {}

/// One raw HTTP answer. The body is a reader so downloads never buffer the
/// whole file in memory.
pub struct RawResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Box<dyn Read + Send>,
}

/// The injectable HTTP transport. `request` performs exactly one exchange:
/// no redirects, no retries. [`ModrinthClient`] adds those.
pub trait Transport: Send + Sync {
    fn request(&self, url: &str) -> Result<RawResponse, ModrinthError>;
}

/// Shared transports (e.g. `Arc` in Tauri state) forward to the inner one.
impl<T: Transport> Transport for std::sync::Arc<T> {
    fn request(&self, url: &str) -> Result<RawResponse, ModrinthError> {
        (**self).request(url)
    }
}

/// `reqwest` over rustls, 15 s timeout, the app User-Agent, no automatic
/// redirects: every hop is checked against [`ALLOWED_HOSTS`] by the client.
pub struct ReqwestTransport {
    client: reqwest::blocking::Client,
}

impl ReqwestTransport {
    pub fn new() -> Result<Self, ModrinthError> {
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(REQUEST_TIMEOUT_SECS))
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(user_agent())
            .build()
            .map_err(|err| ModrinthError::Offline {
                reason: err.to_string(),
            })?;
        Ok(ReqwestTransport { client })
    }
}

impl Default for ReqwestTransport {
    fn default() -> Self {
        ReqwestTransport::new().expect("HTTP client builds")
    }
}

/// `botij0/mine-utils/<version> (github.com/botij0/mine-utils-manager)`.
pub fn user_agent() -> String {
    format!(
        "botij0/mine-utils/{} (github.com/botij0/mine-utils-manager)",
        env!("CARGO_PKG_VERSION")
    )
}

fn classify_reqwest(err: reqwest::Error, url: &str) -> ModrinthError {
    if err.is_redirect() {
        let host = err
            .url()
            .and_then(|u| u.host_str().map(str::to_string))
            .unwrap_or_default();
        return ModrinthError::Redirect { host };
    }
    if err.is_status() {
        if let Some(status) = err.status() {
            return ModrinthError::Status {
                status: status.as_u16(),
                url: url.to_string(),
            };
        }
    }
    ModrinthError::Offline {
        reason: err.to_string(),
    }
}

impl Transport for ReqwestTransport {
    fn request(&self, url: &str) -> Result<RawResponse, ModrinthError> {
        let response = self
            .client
            .get(url)
            .send()
            .map_err(|err| classify_reqwest(err, url))?;
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .map(|(name, value)| {
                (
                    name.as_str().to_ascii_lowercase(),
                    value.to_str().unwrap_or("").to_string(),
                )
            })
            .collect::<Vec<_>>();
        Ok(RawResponse {
            status,
            headers,
            body: Box::new(response),
        })
    }
}

/// A 2xx answer after redirects and retries resolved the final URL.
pub struct Fetched {
    pub url: String,
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Box<dyn Read + Send>,
}

impl std::fmt::Debug for Fetched {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Fetched")
            .field("url", &self.url)
            .field("status", &self.status)
            .field("headers", &self.headers)
            .finish_non_exhaustive()
    }
}

fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

/// `Retry-After` in seconds, or an HTTP-date converted to a wait. Anything
/// unparseable falls back to exponential backoff.
pub fn retry_delay_secs(headers: &[(String, String)], attempt: u32) -> u64 {
    let backoff = (1u64 << attempt.min(4)).min(8);
    let Some(raw) = header(headers, "retry-after") else {
        return backoff;
    };
    if let Ok(secs) = raw.trim().parse::<u64>() {
        return secs.clamp(1, 30);
    }
    if let Some(secs) = http_date_wait_secs(raw.trim()) {
        return secs.clamp(1, 30);
    }
    backoff
}

fn http_date_wait_secs(raw: &str) -> Option<u64> {
    let without_weekday = raw.split_once(", ")?.1;
    let mut parts = without_weekday.split_whitespace();
    let day: i64 = parts.next()?.parse().ok()?;
    let month = match parts.next()? {
        "Jan" => 1,
        "Feb" => 2,
        "Mar" => 3,
        "Apr" => 4,
        "May" => 5,
        "Jun" => 6,
        "Jul" => 7,
        "Aug" => 8,
        "Sep" => 9,
        "Oct" => 10,
        "Nov" => 11,
        "Dec" => 12,
        _ => return None,
    };
    let year: i64 = parts.next()?.parse().ok()?;
    let time = parts.next()?;
    if parts.next()? != "GMT" {
        return None;
    }
    let mut clock = time.split(':');
    let hour: i64 = clock.next()?.parse().ok()?;
    let minute: i64 = clock.next()?.parse().ok()?;
    let second: i64 = clock.next()?.parse().ok()?;
    let days = days_since_epoch(year, month, day)?;
    let stamp = days * 86_400 + hour * 3600 + minute * 60 + second;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs() as i64;
    let wait = stamp.saturating_sub(now);
    u64::try_from(wait).ok()
}

fn days_since_epoch(year: i64, month: i64, day: i64) -> Option<i64> {
    if !(1970..=2100).contains(&year) || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let leap = |y: i64| y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let mut days = 0i64;
    let mut y = 1970;
    while y < year {
        days += if leap(y) { 366 } else { 365 };
        y += 1;
    }
    let mut m = 1;
    while m < month {
        days += match m {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            2 => {
                if leap(year) {
                    29
                } else {
                    28
                }
            }
            _ => return None,
        };
        m += 1;
    }
    Some(days + day - 1)
}

fn join_url(base: &str, location: &str) -> Result<String, ModrinthError> {
    let base_url = url::Url::parse(base).map_err(|err| ModrinthError::BadUrl {
        url: base.to_string(),
        reason: err.to_string(),
    })?;
    base_url
        .join(location)
        .map(|u| u.to_string())
        .map_err(|err| ModrinthError::BadUrl {
            url: location.to_string(),
            reason: err.to_string(),
        })
}

fn host_of(value: &str) -> Result<String, ModrinthError> {
    url::Url::parse(value)
        .map_err(|err| ModrinthError::BadUrl {
            url: value.to_string(),
            reason: err.to_string(),
        })?
        .host_str()
        .map(str::to_string)
        .ok_or_else(|| ModrinthError::BadUrl {
            url: value.to_string(),
            reason: "no host".to_string(),
        })
}

/// Send one request with retries for 429/5xx, honoring `Retry-After`.
fn send<T: Transport + ?Sized>(
    transport: &T,
    url: &str,
    sleep: &SleepFn,
) -> Result<RawResponse, ModrinthError> {
    let mut attempt: u32 = 0;
    loop {
        let response = transport.request(url)?;
        match response.status {
            429 | 500..=599 => {
                if attempt >= MAX_RETRIES {
                    return Err(ModrinthError::RetryExhausted {
                        status: response.status,
                        url: url.to_string(),
                    });
                }
                // Drain the body so a pooled connection can be reused.
                let mut sink = std::io::sink();
                let _ = std::io::copy(&mut { response.body }, &mut sink);
                sleep(retry_delay_secs(&response.headers, attempt));
                attempt += 1;
            }
            _ => return Ok(response),
        }
    }
}

/// Follow redirects (same allowlist), then retry 429/5xx per hop.
pub fn fetch<T: Transport + ?Sized>(
    transport: &T,
    url: &str,
    sleep: &SleepFn,
) -> Result<Fetched, ModrinthError> {
    let mut current = url.to_string();
    for _ in 0..=MAX_REDIRECTS {
        let response = send(transport, &current, sleep)?;
        if (300..400).contains(&response.status) {
            let location = header(&response.headers, "location")
                .ok_or_else(|| ModrinthError::Redirect {
                    host: String::new(),
                })?
                .to_string();
            let next = join_url(&current, &location)?;
            let host = host_of(&next)?;
            if !ALLOWED_HOSTS.contains(&host.as_str()) {
                return Err(ModrinthError::Redirect { host });
            }
            current = next;
            continue;
        }
        return Ok(Fetched {
            url: current,
            status: response.status,
            headers: response.headers,
            body: response.body,
        });
    }
    Err(ModrinthError::TooManyRedirects { url: current })
}

/// Read a capped API body. Anything over [`MAX_API_BODY`] is refused.
pub fn read_capped(body: Box<dyn Read + Send>, url: &str) -> Result<Vec<u8>, ModrinthError> {
    let mut out = Vec::new();
    body.take((MAX_API_BODY as u64) + 1)
        .read_to_end(&mut out)
        .map_err(|err| ModrinthError::Offline {
            reason: err.to_string(),
        })?;
    if out.len() > MAX_API_BODY {
        return Err(ModrinthError::BodyTooLarge {
            url: url.to_string(),
        });
    }
    Ok(out)
}

/// Which Modrinth project type the Discover tab lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchKind {
    ResourcePack,
    Datapack,
}

/// The sort orders the Discover page offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchSort {
    Relevance,
    Downloads,
    Follows,
    Newest,
    Updated,
}

impl SearchSort {
    fn index(self) -> &'static str {
        match self {
            SearchSort::Relevance => "relevance",
            SearchSort::Downloads => "downloads",
            SearchSort::Follows => "follows",
            SearchSort::Newest => "newest",
            SearchSort::Updated => "updated",
        }
    }
}

/// One Discover search. Nothing is fetched until the page calls [`search`].
#[derive(Debug, Clone)]
pub struct SearchQuery {
    pub text: String,
    pub kind: SearchKind,
    pub game_version: Option<String>,
    pub sort: SearchSort,
    pub limit: u8,
    pub offset: u32,
}

/// A search hit. Unknown fields are ignored so API additions never break us.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchHit {
    pub project_id: String,
    #[serde(default)]
    pub slug: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub icon_url: Option<String>,
    #[serde(default)]
    pub downloads: u64,
    #[serde(default)]
    pub follows: u64,
    #[serde(default)]
    pub project_type: Option<String>,
    #[serde(default)]
    pub versions: Vec<String>,
    #[serde(default)]
    pub latest_version: Option<String>,
}

/// The `/v2/search` answer.
#[derive(Debug, Clone, Deserialize)]
struct SearchAnswer {
    #[serde(default)]
    hits: Vec<SearchHit>,
}

/// Project metadata for the detail sheet.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModrinthProject {
    pub id: String,
    #[serde(default)]
    pub slug: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub icon_url: Option<String>,
    #[serde(default)]
    pub downloads: u64,
    #[serde(default)]
    pub follows: u64,
    #[serde(default)]
    pub project_type: Option<String>,
    #[serde(default)]
    pub game_versions: Vec<String>,
    #[serde(default)]
    pub versions: Vec<String>,
}

/// One file inside a version. Only the CDN URL is ever downloaded.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModrinthFile {
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub filename: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub primary: bool,
    #[serde(default)]
    pub hashes: ModrinthHashes,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModrinthHashes {
    #[serde(default)]
    pub sha512: String,
    #[serde(default)]
    pub sha1: String,
}

/// One project version with its game versions and files.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModrinthVersion {
    pub id: String,
    pub project_id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub version_number: String,
    #[serde(default)]
    pub game_versions: Vec<String>,
    #[serde(default)]
    pub loaders: Vec<String>,
    #[serde(default)]
    pub files: Vec<ModrinthFile>,
}

/// The client. One shared instance lives in Tauri state.
pub struct ModrinthClient<T: Transport> {
    transport: T,
    sleep: SleepFn,
}

impl<T: Transport> ModrinthClient<T> {
    pub fn new(transport: T) -> Self {
        ModrinthClient {
            transport,
            sleep: Arc::new(|secs| std::thread::sleep(std::time::Duration::from_secs(secs))),
        }
    }

    /// No sleeping between retries. Tests use this and count attempts.
    pub fn without_sleep(transport: T) -> Self {
        ModrinthClient {
            transport,
            sleep: Arc::new(|_| {}),
        }
    }

    pub fn transport(&self) -> &T {
        &self.transport
    }

    fn get_json<U: for<'de> Deserialize<'de>>(&self, url: &str) -> Result<U, ModrinthError> {
        let fetched = fetch(&self.transport, url, &self.sleep)?;
        if !(200..300).contains(&fetched.status) {
            return Err(ModrinthError::Status {
                status: fetched.status,
                url: fetched.url,
            });
        }
        let owned_url = fetched.url.clone();
        let bytes = read_capped(fetched.body, &owned_url)?;
        serde_json::from_slice(&bytes).map_err(|err| ModrinthError::Parse {
            reason: err.to_string(),
        })
    }

    /// Only [`ALLOWED_HOSTS`] are fetched; the check happens on every hop.
    fn api_url(&self, path: &str) -> Result<String, ModrinthError> {
        let url = format!("{API_BASE}{path}");
        let host = host_of(&url)?;
        if host != "api.modrinth.com" {
            return Err(ModrinthError::Redirect { host });
        }
        Ok(url)
    }

    /// Search projects. For datapacks the facet also returns mods with
    /// datapack versions, so hits are filtered to `project_type == datapack`.
    pub fn search(&self, query: &SearchQuery) -> Result<Vec<SearchHit>, ModrinthError> {
        let project_type = match query.kind {
            SearchKind::ResourcePack => "resourcepack",
            SearchKind::Datapack => "datapack",
        };
        let mut facets: Vec<Vec<String>> = vec![vec![format!("project_type:{project_type}")]];
        if let Some(version) = &query.game_version {
            facets.push(vec![format!("versions:{version}")]);
        }
        let facets_json = serde_json::to_string(&facets).map_err(|err| ModrinthError::Parse {
            reason: err.to_string(),
        })?;
        let url = url::Url::parse_with_params(
            &format!("{API_BASE}/v2/search"),
            &[
                ("query", query.text.as_str()),
                ("facets", facets_json.as_str()),
                ("limit", &query.limit.clamp(1, 100).to_string()),
                ("offset", &query.offset.to_string()),
                ("index", query.sort.index()),
            ],
        )
        .map_err(|err| ModrinthError::BadUrl {
            url: format!("{API_BASE}/v2/search"),
            reason: err.to_string(),
        })?
        .to_string();
        let answer: SearchAnswer = self.get_json(&url)?;
        if query.kind == SearchKind::Datapack {
            Ok(answer
                .hits
                .into_iter()
                .filter(|hit| hit.project_type.as_deref() == Some("datapack"))
                .collect())
        } else {
            Ok(answer.hits)
        }
    }

    /// Project metadata for the detail sheet.
    pub fn project(&self, id_or_slug: &str) -> Result<ModrinthProject, ModrinthError> {
        let url = self.api_url(&format!("/v2/project/{id_or_slug}"))?;
        self.get_json(&url)
    }

    /// Every version of a project, newest first as the API returns them.
    pub fn versions(&self, project_id: &str) -> Result<Vec<ModrinthVersion>, ModrinthError> {
        let url = self.api_url(&format!("/v2/project/{project_id}/version"))?;
        self.get_json(&url)
    }

    /// The raw file stream for [`crate::download`], after redirects/retries.
    /// The final URL must be the CDN; anything else is refused.
    pub fn file_stream(&self, url: &str) -> Result<Fetched, ModrinthError> {
        let host = host_of(url)?;
        if host != CDN_HOST {
            return Err(ModrinthError::Redirect { host });
        }
        let fetched = fetch(&self.transport, url, &self.sleep)?;
        if !(200..300).contains(&fetched.status) {
            return Err(ModrinthError::Status {
                status: fetched.status,
                url: fetched.url,
            });
        }
        let host = host_of(&fetched.url)?;
        if host != CDN_HOST {
            return Err(ModrinthError::Redirect { host });
        }
        Ok(fetched)
    }
}

/// Game versions installed in the game dir: the direct children of
/// `versions/`. Only the caller-selected versions are ever trusted.
pub fn installed_game_versions(game_dir: &std::path::Path) -> Vec<String> {
    let mut versions = Vec::new();
    let Ok(entries) = std::fs::read_dir(game_dir.join("versions")) else {
        return versions;
    };
    for entry in entries.flatten() {
        if let Ok(kind) = entry.file_type() {
            if kind.is_dir() {
                versions.push(entry.file_name().to_string_lossy().into_owned());
            }
        }
    }
    versions.sort();
    versions
}

/// The launcher's selected (or last) version from `launcher_profiles.json`,
/// the resource-pack preference. `None` when nothing selected one.
pub fn preferred_launcher_version(game_dir: &std::path::Path) -> Option<String> {
    let bytes = std::fs::read(game_dir.join("launcher_profiles.json")).ok()?;
    let json: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let profiles = json.get("profiles")?.as_object()?;
    if let Some(selected) = json.get("selectedProfile").and_then(|v| v.as_str()) {
        if let Some(version) = profiles
            .get(selected)
            .and_then(|p| p.get("lastVersionId"))
            .and_then(|v| v.as_str())
        {
            return Some(version.to_string());
        }
    }
    profiles.values().find_map(|profile| {
        profile
            .get("lastVersionId")
            .and_then(|v| v.as_str())
            .map(str::to_string)
    })
}

/// Pick the default version for the picker: the first version compatible
/// with the target, else the newest. The picker still lists every version;
/// this is a default, never a filter.
pub fn prefer_version<'a>(
    versions: &'a [ModrinthVersion],
    target: Option<&str>,
) -> Option<&'a ModrinthVersion> {
    if versions.is_empty() {
        return None;
    }
    if let Some(target) = target {
        if let Some(hit) = versions
            .iter()
            .find(|v| v.game_versions.iter().any(|g| g == target))
        {
            return Some(hit);
        }
    }
    versions.first()
}

/// The primary file of a version, or the first file when none is marked.
pub fn primary_file(version: &ModrinthVersion) -> Option<&ModrinthFile> {
    version
        .files
        .iter()
        .find(|f| f.primary)
        .or_else(|| version.files.first())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_after_seconds_wins_over_backoff() {
        assert_eq!(
            retry_delay_secs(&[("retry-after".into(), "7".into())], 0),
            7
        );
    }

    #[test]
    fn retry_after_caps_at_thirty() {
        assert_eq!(
            retry_delay_secs(&[("retry-after".into(), "3600".into())], 0),
            30
        );
    }

    #[test]
    fn unparseable_retry_after_falls_back_to_backoff() {
        assert_eq!(
            retry_delay_secs(&[("retry-after".into(), "soon".into())], 1),
            2
        );
    }

    #[test]
    fn prefer_version_picks_compatible_first() {
        let versions = vec![
            ModrinthVersion {
                id: "new".into(),
                project_id: "p".into(),
                name: String::new(),
                version_number: "2".into(),
                game_versions: vec!["1.21.2".into()],
                loaders: vec![],
                files: vec![],
            },
            ModrinthVersion {
                id: "old".into(),
                project_id: "p".into(),
                name: String::new(),
                version_number: "1".into(),
                game_versions: vec!["1.21.1".into()],
                loaders: vec![],
                files: vec![],
            },
        ];
        assert_eq!(
            prefer_version(&versions, Some("1.21.1")).map(|v| v.id.as_str()),
            Some("old")
        );
        assert_eq!(
            prefer_version(&versions, Some("1.20.1")).map(|v| v.id.as_str()),
            Some("new")
        );
        assert!(prefer_version(&[], Some("1.21.1")).is_none());
    }
}
