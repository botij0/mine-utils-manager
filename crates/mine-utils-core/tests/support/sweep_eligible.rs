use std::path::Path;

pub fn eligible(path: &Path) -> bool {
    use std::os::fd::AsRawFd;
    let file = std::fs::File::open(path).unwrap();
    let mut stat = std::mem::MaybeUninit::<libc::statfs>::uninit();
    if unsafe { libc::fstatfs(file.as_raw_fd(), stat.as_mut_ptr()) } != 0 {
        return false;
    }
    let stat = unsafe { stat.assume_init() };
    #[cfg(target_os = "linux")]
    return [
        0xef53, 0x9123683e, 0x58465342, 0xf2f52010, 0x01021994, 0x858458f6, 0x2fc12fc1,
    ]
    .contains(&(stat.f_type as u32));
    #[cfg(target_os = "macos")]
    return stat.f_flags & libc::MNT_LOCAL as u32 != 0;
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    false
}
