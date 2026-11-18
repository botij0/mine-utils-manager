#![cfg(unix)]

use std::io::{BufRead, Cursor, Read, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, SystemTime};

use mine_utils_core::{
    download_verified, ModrinthClient, ModrinthError, RawResponse, Transport, ZipRoot,
};
use sha2::{Digest, Sha512};

const URL: &str = "https://cdn.modrinth.com/data/test/pack.zip";

fn pack() -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    zip.start_file("assets/pack.png", zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.write_all(b"complete pack").unwrap();
    zip.finish().unwrap().into_inner()
}

struct Cdn;

impl Transport for Cdn {
    fn request(&self, _: &str) -> Result<RawResponse, ModrinthError> {
        Ok(RawResponse {
            status: 200,
            headers: vec![],
            body: Box::new(PausingBody(Cursor::new(pack()))),
        })
    }
}

struct PausingBody(Cursor<Vec<u8>>);

impl Read for PausingBody {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.0.position() == self.0.get_ref().len() as u64
            && std::env::var("DOWNLOAD_HOLD_PHASE").as_deref() == Ok("streaming")
        {
            pause_owner();
        }
        self.0.read(buf)
    }
}

fn pause_owner() {
    println!("DOWNLOAD_PAUSED");
    std::io::stdout().flush().unwrap();
    let mut resume = String::new();
    assert_eq!(std::io::stdin().read_line(&mut resume).unwrap(), 7);
    assert_eq!(resume, "resume\n");
}

fn download(dir: &Path, name: &str) -> mine_utils_core::VerifiedDownload {
    download_verified(
        &ModrinthClient::without_sleep(Cdn),
        URL,
        dir,
        name,
        &hex::encode(Sha512::digest(pack())),
        ZipRoot::Assets,
    )
    .unwrap()
}

fn age(path: &Path) {
    std::fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(SystemTime::now() - Duration::from_secs(7200))
        .unwrap();
}

fn listing(dir: &Path) -> Vec<String> {
    let mut names: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

#[test]
fn sweep_keeps_recent_temps_and_non_regular_or_malformed_entries() {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().unwrap();
    let stale = ".stale.zip.mine-utils-download-4294967295-1-0.tmp";
    let recent = ".recent.zip.mine-utils-download-4294967295-1-0.tmp";
    let current = format!(
        ".current.zip.mine-utils-download-{}-1-0.tmp",
        std::process::id()
    );
    let link = ".link.zip.mine-utils-download-4294967295-1-0.tmp";
    let directory = ".dir.zip.mine-utils-download-4294967295-1-0.tmp";
    let malformed = [
        ".unrelated",
        ".bad.mine-utils-download-4294967295-x-0.tmp",
        ".bad.mine-utils-download-4294967295-1-0-extra.tmp",
        "bad.mine-utils-download-4294967295-1-0.tmp",
        ".mine-utils-download-4294967295-1-0.tmp",
    ];
    for name in [stale, recent, &current].into_iter().chain(malformed) {
        let path = dir.path().join(name);
        std::fs::write(&path, b"keep").unwrap();
        if name != recent {
            age(&path);
        }
    }
    symlink(dir.path().join(stale), dir.path().join(link)).unwrap();
    std::fs::create_dir(dir.path().join(directory)).unwrap();
    println!("before: {:?}", listing(dir.path()));
    let dest = download(dir.path(), "fresh.zip")
        .commit(dir.path(), "fresh.zip")
        .unwrap();
    println!("after: {:?}", listing(dir.path()));
    assert_eq!(std::fs::read(dest).unwrap(), pack());
    let mut expected: Vec<String> = [recent, &current, link, directory, "fresh.zip"]
        .into_iter()
        .chain(malformed)
        .map(str::to_string)
        .collect();
    expected.sort();
    assert_eq!(listing(dir.path()), expected);
    assert!(std::fs::symlink_metadata(dir.path().join(link))
        .unwrap()
        .is_symlink());
}

#[test]
fn download_temp_owner_helper() {
    let Some(dir) = std::env::var_os("DOWNLOAD_OWNER_DIR") else {
        return;
    };
    let dir = Path::new(&dir);
    let verified = download(dir, "owner.zip");
    if std::env::var("DOWNLOAD_HOLD_PHASE").as_deref() == Ok("verified") {
        pause_owner();
    }
    let dest = verified.commit(dir, "owner.zip").unwrap();
    assert_eq!(std::fs::read(dest).unwrap(), pack());
    println!("DOWNLOAD_RESUMED_AND_COMMITTED");
}

struct Owner(Child);

impl Drop for Owner {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn live_owner_survives(mode: &str, phase: &str) {
    let dir = tempfile::tempdir().unwrap();
    let executable_dir = tempfile::tempdir().unwrap();
    let executable = executable_dir.path().join("copied-downloader");
    std::fs::copy(std::env::current_exe().unwrap(), &executable).unwrap();
    let mut command = if mode == "namespace" {
        let mut command = Command::new("unshare");
        command.args([
            "--user",
            "--map-root-user",
            "--pid",
            "--fork",
            "--kill-child",
            "--mount-proc",
        ]);
        command.args([
            "sh",
            "-c",
            "exec 3<&0; echo 4000000 > /proc/sys/kernel/ns_last_pid || exit 1; \"$@\" <&3 & wait $!",
            "owner",
        ]);
        command.arg(&executable);
        command
    } else {
        Command::new(&executable)
    };
    let mut owner = Owner(
        command
            .args(["--exact", "download_temp_owner_helper", "--nocapture"])
            .env("DOWNLOAD_OWNER_DIR", dir.path())
            .env("DOWNLOAD_HOLD_PHASE", phase)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let mut output = std::io::BufReader::new(owner.0.stdout.take().unwrap());
    loop {
        let mut line = String::new();
        assert_ne!(
            output.read_line(&mut line).unwrap(),
            0,
            "owner exited before pausing"
        );
        if line.trim() == "DOWNLOAD_PAUSED" {
            break;
        }
    }
    if mode == "replaced" {
        std::fs::rename(&executable, executable_dir.path().join("old-downloader")).unwrap();
        std::fs::copy(std::env::current_exe().unwrap(), &executable).unwrap();
    } else if mode == "unlinked" {
        std::fs::remove_file(&executable).unwrap();
    }
    let live = listing(dir.path()).pop().unwrap();
    if mode == "namespace" {
        let pid = live
            .rsplit_once(".mine-utils-download-")
            .unwrap()
            .1
            .split('-')
            .next()
            .unwrap();
        assert!(
            !Path::new(&format!("/proc/{pid}")).exists(),
            "namespace PID must be absent in sweeper namespace"
        );
        println!("namespace owner PID {pid} is absent from sweeper /proc");
    }
    age(&dir.path().join(&live));
    let stale = dir
        .path()
        .join(".stale.zip.mine-utils-download-4294967295-1-0.tmp");
    std::fs::write(&stale, b"orphan").unwrap();
    age(&stale);
    println!("{mode}/{phase} before: {:?}", listing(dir.path()));
    let verified = download(dir.path(), "sweeper.zip");
    drop(verified);
    println!("{mode}/{phase} after: {:?}", listing(dir.path()));
    assert_eq!(listing(dir.path()), vec![live]);
    owner
        .0
        .stdin
        .take()
        .unwrap()
        .write_all(b"resume\n")
        .unwrap();
    let mut rest = String::new();
    output.read_to_string(&mut rest).unwrap();
    assert!(owner.0.wait().unwrap().success(), "{rest}");
    assert!(rest.contains("DOWNLOAD_RESUMED_AND_COMMITTED"), "{rest}");
    assert_eq!(std::fs::read(dir.path().join("owner.zip")).unwrap(), pack());
    println!("{mode}/{phase}: live download resumed and committed exact archive bytes");
}

#[test]
fn sweep_keeps_live_downloads_across_executables_and_pid_namespaces() {
    for mode in ["copied", "replaced", "unlinked"] {
        for phase in ["streaming", "verified"] {
            live_owner_survives(mode, phase);
        }
    }

    #[cfg(target_os = "linux")]
    {
        let available = Command::new("unshare")
            .args([
                "--user",
                "--map-root-user",
                "--pid",
                "--fork",
                "--kill-child",
                "--mount-proc",
                "sh",
                "-c",
                "echo 4000000 > /proc/sys/kernel/ns_last_pid",
            ])
            .output();
        if !available.is_ok_and(|output| output.status.success()) {
            eprintln!("SKIP: unprivileged PID namespaces unavailable");
            return;
        }
        live_owner_survives("namespace", "streaming");
        live_owner_survives("namespace", "verified");
    }
}

#[test]
fn sweep_never_follows_a_symlinked_target_directory() {
    let root = tempfile::tempdir().unwrap();
    let outside = root.path().join("outside");
    std::fs::create_dir(&outside).unwrap();
    let foreign = outside.join(".foreign.zip.mine-utils-download-4294967295-1-0.tmp");
    std::fs::write(&foreign, b"foreign sentinel").unwrap();
    age(&foreign);
    let target = root.path().join("resourcepacks");
    std::os::unix::fs::symlink(&outside, &target).unwrap();
    let result = download_verified(
        &ModrinthClient::without_sleep(Cdn),
        URL,
        &target,
        "pack.zip",
        &hex::encode(Sha512::digest(pack())),
        ZipRoot::Assets,
    );
    assert_eq!(result.unwrap_err().code(), "download_io");
    assert_eq!(std::fs::read(&foreign).unwrap(), b"foreign sentinel");
}
