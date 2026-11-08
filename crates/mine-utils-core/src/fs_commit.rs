use std::io::ErrorKind;
use std::path::Path;

/// Move `temp` to `dest` without replacing an existing entry. On Linux the
/// rename is a single `renameat2` with `RENAME_NOREPLACE`, so on a
/// filesystem that supports the flag the exists-then-rename race cannot
/// happen.
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
/// no-replace for a file. A directory cannot be hard-linked, so it falls to
/// `checked_rename`, as does a file on a filesystem that refuses links.
pub(crate) fn no_replace_fallback(temp: &Path, dest: &Path) -> std::io::Result<()> {
    match std::fs::hard_link(temp, dest) {
        Ok(()) => {
            if std::fs::remove_file(temp).is_err() {
                let _ = std::fs::remove_file(temp);
            }
            Ok(())
        }
        Err(err) if err.kind() == ErrorKind::AlreadyExists => Err(err),
        Err(_) => checked_rename(temp, dest, |from, to| std::fs::rename(from, to)),
    }
}

/// A directory claims `dest` with `create_dir`, which fails for any existing
/// entry, then renames onto its own empty claim. A rival that removes the
/// claim and puts its own empty directory there is still replaced by that
/// rename, and no portable fix exists on this fallback. A crash before the
/// rename leaves the empty claim at `dest`. If the rename onto the claim fails
/// without naming a rival, as a plain SFTP rename does for any existing
/// target, a claim that is still ours is removed and the directory is moved
/// like a file. A file checks `dest` with `symlink_metadata` and then
/// renames, so a rival that lands between the two is replaced.
fn checked_rename(
    temp: &Path,
    dest: &Path,
    mut rename: impl FnMut(&Path, &Path) -> std::io::Result<()>,
) -> std::io::Result<()> {
    let rivals = if std::fs::symlink_metadata(temp)?.is_dir() {
        std::fs::create_dir(dest)?;
        let claim = std::fs::symlink_metadata(dest)?;
        let Err(err) = rename(temp, dest) else {
            return Ok(());
        };
        if !remove_claim(dest, &claim) || !plain_rename_refusal(&err) {
            return Err(taken_as_already_exists(err, DIR_RIVALS));
        }
        DIR_RIVALS
    } else {
        FILE_RIVALS
    };
    if std::fs::symlink_metadata(dest).is_ok() {
        return Err(std::io::Error::new(
            ErrorKind::AlreadyExists,
            "destination exists",
        ));
    }
    rename(temp, dest).map_err(|err| taken_as_already_exists(err, rivals))
}

/// `rename` reports a rival at `dest` with these errors instead of `EEXIST`.
/// A directory temp gets them for a non-directory or a directory with
/// entries, and a file temp gets them for a directory.
const DIR_RIVALS: &[ErrorKind] = &[ErrorKind::NotADirectory, ErrorKind::DirectoryNotEmpty];
const FILE_RIVALS: &[ErrorKind] = &[ErrorKind::IsADirectory];

fn taken_as_already_exists(err: std::io::Error, rivals: &[ErrorKind]) -> std::io::Error {
    if rivals.contains(&err.kind()) {
        std::io::Error::new(ErrorKind::AlreadyExists, err)
    } else {
        err
    }
}

/// An SFTP server without `posix-rename@openssh.com` gets a plain rename,
/// which refuses any existing target. sshfs reports that as `EPERM`, and
/// some servers as `EEXIST`.
fn plain_rename_refusal(err: &std::io::Error) -> bool {
    #[cfg(unix)]
    if err.raw_os_error() == Some(libc::EPERM) {
        return true;
    }
    err.kind() == ErrorKind::AlreadyExists
}

fn remove_claim(dest: &Path, claim: &std::fs::Metadata) -> bool {
    std::fs::symlink_metadata(dest).is_ok_and(|now| same_entry(&now, claim))
        && std::fs::remove_dir(dest).is_ok()
}

#[cfg(unix)]
fn same_entry(a: &std::fs::Metadata, b: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt as _;
    (a.dev(), a.ino()) == (b.dev(), b.ino())
}

/// Stable std has no file identity here, so the claim is taken as unchanged.
#[cfg(not(unix))]
fn same_entry(_: &std::fs::Metadata, _: &std::fs::Metadata) -> bool {
    true
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

    #[cfg(unix)]
    fn entry_summary(path: &Path) -> String {
        let Ok(entry) = std::fs::symlink_metadata(path) else {
            return "missing".to_string();
        };
        if entry.is_symlink() {
            format!("symlink to {}", std::fs::read_link(path).unwrap().display())
        } else if entry.is_dir() {
            format!(
                "dir with {} entries",
                std::fs::read_dir(path).unwrap().count()
            )
        } else {
            format!("file {}", std::fs::read_to_string(path).unwrap())
        }
    }

    #[cfg(unix)]
    fn first_rename_fails_with(errno: i32) -> impl FnMut(&Path, &Path) -> std::io::Result<()> {
        let mut failed = false;
        move |from, to| {
            if failed {
                return std::fs::rename(from, to);
            }
            failed = true;
            Err(std::io::Error::from_raw_os_error(errno))
        }
    }

    #[cfg(unix)]
    const RENAME_ERRNOS: [(&str, i32); 9] = [
        ("ENOTDIR", libc::ENOTDIR),
        ("EISDIR", libc::EISDIR),
        ("ENOTEMPTY", libc::ENOTEMPTY),
        ("EEXIST", libc::EEXIST),
        ("EPERM", libc::EPERM),
        ("EACCES", libc::EACCES),
        ("EXDEV", libc::EXDEV),
        ("ENOSPC", libc::ENOSPC),
        ("EIO", libc::EIO),
    ];

    #[cfg(unix)]
    #[test]
    fn a_folder_whose_rename_onto_the_claim_fails_retries_only_a_plain_rename_refusal() {
        let seen: Vec<String> = RENAME_ERRNOS
            .into_iter()
            .map(|(name, errno)| {
                let root = tempfile::tempdir().unwrap();
                let temp = root.path().join(".Pack.tmp");
                std::fs::create_dir(&temp).unwrap();
                std::fs::write(temp.join("pack.mcmeta"), b"{}").unwrap();
                let dest = root.path().join("Pack");

                let result = checked_rename(&temp, &dest, first_rename_fails_with(errno))
                    .map_err(|err| err.kind());

                format!(
                    "{name}: {result:?}; dest {}; temp kept {}",
                    entry_summary(&dest),
                    temp.exists()
                )
            })
            .collect();

        assert_eq!(
            seen,
            [
                "ENOTDIR: Err(AlreadyExists); dest missing; temp kept true",
                "EISDIR: Err(IsADirectory); dest missing; temp kept true",
                "ENOTEMPTY: Err(AlreadyExists); dest missing; temp kept true",
                "EEXIST: Ok(()); dest dir with 1 entries; temp kept false",
                "EPERM: Ok(()); dest dir with 1 entries; temp kept false",
                "EACCES: Err(PermissionDenied); dest missing; temp kept true",
                "EXDEV: Err(CrossesDevices); dest missing; temp kept true",
                "ENOSPC: Err(StorageFull); dest missing; temp kept true",
                "EIO: Err(Uncategorized); dest missing; temp kept true",
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_file_rename_reads_only_a_folder_rival_as_taken() {
        let seen: Vec<String> = RENAME_ERRNOS
            .into_iter()
            .map(|(name, errno)| {
                let root = tempfile::tempdir().unwrap();
                let temp = root.path().join(".Pack.zip.tmp");
                std::fs::write(&temp, b"pack").unwrap();
                let dest = root.path().join("Pack.zip");

                let result = checked_rename(&temp, &dest, first_rename_fails_with(errno))
                    .map_err(|err| err.kind());

                format!(
                    "{name}: {result:?}; dest {}; temp kept {}",
                    entry_summary(&dest),
                    temp.exists()
                )
            })
            .collect();

        assert_eq!(
            seen,
            [
                "ENOTDIR: Err(NotADirectory); dest missing; temp kept true",
                "EISDIR: Err(AlreadyExists); dest missing; temp kept true",
                "ENOTEMPTY: Err(DirectoryNotEmpty); dest missing; temp kept true",
                "EEXIST: Err(AlreadyExists); dest missing; temp kept true",
                "EPERM: Err(PermissionDenied); dest missing; temp kept true",
                "EACCES: Err(PermissionDenied); dest missing; temp kept true",
                "EXDEV: Err(CrossesDevices); dest missing; temp kept true",
                "ENOSPC: Err(StorageFull); dest missing; temp kept true",
                "EIO: Err(Uncategorized); dest missing; temp kept true",
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_failed_rename_leaves_a_folder_that_replaced_the_claim() {
        let root = tempfile::tempdir().unwrap();
        let temp = root.path().join(".Pack.tmp");
        std::fs::create_dir(&temp).unwrap();
        let dest = root.path().join("Pack");
        let rival = root.path().join("rival");

        let result = checked_rename(&temp, &dest, |_, to| {
            std::fs::create_dir(&rival).unwrap();
            std::fs::remove_dir(to).unwrap();
            std::fs::rename(&rival, to).unwrap();
            Err(std::io::Error::from_raw_os_error(libc::EPERM))
        })
        .map_err(|err| err.kind());

        assert_eq!(
            format!(
                "{result:?}; dest {}; temp kept {}",
                entry_summary(&dest),
                temp.exists()
            ),
            "Err(PermissionDenied); dest dir with 0 entries; temp kept true"
        );
    }

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

                let result = checked_rename(&temp, &dest, |from, to| std::fs::rename(from, to))
                    .map_err(|err| err.kind());

                seen.push(format!(
                    "{temp_kind} temp, {label}: {result:?}; dest {}; temp kept {}",
                    entry_summary(&dest),
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
