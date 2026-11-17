use std::io::{self, Read};
use std::path::Path;

pub(crate) fn read_capped(path: &Path, cap: u64) -> io::Result<Vec<u8>> {
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() {
        return Err(not_regular());
    }
    let mut bytes = Vec::new();
    open_regular(path)?.take(cap + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > cap {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "file is too large",
        ));
    }
    Ok(bytes)
}

pub(crate) fn open_regular(path: &Path) -> io::Result<std::fs::File> {
    #[cfg(test)]
    before_open(path);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)
            .map_err(|err| {
                if err.raw_os_error() == Some(libc::ELOOP) {
                    io::Error::new(io::ErrorKind::InvalidData, err)
                } else {
                    err
                }
            })?;
        if !file.metadata()?.file_type().is_file() {
            return Err(not_regular());
        }
        Ok(file)
    }
    #[cfg(not(unix))]
    {
        if !std::fs::symlink_metadata(path)?.file_type().is_file() {
            return Err(not_regular());
        }
        std::fs::File::open(path)
    }
}

fn not_regular() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "not a regular file")
}

#[cfg(test)]
thread_local! {
    static BEFORE_OPEN: std::cell::RefCell<Option<Box<dyn FnOnce(&Path)>>> =
        const { std::cell::RefCell::new(None) };
}

/// Runs `hook` once, on this thread, right before the next `open_regular`
/// opens its path, so a test can replace the entry after any earlier check.
#[cfg(test)]
pub(crate) fn swap_before_next_open(hook: impl FnOnce(&Path) + 'static) {
    BEFORE_OPEN.set(Some(Box::new(hook)));
}

#[cfg(test)]
fn before_open(path: &Path) {
    if let Some(hook) = BEFORE_OPEN.take() {
        hook(path);
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::open_regular;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::symlink;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn opened_handle_rejects_fifo_and_symlink_without_blocking() {
        let dir = tempfile::tempdir().unwrap();
        let fifo = dir.path().join("fifo");
        let c_path = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) }, 0);
        let target = dir.path().join("target");
        std::fs::write(&target, b"external").unwrap();
        let link = dir.path().join("link");
        symlink(&target, &link).unwrap();
        for path in [fifo, link] {
            let (tx, rx) = mpsc::channel();
            std::thread::spawn(move || {
                let _ = tx.send(open_regular(&path).is_err());
            });
            assert_eq!(rx.recv_timeout(Duration::from_secs(2)), Ok(true));
        }
    }
}
