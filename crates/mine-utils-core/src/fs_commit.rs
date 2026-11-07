use std::path::Path;

/// Move `temp` to `dest` without replacing an existing file. On Linux the
/// rename is a single `renameat2` with `RENAME_NOREPLACE`, so the
/// exists-then-rename race cannot happen.
pub(crate) fn atomic_no_replace(temp: &Path, dest: &Path) -> std::io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        // renameat2 with AT_FDCWD renames by path. RENAME_NOREPLACE makes
        // an existing destination fail with EEXIST instead of replacing it.
        const RENAME_NOREPLACE: libc::c_uint = 1;
        let temp_c = std::ffi::CString::new(temp.as_os_str().as_encoded_bytes());
        let dest_c = std::ffi::CString::new(dest.as_os_str().as_encoded_bytes());
        if let (Ok(temp_c), Ok(dest_c)) = (temp_c, dest_c) {
            // SAFETY: plain integers and NUL-terminated path pointers; no
            // memory is retained by the kernel after the call returns.
            let rc = unsafe {
                libc::renameat2(
                    libc::AT_FDCWD,
                    temp_c.as_ptr(),
                    libc::AT_FDCWD,
                    dest_c.as_ptr(),
                    RENAME_NOREPLACE,
                )
            };
            if rc == 0 {
                return Ok(());
            }
            let err = std::io::Error::last_os_error();
            if !rename_noreplace_unsupported(&err) {
                return Err(err);
            }
        }
    }
    no_replace_fallback(temp, dest)
}

/// A hard link (same directory, so same filesystem) is the atomic
/// no-replace, with an exists-checked rename as the last resort.
pub(crate) fn no_replace_fallback(temp: &Path, dest: &Path) -> std::io::Result<()> {
    match std::fs::hard_link(temp, dest) {
        Ok(()) => {
            if std::fs::remove_file(temp).is_err() {
                let _ = std::fs::remove_file(temp);
            }
            Ok(())
        }
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => Err(err),
        Err(_) => checked_rename(temp, dest),
    }
}

fn checked_rename(temp: &Path, dest: &Path) -> std::io::Result<()> {
    if dest.exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "destination exists",
        ));
    }
    std::fs::rename(temp, dest)
}

/// NFS, sshfs, and FUSE filesystems without rename flags answer EINVAL; a
/// seccomp filter answers ENOSYS, EPERM, or EACCES.
#[cfg(target_os = "linux")]
fn rename_noreplace_unsupported(err: &std::io::Error) -> bool {
    matches!(
        err.raw_os_error(),
        Some(libc::EINVAL | libc::ENOSYS | libc::EPERM | libc::EACCES)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    #[test]
    fn renameat2_errors_that_mean_unsupported_take_the_link_fallback() {
        let decisions: Vec<(&str, bool)> = [
            ("EINVAL", libc::EINVAL),
            ("ENOSYS", libc::ENOSYS),
            ("EPERM", libc::EPERM),
            ("EACCES", libc::EACCES),
            ("EEXIST", libc::EEXIST),
        ]
        .into_iter()
        .map(|(name, errno)| {
            let err = std::io::Error::from_raw_os_error(errno);
            (name, rename_noreplace_unsupported(&err))
        })
        .collect();
        assert_eq!(
            decisions,
            vec![
                ("EINVAL", true),
                ("ENOSYS", true),
                ("EPERM", true),
                ("EACCES", true),
                ("EEXIST", false),
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_link_fallback_commits_even_when_the_temp_will_not_unlink() {
        use std::os::unix::fs::PermissionsExt as _;
        let root = tempfile::tempdir().unwrap();
        let locked = root.path().join("locked");
        std::fs::create_dir(&locked).unwrap();
        let temp = locked.join(".Pack.zip.tmp");
        std::fs::write(&temp, b"pack").unwrap();
        let dest = root.path().join("Pack.zip");
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).unwrap();

        let result = no_replace_fallback(&temp, &dest).map_err(|err| err.kind());
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();

        assert_eq!(result, Ok(()));
        assert_eq!(std::fs::read(&dest).unwrap(), b"pack");
    }

    #[cfg(unix)]
    #[test]
    fn the_checked_rename_never_replaces_a_rival() {
        type MakeRival = fn(&Path);
        let rivals: [(&str, MakeRival); 3] = [
            ("dangling symlink", |path| {
                std::os::unix::fs::symlink("missing", path).unwrap()
            }),
            ("empty dir", |path| std::fs::create_dir(path).unwrap()),
            ("regular file", |path| {
                std::fs::write(path, b"rival").unwrap()
            }),
        ];
        let mut seen = Vec::new();
        for temp_kind in ["dir", "file"] {
            for (label, make_rival) in rivals {
                let root = tempfile::tempdir().unwrap();
                let temp = root.path().join(".Pack.tmp");
                if temp_kind == "dir" {
                    std::fs::create_dir(&temp).unwrap();
                } else {
                    std::fs::write(&temp, b"pack").unwrap();
                }
                let dest = root.path().join("Pack");
                make_rival(&dest);

                let result = checked_rename(&temp, &dest).map_err(|err| err.kind());

                let entry = std::fs::symlink_metadata(&dest).unwrap();
                let left = if entry.is_symlink() {
                    format!(
                        "symlink to {}",
                        std::fs::read_link(&dest).unwrap().display()
                    )
                } else if entry.is_dir() {
                    format!(
                        "dir with {} entries",
                        std::fs::read_dir(&dest).unwrap().count()
                    )
                } else {
                    format!("file {}", std::fs::read_to_string(&dest).unwrap())
                };
                seen.push(format!(
                    "{temp_kind} temp, {label}: {result:?}; dest {left}; temp kept {}",
                    temp.exists()
                ));
            }
        }

        assert_eq!(
            seen,
            [
                "dir temp, dangling symlink: Err(AlreadyExists); dest symlink to missing; temp kept true",
                "dir temp, empty dir: Err(AlreadyExists); dest dir with 0 entries; temp kept true",
                "dir temp, regular file: Err(AlreadyExists); dest file rival; temp kept true",
                "file temp, dangling symlink: Err(AlreadyExists); dest symlink to missing; temp kept true",
                "file temp, empty dir: Err(AlreadyExists); dest dir with 0 entries; temp kept true",
                "file temp, regular file: Err(AlreadyExists); dest file rival; temp kept true",
            ]
        );
    }
}
