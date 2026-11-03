//! Modrinth client tests. The transport is mocked; nothing here touches
//! the network.

use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::sync::{Arc, Mutex};

use mine_utils_core::{
    installed_game_versions, prefer_version, preferred_launcher_version, ModrinthClient,
    ModrinthError, RawResponse, SearchKind, SearchQuery, SearchSort, Transport,
};

struct MockTransport {
    calls: Mutex<Vec<String>>,
    attempts: Mutex<HashMap<String, usize>>,
    handler: MockHandler,
}

/// The scripted answer for one request URL and its attempt number.
type MockHandler = Box<dyn Fn(&str, usize) -> Result<RawResponse, ModrinthError> + Send + Sync>;

impl MockTransport {
    fn scripted(
        handler: impl Fn(&str, usize) -> Result<RawResponse, ModrinthError> + Send + Sync + 'static,
    ) -> Self {
        MockTransport {
            calls: Mutex::new(Vec::new()),
            attempts: Mutex::new(HashMap::new()),
            handler: Box::new(handler),
        }
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().expect("calls").clone()
    }

    fn attempts_for(&self, needle: &str) -> usize {
        self.attempts
            .lock()
            .expect("attempts")
            .iter()
            .filter(|(url, _)| url.contains(needle))
            .map(|(_, count)| count)
            .sum()
    }
}

impl Transport for MockTransport {
    fn request(&self, url: &str) -> Result<RawResponse, ModrinthError> {
        self.calls.lock().expect("calls").push(url.to_string());
        let attempt = {
            let mut attempts = self.attempts.lock().expect("attempts");
            let entry = attempts.entry(url.to_string()).or_insert(0);
            *entry += 1;
            *entry
        };
        (self.handler)(url, attempt)
    }
}

fn json_response(status: u16, body: &str) -> Result<RawResponse, ModrinthError> {
    Ok(RawResponse {
        status,
        headers: vec![("content-type".to_string(), "application/json".to_string())],
        body: Box::new(Cursor::new(body.as_bytes().to_vec())),
    })
}

fn json_response_headers(
    status: u16,
    headers: Vec<(String, String)>,
    body: &str,
) -> Result<RawResponse, ModrinthError> {
    Ok(RawResponse {
        status,
        headers,
        body: Box::new(Cursor::new(body.as_bytes().to_vec())),
    })
}

fn client(
    handler: impl Fn(&str, usize) -> Result<RawResponse, ModrinthError> + Send + Sync + 'static,
) -> (ModrinthClient<MockTransport>, Arc<()>) {
    let _keep = Arc::new(());
    (
        ModrinthClient::without_sleep(MockTransport::scripted(handler)),
        _keep,
    )
}

const SEARCH_RESOURCE: &str = r#"{
  "hits": [
    {
      "project_id": "7q7JnQD0",
      "slug": "fresh-animations",
      "title": "Fresh Animations",
      "description": "Fresh animations for mobs and players.",
      "icon_url": "https://cdn.modrinth.com/icons/fresh.webp",
      "downloads": 123456,
      "follows": 789,
      "project_type": "resourcepack",
      "versions": ["1.21.1", "1.21"],
      "latest_version": "v1",
      "unknown_future_field": true
    },
    {
      "project_id": "aaaa",
      "slug": "other-pack",
      "title": "Other Pack",
      "description": "Another resource pack.",
      "downloads": 10,
      "follows": 1,
      "project_type": "resourcepack",
      "versions": ["1.21"]
    }
  ]
}"#;

const SEARCH_DATAPACK_MIXED: &str = r#"{
  "hits": [
    {
      "project_id": "mod1",
      "slug": "mod-with-datapack",
      "title": "A Mod",
      "description": "A mod that also ships datapack versions.",
      "downloads": 999,
      "follows": 9,
      "project_type": "mod",
      "versions": ["1.21.1"]
    },
    {
      "project_id": "dp1",
      "slug": "pure-datapack",
      "title": "Pure Datapack",
      "description": "Only datapack files.",
      "downloads": 500,
      "follows": 5,
      "project_type": "mod",
      "versions": ["1.21.1"]
    }
  ]
}"#;

const VERSIONS_JSON: &str = r#"[
  {
    "id": "v2",
    "project_id": "7q7JnQD0",
    "name": "Fresh Animations 1.9",
    "version_number": "1.9",
    "game_versions": ["1.21.1", "1.21"],
    "loaders": ["minecraft"],
    "files": [
      {
        "url": "https://cdn.modrinth.com/data/7q7/fresh-1.9.zip",
        "filename": "fresh-1.9.zip",
        "size": 1024,
        "primary": true,
        "hashes": {"sha512": "abc", "sha1": "def"}
      }
    ]
  },
  {
    "id": "v1",
    "project_id": "7q7JnQD0",
    "name": "Fresh Animations 1.8",
    "version_number": "1.8",
    "game_versions": ["1.20.1"],
    "loaders": ["minecraft"],
    "files": []
  }
]"#;

const PROJECT_JSON: &str = r#"{
  "id": "7q7JnQD0",
  "slug": "fresh-animations",
  "title": "Fresh Animations",
  "description": "Fresh animations for mobs and players.",
  "body": "Long body.",
  "icon_url": "https://cdn.modrinth.com/icons/fresh.webp",
  "downloads": 123456,
  "follows": 789,
  "project_type": "resourcepack",
  "game_versions": ["1.21.1", "1.21"],
  "versions": ["v2", "v1"]
}"#;

fn query(kind: SearchKind) -> SearchQuery {
    SearchQuery {
        text: "fresh".to_string(),
        kind,
        game_version: None,
        sort: SearchSort::Relevance,
        limit: 20,
        offset: 0,
    }
}

#[test]
fn search_resourcepacks_returns_hits() {
    let (client, _) = client(|url, _| {
        assert!(url.contains("/v2/search"), "unexpected {url}");
        json_response(200, SEARCH_RESOURCE)
    });
    let hits = client
        .search(&query(SearchKind::ResourcePack))
        .expect("search");
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].title, "Fresh Animations");
    assert_eq!(
        hits[0].icon_url.as_deref(),
        Some("https://cdn.modrinth.com/icons/fresh.webp")
    );
}

#[test]
fn search_datapack_keeps_mods_with_datapack_versions() {
    let (client, _) = client(|_, _| json_response(200, SEARCH_DATAPACK_MIXED));
    let hits = client.search(&query(SearchKind::Datapack)).expect("search");
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].slug, "mod-with-datapack");
    assert_eq!(hits[1].slug, "pure-datapack");
}

#[test]
fn search_sends_facets_and_sort() {
    let (client, _) = client(|url, _| {
        assert!(url.contains("project_type"), "facet missing in {url}");
        json_response(200, r#"{"hits": []}"#)
    });
    let mut q = query(SearchKind::ResourcePack);
    q.game_version = Some("1.21.1".to_string());
    q.sort = SearchSort::Downloads;
    let hits = client.search(&q).expect("search");
    assert!(hits.is_empty());
    let calls = client.transport().calls();
    assert_eq!(calls.len(), 1);
    assert!(
        calls[0].contains("index=downloads"),
        "sort missing: {}",
        calls[0]
    );
    assert!(
        calls[0].contains("resourcepack"),
        "type facet missing: {}",
        calls[0]
    );
}

#[test]
fn search_sends_offset_and_limit() {
    let (client, _) = client(|url, _| {
        assert!(url.contains("offset=40"), "offset missing in {url}");
        assert!(url.contains("limit=20"), "limit missing in {url}");
        json_response(200, r#"{"hits": []}"#)
    });
    let mut q = query(SearchKind::ResourcePack);
    q.offset = 40;
    let hits = client.search(&q).expect("search");
    assert!(hits.is_empty());
}

#[test]
fn project_and_versions_parse() {
    let (client, _) = client(|url, _| {
        if url.contains("/version") {
            json_response(200, VERSIONS_JSON)
        } else {
            json_response(200, PROJECT_JSON)
        }
    });
    let project = client.project("fresh-animations").expect("project");
    assert_eq!(project.title, "Fresh Animations");
    let versions = client.versions("7q7JnQD0").expect("versions");
    assert_eq!(versions.len(), 2);
    assert_eq!(versions[0].game_versions, vec!["1.21.1", "1.21"]);
    assert!(versions[0].files[0].primary);
}

#[test]
fn retry_429_with_retry_after_then_success() {
    let (client, _) = client(|_, attempt| {
        if attempt == 1 {
            json_response_headers(
                429,
                vec![("retry-after".to_string(), "0".to_string())],
                "slow down",
            )
        } else {
            json_response(200, r#"{"hits": []}"#)
        }
    });
    let hits = client
        .search(&query(SearchKind::ResourcePack))
        .expect("search");
    assert!(hits.is_empty());
    assert_eq!(client.transport().attempts_for("/v2/search"), 2);
}

#[test]
fn retry_500_exhausts_after_three_retries() {
    let (client, _) = client(|_, _| json_response(500, "boom"));
    let err = client
        .search(&query(SearchKind::ResourcePack))
        .expect_err("500 must fail");
    assert!(matches!(
        err,
        ModrinthError::RetryExhausted { status: 500, .. }
    ));
    assert_eq!(err.code(), "modrinth_status");
    assert_eq!(client.transport().attempts_for("/v2/search"), 4);
}

#[test]
fn redirect_to_non_modrinth_host_is_rejected() {
    let (client, _) = client(|_, _| {
        json_response_headers(
            302,
            vec![("location".to_string(), "https://evil.example/x".to_string())],
            "",
        )
    });
    let err = client
        .search(&query(SearchKind::ResourcePack))
        .expect_err("evil redirect must fail");
    assert!(matches!(err, ModrinthError::Redirect { .. }));
    assert_eq!(err.code(), "modrinth_redirect");
    assert_eq!(client.transport().attempts_for("/v2/search"), 1);
}

#[test]
fn redirect_within_modrinth_hosts_is_followed() {
    let (client, _) = client(|url, _| {
        if url.contains("facets") {
            json_response_headers(
                302,
                vec![(
                    "location".to_string(),
                    "https://api.modrinth.com/v2/search?query=x".to_string(),
                )],
                "",
            )
        } else {
            json_response(200, r#"{"hits": []}"#)
        }
    });
    let hits = client
        .search(&query(SearchKind::ResourcePack))
        .expect("search");
    assert!(hits.is_empty());
    assert_eq!(client.transport().calls().len(), 2);
}

#[test]
fn oversized_body_is_rejected() {
    let (client, _) = client(|_, _| {
        Ok(RawResponse {
            status: 200,
            headers: vec![],
            body: Box::new(OversizedBody::new()),
        })
    });
    let err = client
        .search(&query(SearchKind::ResourcePack))
        .expect_err("oversized must fail");
    assert!(matches!(err, ModrinthError::BodyTooLarge { .. }));
    assert_eq!(err.code(), "modrinth_body");
}

struct OversizedBody {
    remaining: usize,
}

impl OversizedBody {
    fn new() -> Self {
        OversizedBody {
            remaining: mine_utils_core::MAX_API_BODY + 16,
        }
    }
}

impl Read for OversizedBody {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.remaining == 0 {
            return Ok(0);
        }
        let n = buf.len().min(self.remaining).min(8192);
        buf[..n].fill(b'x');
        self.remaining -= n;
        Ok(n)
    }
}

#[test]
fn file_stream_rejects_non_cdn_urls() {
    let (client, _) = client(|_, _| json_response(200, "{}"));
    let err = client
        .file_stream("https://evil.example/pack.zip")
        .expect_err("non-CDN must fail");
    assert_eq!(err.code(), "modrinth_redirect");
    assert!(client.transport().calls().is_empty());
}

#[test]
fn installed_version_selection_prefers_launcher_version() {
    let dir = tempfile::tempdir().expect("tempdir");
    for version in ["1.21", "1.21.1"] {
        std::fs::create_dir_all(dir.path().join("versions").join(version)).expect("mkdir");
    }
    std::fs::write(
        dir.path().join("launcher_profiles.json"),
        r#"{"selectedProfile": "abc", "profiles": {"abc": {"lastVersionId": "1.21.1"}}}"#,
    )
    .expect("write");
    let installed = installed_game_versions(dir.path());
    assert_eq!(installed, vec!["1.21".to_string(), "1.21.1".to_string()]);
    let preferred = preferred_launcher_version(dir.path());
    assert_eq!(preferred.as_deref(), Some("1.21.1"));

    let (client, _) = client(|_, _| json_response(200, VERSIONS_JSON));
    let versions = client.versions("7q7JnQD0").expect("versions");
    let picked = prefer_version(&versions, preferred.as_deref()).expect("pick");
    assert_eq!(picked.id, "v2");
    let fallback = prefer_version(&versions, Some("1.19")).expect("fallback");
    assert_eq!(fallback.id, "v2");
}

#[test]
fn user_agent_names_the_app() {
    let agent = mine_utils_core::modrinth::user_agent();
    assert!(agent.starts_with("botij0/mine-utils/"), "agent: {agent}");
    assert!(
        agent.contains("github.com/botij0/mine-utils-manager"),
        "agent: {agent}"
    );
}
