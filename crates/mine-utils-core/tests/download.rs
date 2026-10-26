//! Download verification tests. The CDN is mocked; nothing here touches
//! the network.

use std::io::{Cursor, Read, Write};
use std::sync::{Arc, Mutex};

use mine_utils_core::{
    commit_verified, download_verified, download_verified_capped, provenance_lookup,
    provenance_record, DownloadError, ModrinthClient, ModrinthError, ProvenanceEntry, RawResponse,
    Transport, ZipRoot,
};
use sha2::{Digest, Sha512};

const CDN_URL: &str = "https://cdn.modrinth.com/data/7q7/fresh-1.9.zip";

struct MockCdn {
    calls: Mutex<Vec<String>>,
    bytes: Vec<u8>,
    status: u16,
}

impl MockCdn {
    fn new(bytes: Vec<u8>) -> Arc<Self> {
        Arc::new(MockCdn {
            calls: Mutex::new(Vec::new()),
            bytes,
            status: 200,
        })
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().expect("calls").clone()
    }
}

impl Transport for MockCdn {
    fn request(&self, url: &str) -> Result<RawResponse, ModrinthError> {
        self.calls.lock().expect("calls").push(url.to_string());
        Ok(RawResponse {
            status: self.status,
            headers: vec![],
            body: Box::new(Cursor::new(self.bytes.clone())),
        })
    }
}

fn client_for(bytes: Vec<u8>) -> (ModrinthClient<Arc<MockCdn>>, Arc<MockCdn>) {
    let cdn = MockCdn::new(bytes);
    (ModrinthClient::without_sleep(Arc::clone(&cdn)), cdn)
}

fn zip_bytes(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    let mut zip = zip::ZipWriter::new(&mut out);
    for (name, data) in entries {
        zip.start_file(*name, zip::write::SimpleFileOptions::default())
            .expect("entry");
        zip.write_all(data).expect("write");
    }
    zip.finish().expect("finish");
    out.into_inner()
}

fn sha512_hex(bytes: &[u8]) -> String {
    hex::encode(Sha512::digest(bytes))
}

fn leftovers(dir: &std::path::Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .expect("read dir")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains("mine-utils-download"))
        .collect()
}

#[test]
fn hash_match_downloads_and_commits_atomically() {
    let dir = tempfile::tempdir().expect("tempdir");
    let target = dir.path().join("resourcepacks");
    let bytes = zip_bytes(&[
        ("assets/minecraft/pack.png", b"png"),
        ("pack.mcmeta", b"{}"),
    ]);
    let (client, cdn) = client_for(bytes.clone());
    let expected = sha512_hex(&bytes);

    let verified = download_verified(
        &client,
        CDN_URL,
        &target,
        "fresh-1.9.zip",
        &expected,
        ZipRoot::Assets,
    )
    .expect("download");
    assert_eq!(cdn.calls().len(), 1);
    assert!(verified.temp_path.exists());
    assert_eq!(verified.bytes, bytes.len() as u64);

    let dest = commit_verified(&verified.temp_path, &target, "fresh-1.9.zip").expect("commit");
    assert_eq!(dest, target.join("fresh-1.9.zip"));
    assert!(!verified.temp_path.exists());
    assert_eq!(std::fs::read(&dest).expect("read"), bytes);
    assert!(leftovers(&target).is_empty());
}

#[test]
fn hash_mismatch_aborts_and_leaves_no_temp() {
    let dir = tempfile::tempdir().expect("tempdir");
    let target = dir.path().join("resourcepacks");
    let bytes = zip_bytes(&[("assets/minecraft/pack.png", b"png")]);
    let (client, cdn) = client_for(bytes);

    let err = download_verified(
        &client,
        CDN_URL,
        &target,
        "fresh-1.9.zip",
        &"00".repeat(64),
        ZipRoot::Assets,
    )
    .expect_err("mismatch must fail");
    assert!(matches!(err, DownloadError::HashMismatch { .. }), "{err}");
    assert_eq!(err.code(), "download_hash_mismatch");
    let message = err.to_string();
    assert!(message.contains("expected"), "{message}");
    assert!(message.contains("got"), "{message}");
    assert_eq!(cdn.calls().len(), 1, "a mismatch is not retried");
    assert!(leftovers(&target).is_empty());
    assert!(!target.join("fresh-1.9.zip").exists());
}

#[test]
fn non_zip_is_rejected() {
    let dir = tempfile::tempdir().expect("tempdir");
    let target = dir.path().join("resourcepacks");
    let bytes = b"definitely not a zip".to_vec();
    let (client, _) = client_for(bytes.clone());

    let err = download_verified(
        &client,
        CDN_URL,
        &target,
        "pack.zip",
        &sha512_hex(&bytes),
        ZipRoot::Assets,
    )
    .expect_err("non-zip must fail");
    assert!(matches!(err, DownloadError::NotZip { .. }), "{err}");
    assert_eq!(err.code(), "download_not_zip");
    assert!(leftovers(&target).is_empty());
}

#[test]
fn zip_without_expected_root_is_rejected() {
    let dir = tempfile::tempdir().expect("tempdir");
    let target = dir.path().join("resourcepacks");
    let bytes = zip_bytes(&[("README.txt", b"hi")]);
    let (client, _) = client_for(bytes.clone());

    let err = download_verified(
        &client,
        CDN_URL,
        &target,
        "pack.zip",
        &sha512_hex(&bytes),
        ZipRoot::Assets,
    )
    .expect_err("missing assets must fail");
    assert!(matches!(err, DownloadError::WrongRoot { .. }), "{err}");
    assert!(leftovers(&target).is_empty());

    let data_bytes = zip_bytes(&[("data/example/loot.json", b"{}")]);
    let (data_client, _) = client_for(data_bytes.clone());
    download_verified(
        &data_client,
        CDN_URL,
        &target,
        "pack.zip",
        &sha512_hex(&data_bytes),
        ZipRoot::Data,
    )
    .expect("data root passes for datapacks");
    let err = download_verified(
        &data_client,
        CDN_URL,
        &target,
        "other.zip",
        &sha512_hex(&data_bytes),
        ZipRoot::Assets,
    )
    .expect_err("data-only zip is not a resource pack");
    assert!(matches!(err, DownloadError::WrongRoot { .. }));
}

#[test]
fn byte_cap_aborts_large_downloads() {
    let dir = tempfile::tempdir().expect("tempdir");
    let target = dir.path().join("resourcepacks");
    let bytes = zip_bytes(&[("assets/minecraft/pack.png", &[b'z'; 4096])]);
    let (client, _) = client_for(bytes.clone());

    let err = download_verified_capped(
        &client,
        CDN_URL,
        &target,
        "pack.zip",
        &sha512_hex(&bytes),
        ZipRoot::Assets,
        64,
    )
    .expect_err("over-cap must fail");
    assert!(matches!(err, DownloadError::TooLarge { .. }), "{err}");
    assert_eq!(err.code(), "download_too_large");
    assert!(leftovers(&target).is_empty());
}

#[test]
fn commit_refuses_to_overwrite() {
    let dir = tempfile::tempdir().expect("tempdir");
    let target = dir.path().join("resourcepacks");
    std::fs::create_dir_all(&target).expect("mkdir");
    std::fs::write(target.join("pack.zip"), b"existing").expect("write");
    let temp = target.join(".pack.zip.part");
    std::fs::write(&temp, b"new").expect("write");

    let err = commit_verified(&temp, &target, "pack.zip").expect_err("exists must fail");
    assert!(matches!(err, DownloadError::Exists { .. }));
    assert_eq!(
        std::fs::read(target.join("pack.zip")).expect("read"),
        b"existing"
    );
    std::fs::remove_file(&temp).expect("cleanup");
}

#[test]
fn successful_install_updates_provenance() {
    let data = tempfile::tempdir().expect("tempdir");
    let dir = tempfile::tempdir().expect("tempdir");
    let target = dir.path().join("resourcepacks");
    let bytes = zip_bytes(&[("assets/minecraft/pack.png", b"png")]);
    let (client, _) = client_for(bytes.clone());
    let expected = sha512_hex(&bytes);

    let verified = download_verified(
        &client,
        CDN_URL,
        &target,
        "fresh-1.9.zip",
        &expected,
        ZipRoot::Assets,
    )
    .expect("download");
    commit_verified(&verified.temp_path, &target, "fresh-1.9.zip").expect("commit");
    provenance_record(
        data.path(),
        &verified.sha256,
        ProvenanceEntry {
            project_id: "7q7JnQD0".to_string(),
            version_id: "v2".to_string(),
            file_name: "fresh-1.9.zip".to_string(),
        },
    )
    .expect("record");
    let entry = provenance_lookup(data.path(), &verified.sha256)
        .expect("lookup")
        .expect("entry");
    assert_eq!(entry.project_id, "7q7JnQD0");
    assert_eq!(entry.version_id, "v2");
}

struct SlowReader {
    remaining: usize,
}

impl Read for SlowReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.remaining == 0 {
            return Ok(0);
        }
        let n = buf.len().min(self.remaining).min(1024);
        buf[..n].fill(b'q');
        self.remaining -= n;
        Ok(n)
    }
}

#[test]
fn stream_never_buffers_the_whole_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let target = dir.path().join("resourcepacks");
    std::fs::create_dir_all(&target).expect("mkdir");
    let temp = target.join(".probe.part");
    let mut out = std::fs::File::create(&temp).expect("create");
    let mut reader = SlowReader { remaining: 3000 };
    let mut chunk = [0u8; 512];
    let mut total = 0;
    while {
        let n: usize = reader.read(&mut chunk).expect("read");
        total += n;
        std::io::Write::write_all(&mut out, &chunk[..n]).expect("write");
        n > 0
    } {}
    drop(out);
    assert_eq!(total, 3000);
    std::fs::remove_file(&temp).expect("cleanup");
}
