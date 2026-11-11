#![cfg(target_os = "linux")]

use std::io::{Cursor, Write};
use std::path::Path;
use std::process::Command;
use std::time::{Duration, SystemTime};

use mine_utils_core::{
    download_verified, ModrinthClient, ModrinthError, RawResponse, Transport, ZipRoot,
};
use sha2::{Digest, Sha512};

struct Cdn(Vec<u8>);

impl Transport for Cdn {
    fn request(&self, _: &str) -> Result<RawResponse, ModrinthError> {
        Ok(RawResponse {
            status: 200,
            headers: vec![],
            body: Box::new(Cursor::new(self.0.clone())),
        })
    }
}

fn pack() -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    zip.start_file("assets/pack.png", zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.write_all(b"complete pack").unwrap();
    zip.finish().unwrap().into_inner()
}

#[test]
fn filesystem_download_helper() {
    let Some(dir) = std::env::var_os("DOWNLOAD_TEST_DIR") else {
        return;
    };
    let dir = Path::new(&dir);
    let bytes = pack();
    let result = download_verified(
        &ModrinthClient::without_sleep(Cdn(bytes.clone())),
        "https://cdn.modrinth.com/data/test/pack.zip",
        dir,
        "pack.zip",
        &hex::encode(Sha512::digest(&bytes)),
        ZipRoot::Assets,
    );
    if std::env::var_os("DOWNLOAD_TEST_EXPECT_IO").is_some() {
        assert_eq!(result.unwrap_err().code(), "download_io");
        assert_eq!(std::fs::read_dir(dir).unwrap().count(), 1);
        return;
    }
    let dest = result.unwrap().commit(dir, "pack.zip").unwrap();
    assert_eq!(std::fs::read(dest).unwrap(), bytes);
    assert_eq!(
        std::fs::read(dir.join(".stale.zip.mine-utils-download-4294967295-1-0.tmp")).unwrap(),
        b"orphan"
    );
    assert_eq!(std::fs::read_dir(dir).unwrap().count(), 2);
}

fn shim(dir: &Path) -> std::path::PathBuf {
    let output = dir.join("download_fs.so");
    let result = Command::new("cc")
        .args(["-shared", "-fPIC", "-Wall", "-Wextra", "-Werror"])
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/support/download_fs.c"
        ))
        .args(["-o"])
        .arg(&output)
        .arg("-ldl")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    output
}

fn check_download(shim: &Path, variables: &[(&str, String)]) {
    let dir = tempfile::tempdir().unwrap();
    let stale = dir
        .path()
        .join(".stale.zip.mine-utils-download-4294967295-1-0.tmp");
    std::fs::write(&stale, b"orphan").unwrap();
    std::fs::File::options()
        .write(true)
        .open(&stale)
        .unwrap()
        .set_modified(SystemTime::now() - Duration::from_secs(7200))
        .unwrap();
    let result = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "filesystem_download_helper", "--nocapture"])
        .env("LD_PRELOAD", shim)
        .env("DOWNLOAD_TEST_DIR", dir.path())
        .envs(variables.iter().cloned())
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{variables:?}\n{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn downloads_succeed_when_owner_locks_are_unsupported() {
    let dir = tempfile::tempdir().unwrap();
    let shim = shim(dir.path());
    for error in [
        libc::EOPNOTSUPP,
        libc::ENOTSUP,
        libc::ENOLCK,
        libc::ENOSYS,
        libc::EINVAL,
    ] {
        check_download(&shim, &[("DOWNLOAD_TEST_LOCK_ERROR", error.to_string())]);
    }
}

#[test]
fn sweeps_skip_remote_unknown_and_failed_filesystem_probes() {
    let dir = tempfile::tempdir().unwrap();
    let shim = shim(dir.path());
    for fs_type in [
        "0x65735546",
        "0x6969",
        "0xff534d42",
        "0xfe534d42",
        "0x12345678",
        "-1",
    ] {
        check_download(
            &shim,
            &[
                ("DOWNLOAD_TEST_FS_TYPE", fs_type.to_string()),
                ("DOWNLOAD_TEST_LOCAL_LOCKS", "1".to_string()),
            ],
        );
    }
}

#[test]
fn other_owner_lock_errors_still_report_io_and_clean_up() {
    let dir = tempfile::tempdir().unwrap();
    let shim = shim(dir.path());
    check_download(
        &shim,
        &[
            ("DOWNLOAD_TEST_LOCK_ERROR", libc::EIO.to_string()),
            ("DOWNLOAD_TEST_EXPECT_IO", "1".to_string()),
        ],
    );
}
