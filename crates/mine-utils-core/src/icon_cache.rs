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

impl IconCache {
    /// `cache_root` is the app cache dir. Icons live under
    /// `mine-utils/icons/<sha256>.png`.
    pub fn new(cache_root: &Path) -> Self {
        IconCache {
            dir: cache_root.join("mine-utils").join("icons"),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn store(&self, bytes: &[u8]) -> Result<PackIcon, AppError> {
        let sha256 = hex::encode(Sha256::digest(bytes));
        let path = self.dir.join(format!("{sha256}.png"));
        if path.is_file() {
            return Ok(PackIcon { sha256, path });
        }
        std::fs::create_dir_all(&self.dir)?;

        let tmp = self.dir.join(format!(".{sha256}.{}.tmp", std::process::id()));
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
                .join("mine-utils/icons")
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
