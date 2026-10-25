use std::io::Write;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::error::AppError;
use crate::pack::PackIcon;

/// Content-addressed icon cache. The name is the SHA-256 of the icon bytes, so
/// two packs that ship the same pack.png share one file.
pub struct IconCache {
    dir: PathBuf,
}

/// Largest icon edge the cache accepts. The webview decodes these bytes, so a
/// hostile IHDR must not request a gigapixel decode.
pub const MAX_ICON_DIMENSION: u32 = 4096;

/// Width and height from a real PNG IHDR chunk, when the bytes start with a PNG
/// header. Anything the helper cannot parse is left for the caller to accept.
pub fn png_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    if bytes.len() < 24 || bytes[..8] != SIGNATURE || &bytes[12..16] != b"IHDR" {
        return None;
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
    let height = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
    Some((width, height))
}

/// False only when a real PNG header claims more pixels than the cap.
pub fn icon_within_pixel_cap(bytes: &[u8]) -> bool {
    match png_dimensions(bytes) {
        Some((width, height)) => width <= MAX_ICON_DIMENSION && height <= MAX_ICON_DIMENSION,
        None => true,
    }
}

impl IconCache {
    /// `app_cache_dir` is already the app cache dir (`.../mine-utils`). Icons
    /// live under `icons/<sha256>.png`, so the full path is
    /// `$XDG_CACHE_HOME/mine-utils/icons/<sha256>.png`.
    pub fn new(app_cache_dir: &Path) -> Self {
        IconCache {
            dir: app_cache_dir.join("icons"),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn store(&self, bytes: &[u8]) -> Result<PackIcon, AppError> {
        // Checked before caching, so a lying header never reaches the webview.
        if !icon_within_pixel_cap(bytes) {
            return Err(crate::pack::PackError::BrokenArchive {
                name: "pack.png".to_string(),
                reason: format!(
                    "icon claims more than {MAX_ICON_DIMENSION} by {MAX_ICON_DIMENSION} pixels"
                ),
            }
            .into());
        }
        let sha256 = hex::encode(Sha256::digest(bytes));
        let path = self.dir.join(format!("{sha256}.png"));
        if path.is_file() {
            return Ok(PackIcon { sha256, path });
        }
        std::fs::create_dir_all(&self.dir)?;

        let tmp = self
            .dir
            .join(format!(".{sha256}.{}.tmp", std::process::id()));
        let write_result = (|| -> std::io::Result<()> {
            let mut file = std::fs::File::create(&tmp)?;
            file.write_all(bytes)?;
            file.sync_all()?;
            std::fs::rename(&tmp, &path)?;
            Ok(())
        })();
        if let Err(err) = write_result {
            let _ = std::fs::remove_file(&tmp);
            return Err(err.into());
        }
        Ok(PackIcon { sha256, path })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png_header(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        bytes.extend_from_slice(&13u32.to_be_bytes());
        bytes.extend_from_slice(b"IHDR");
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes.extend_from_slice(&[0; 5]);
        bytes
    }

    #[test]
    fn reads_png_dimensions_from_the_ihdr() {
        assert_eq!(png_dimensions(&png_header(64, 32)), Some((64, 32)));
        assert_eq!(png_dimensions(b"not a png at all"), None);
        assert_eq!(png_dimensions(&png_header(1, 1)[..20]), None);
        assert!(icon_within_pixel_cap(&png_header(64, 32)));
        assert!(icon_within_pixel_cap(b"not a png at all"));
    }

    #[test]
    fn refuses_an_icon_claiming_to_be_huge() {
        assert!(!icon_within_pixel_cap(&png_header(4097, 16)));
        assert!(!icon_within_pixel_cap(&png_header(16, 4097)));

        let root = tempfile::tempdir().unwrap();
        let cache = IconCache::new(root.path());
        let err = cache.store(&png_header(65_535, 65_535)).unwrap_err();
        assert_eq!(err.code(), "pack_broken_archive");
        assert_eq!(
            std::fs::read_dir(cache.dir())
                .map(|dir| dir.count())
                .unwrap_or(0),
            0
        );
    }

    #[test]
    fn stores_once_per_content_hash() {
        let root = tempfile::tempdir().unwrap();
        let cache = IconCache::new(root.path());
        let icon = cache.store(b"fake png bytes").unwrap();
        assert!(icon.path.is_file());
        assert_eq!(icon.path.file_name().unwrap().to_string_lossy().len(), 68);
        assert_eq!(
            icon.path,
            root.path()
                .join("icons")
                .join(format!("{}.png", icon.sha256))
        );

        let again = cache.store(b"fake png bytes").unwrap();
        assert_eq!(again, icon);
        assert_eq!(std::fs::read_dir(cache.dir()).unwrap().count(), 1);

        let other = cache.store(b"different").unwrap();
        assert_ne!(other.sha256, icon.sha256);
        assert_eq!(std::fs::read_dir(cache.dir()).unwrap().count(), 2);
    }
}
