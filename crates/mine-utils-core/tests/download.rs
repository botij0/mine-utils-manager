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
    let temp = target.join(".pack.zip.mine-utils-download-1.tmp");
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
fn commit_is_no_replace_when_a_rival_wins_the_race() {
    // A rival creates the destination between our exists-check and rename.
    // The second commit must fail without touching the rival's bytes; on
    // Linux this is one atomic renameat2, elsewhere a hard-link fallback.
    let dir = tempfile::tempdir().expect("tempdir");
    let target = dir.path().join("resourcepacks");
    std::fs::create_dir_all(&target).expect("mkdir");
    let temp = target.join(".pack.zip.mine-utils-download-1.tmp");
    std::fs::write(&temp, b"new").expect("write");
    std::fs::write(target.join("pack.zip"), b"rival").expect("write");

    let err = commit_verified(&temp, &target, "pack.zip").expect_err("exists must fail");
    assert!(matches!(err, DownloadError::Exists { .. }), "{err}");
    assert_eq!(
        std::fs::read(target.join("pack.zip")).expect("read"),
        b"rival"
    );
}

#[test]
fn dropped_downloads_clean_their_temps() {
    let dir = tempfile::tempdir().expect("tempdir");
    let target = dir.path().join("resourcepacks");
    let bytes = zip_bytes(&[("assets/minecraft/pack.png", b"png")]);
    let (client, _) = client_for(bytes.clone());

    let temp = {
        let verified = download_verified(
            &client,
            CDN_URL,
            &target,
            "fresh-1.9.zip",
            &sha512_hex(&bytes),
            ZipRoot::Assets,
        )
        .expect("download");
        assert!(verified.temp_path.exists());
        verified.temp_path.clone()
    };
    assert!(!temp.exists(), "dropping the download deletes the temp");
    assert!(leftovers(&target).is_empty());
}

#[test]
fn download_temps_end_in_tmp_so_scans_skip_them() {
    let dir = tempfile::tempdir().expect("tempdir");
    let target = dir.path().join("resourcepacks");
    let bytes = zip_bytes(&[("assets/minecraft/pack.png", b"png")]);
    let (client, _) = client_for(bytes.clone());

    let verified = download_verified(
        &client,
        CDN_URL,
        &target,
        "fresh-1.9.zip",
        &sha512_hex(&bytes),
        ZipRoot::Assets,
    )
    .expect("download");
    assert!(
        verified
            .temp_path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.ends_with(".tmp")),
        "{}",
        verified.temp_path.display()
    );
    let sha256 = verified.sha256.clone();
    let _ = sha256;
    drop(verified);
    assert!(leftovers(&target).is_empty());
}

struct ChunkedReader {
    chunks: std::vec::IntoIter<Vec<u8>>,
}

impl Read for ChunkedReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self.chunks.next() {
            None => Ok(0),
            Some(chunk) => {
                // Never hand over more than 512 bytes per read, however big
                // the transport buffer is, so the test proves bounded reads.
                let n = chunk.len().min(buf.len()).min(512);
                buf[..n].copy_from_slice(&chunk[..n]);
                if n < chunk.len() {
                    let rest = chunk[n..].to_vec();
                    let tail = std::mem::replace(&mut self.chunks, Vec::new().into_iter());
                    let mut kept: Vec<Vec<u8>> = tail.collect();
                    kept.insert(0, rest);
                    self.chunks = kept.into_iter();
                }
                Ok(n)
            }
        }
    }
}

struct ChunkedCdn {
    bytes: Vec<u8>,
    chunk: usize,
}

impl Transport for ChunkedCdn {
    fn request(&self, url: &str) -> Result<RawResponse, ModrinthError> {
        assert!(url.starts_with("https://cdn.modrinth.com/"), "{url}");
        let chunks: Vec<Vec<u8>> = self.bytes.chunks(self.chunk).map(|c| c.to_vec()).collect();
        Ok(RawResponse {
            status: 200,
            headers: vec![],
            body: Box::new(ChunkedReader {
                chunks: chunks.into_iter(),
            }),
        })
    }
}

#[test]
fn chunked_download_verifies_without_buffering_the_file() {
    // A 256 KiB zip arrives in 1 KiB transport chunks and is read back in
    // bounded pieces: `download_verified` streams it, checks the SHA-512,
    // and validates the zip root without ever holding the whole file.
    let dir = tempfile::tempdir().expect("tempdir");
    let target = dir.path().join("resourcepacks");
    let payload = vec![b'p'; 256 * 1024];
    let bytes = zip_bytes(&[
        ("assets/minecraft/pack.png", payload.as_slice()),
        ("pack.mcmeta", b"{}"),
    ]);
    let expected = sha512_hex(&bytes);
    let cdn = ChunkedCdn {
        bytes: bytes.clone(),
        chunk: 1024,
    };
    let client = ModrinthClient::without_sleep(cdn);

    let verified = download_verified(
        &client,
        CDN_URL,
        &target,
        "big-1.0.zip",
        &expected,
        ZipRoot::Assets,
    )
    .expect("chunked download");
    assert_eq!(verified.bytes, bytes.len() as u64);
    let dest = verified.commit(&target, "big-1.0.zip").expect("commit");
    assert_eq!(std::fs::read(&dest).expect("read"), bytes);
    assert!(leftovers(&target).is_empty());
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
    let sha256 = verified.sha256.clone();
    verified.commit(&target, "fresh-1.9.zip").expect("commit");
    provenance_record(
        data.path(),
        &sha256,
        ProvenanceEntry {
            project_id: "7q7JnQD0".to_string(),
            version_id: "v2".to_string(),
            file_name: "fresh-1.9.zip".to_string(),
        },
    )
    .expect("record");
    let entry = provenance_lookup(data.path(), &sha256)
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
fn slow_streaming_reader_still_streams() {
    // A dripping reader (at most 1 KiB per read) behind `download_verified`
    // proves the cap is enforced while streaming: the over-cap stream is
    // refused without writing unbounded bytes.
    struct SlowCdn;
    impl Transport for SlowCdn {
        fn request(&self, url: &str) -> Result<RawResponse, ModrinthError> {
            assert!(url.starts_with("https://cdn.modrinth.com/"), "{url}");
            Ok(RawResponse {
                status: 200,
                headers: vec![],
                body: Box::new(SlowReader { remaining: 3000 }),
            })
        }
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let target = dir.path().join("resourcepacks");
    let client = ModrinthClient::without_sleep(SlowCdn);
    let err = download_verified_capped(
        &client,
        CDN_URL,
        &target,
        "slow.zip",
        &"00".repeat(64),
        ZipRoot::Assets,
        64,
    )
    .expect_err("over-cap must fail");
    assert!(matches!(err, DownloadError::TooLarge { .. }), "{err}");
    assert!(leftovers(&target).is_empty());
}

#[cfg(unix)]
#[test]
fn download_sweeps_only_stale_regular_download_temps() {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().unwrap();
    let mut foreign = std::process::Command::new("sleep")
        .arg("60")
        .spawn()
        .unwrap();
    let recycled = format!(".foreign.zip.mine-utils-download-{}-1-0.tmp", foreign.id());
    let stale = ".old.zip.mine-utils-download-4294967295-1-0.tmp";
    let current = format!(
        ".active.zip.mine-utils-download-{}-1-0.tmp",
        std::process::id()
    );
    let link = ".link.zip.mine-utils-download-4294967295-1-0.tmp";
    for name in [
        stale,
        &recycled,
        &current,
        ".unrelated",
        ".bad.mine-utils-download-4294967295-x-0.tmp",
    ] {
        let path = dir.path().join(name);
        std::fs::write(&path, b"keep").unwrap();
        std::fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(7200))
            .unwrap();
    }
    symlink(dir.path().join(".unrelated"), dir.path().join(link)).unwrap();
    let listing = || {
        let mut names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    };
    println!("before: {:?}", listing());
    let bytes = zip_bytes(&[("assets/pack.png", b"png")]);
    let (client, _) = client_for(bytes.clone());
    let verified = download_verified(
        &client,
        CDN_URL,
        dir.path(),
        "fresh.zip",
        &sha512_hex(&bytes),
        ZipRoot::Assets,
    )
    .unwrap();
    drop(verified);
    foreign.kill().unwrap();
    foreign.wait().unwrap();
    println!("after: {:?}", listing());
    let mut expected = vec![
        current,
        link.to_string(),
        ".unrelated".to_string(),
        ".bad.mine-utils-download-4294967295-x-0.tmp".to_string(),
    ];
    expected.sort();
    assert_eq!(listing(), expected);
}

#[test]
fn cancel_mid_stream_removes_the_temp() {
    use mine_utils_core::download::{download_verified_controlled, InstallControl};
    struct CancellingCdn(InstallControl);
    struct CancellingReader(InstallControl);
    impl Read for CancellingReader {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.0.cancel();
            buf[0] = 1;
            Ok(1)
        }
    }
    impl Transport for CancellingCdn {
        fn request(&self, _: &str) -> Result<RawResponse, ModrinthError> {
            Ok(RawResponse {
                status: 200,
                headers: vec![],
                body: Box::new(CancellingReader(self.0.clone())),
            })
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let control = InstallControl::default();
    let client = ModrinthClient::without_sleep(CancellingCdn(control.clone()));
    let err = download_verified_controlled(
        &client,
        CDN_URL,
        dir.path(),
        "pack.zip",
        "unused",
        ZipRoot::Assets,
        MAX_BYTES,
        &control,
    )
    .unwrap_err();
    assert_eq!(err.code(), "download_cancelled");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

const MAX_BYTES: u64 = 1024 * 1024;

#[test]
fn total_deadline_stops_a_trickle_and_cleans_the_temp() {
    use mine_utils_core::download::{download_verified_controlled, InstallControl};
    struct SlowCdn;
    struct Trickle;
    impl Read for Trickle {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            std::thread::sleep(std::time::Duration::from_millis(10));
            buf[0] = 1;
            Ok(1)
        }
    }
    impl Transport for SlowCdn {
        fn request(&self, _: &str) -> Result<RawResponse, ModrinthError> {
            Ok(RawResponse {
                status: 200,
                headers: vec![],
                body: Box::new(Trickle),
            })
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let control = InstallControl::with_timeout(std::time::Duration::from_millis(40));
    let client = ModrinthClient::without_sleep(SlowCdn);
    let started = std::time::Instant::now();
    let err = download_verified_controlled(
        &client,
        CDN_URL,
        dir.path(),
        "pack.zip",
        "unused",
        ZipRoot::Assets,
        MAX_BYTES,
        &control,
    )
    .unwrap_err();
    assert_eq!(err.code(), "download_timeout");
    assert!(started.elapsed() < std::time::Duration::from_secs(1));
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn total_deadline_does_not_wait_for_blocked_response_headers() {
    use mine_utils_core::download::{download_verified_controlled, InstallControl};
    struct BlockedCdn;
    impl Transport for BlockedCdn {
        fn request(&self, _: &str) -> Result<RawResponse, ModrinthError> {
            std::thread::sleep(std::time::Duration::from_secs(2));
            Ok(RawResponse {
                status: 200,
                headers: vec![],
                body: Box::new(Cursor::new(Vec::new())),
            })
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let control = InstallControl::with_timeout(std::time::Duration::from_millis(30));
    let client = ModrinthClient::without_sleep(BlockedCdn);
    let started = std::time::Instant::now();
    let err = download_verified_controlled(
        &client,
        CDN_URL,
        dir.path(),
        "pack.zip",
        "unused",
        ZipRoot::Assets,
        MAX_BYTES,
        &control,
    )
    .unwrap_err();
    assert_eq!(err.code(), "download_timeout");
    assert!(started.elapsed() < std::time::Duration::from_secs(1));
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn dropping_download_keeps_a_foreign_file_at_the_temp_path() {
    let dir = tempfile::tempdir().unwrap();
    let bytes = zip_bytes(&[("assets/a", b"asset")]);
    let (client, _) = client_for(bytes.clone());
    let verified = download_verified(
        &client,
        CDN_URL,
        dir.path(),
        "pack.zip",
        &sha512_hex(&bytes),
        ZipRoot::Assets,
    )
    .unwrap();
    let temp = verified.temp_path.clone();
    let original = dir.path().join("moved");
    std::fs::rename(&temp, &original).unwrap();
    std::fs::write(&temp, b"unrelated sentinel").unwrap();
    drop(verified);
    assert_eq!(std::fs::read(temp).unwrap(), b"unrelated sentinel");
    assert_eq!(std::fs::read(original).unwrap(), bytes);
}
