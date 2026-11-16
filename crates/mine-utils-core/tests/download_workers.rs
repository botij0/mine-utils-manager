use mine_utils_core::download::{download_verified_controlled, InstallControl, ZipRoot};
use mine_utils_core::{ModrinthClient, ModrinthError, RawResponse, Transport};
use std::io::Read;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

#[derive(Default)]
struct Readers {
    released: bool,
    live: usize,
    peak: usize,
}

type SharedReaders = Arc<(Mutex<Readers>, Condvar)>;

struct BlockedReader(SharedReaders);

impl Read for BlockedReader {
    fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
        let (lock, wake) = &*self.0;
        let mut readers = lock.lock().unwrap();
        readers.live += 1;
        readers.peak = readers.peak.max(readers.live);
        while !readers.released {
            readers = wake.wait(readers).unwrap();
        }
        readers.live -= 1;
        wake.notify_all();
        Ok(0)
    }
}

struct BlockedCdn(SharedReaders);

impl Transport for BlockedCdn {
    fn request(&self, _: &str) -> Result<RawResponse, ModrinthError> {
        Ok(RawResponse {
            status: 200,
            headers: vec![],
            body: Box::new(BlockedReader(Arc::clone(&self.0))),
        })
    }
}

#[test]
fn sequential_cancels_bound_live_blocked_readers() {
    let readers = SharedReaders::default();
    let client = ModrinthClient::without_sleep(BlockedCdn(Arc::clone(&readers)));
    let target = tempfile::tempdir().unwrap();
    for _ in 0..24 {
        let control = InstallControl::default();
        let cancel = control.clone();
        let canceller = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(40));
            cancel.cancel();
        });
        let result = download_verified_controlled(
            &client,
            "https://cdn.modrinth.com/pack.zip",
            target.path(),
            "pack.zip",
            "unused",
            ZipRoot::Assets,
            1024,
            &control,
        );
        canceller.join().unwrap();
        assert_eq!(result.unwrap_err().code(), "download_cancelled");
    }
    let (lock, wake) = &*readers;
    let mut state = lock.lock().unwrap();
    let peak = state.peak;
    let live = state.live;
    state.released = true;
    wake.notify_all();
    let (state, timeout) = wake
        .wait_timeout_while(state, Duration::from_secs(2), |state| state.live != 0)
        .unwrap();
    println!(
        "24 cancels: live blocked readers={live}, peak={peak}, after release={}",
        state.live
    );
    assert!(!timeout.timed_out(), "all readers must exit after release");
    assert!(peak > 0, "the transport must actually block");
    assert!(
        peak <= 8,
        "at most 8 network reader workers, observed {peak}"
    );
    assert_eq!(std::fs::read_dir(target.path()).unwrap().count(), 0);
}
