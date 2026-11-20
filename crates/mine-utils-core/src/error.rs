use std::fmt;
use std::path::PathBuf;

use serde::{Serialize, Serializer};

/// Every fallible core operation returns this error. Tauri commands return it
/// unchanged, so the frontend sees one wire shape.
#[derive(Debug)]
pub enum AppError {
    Context {
        error: Box<AppError>,
        message: String,
    },
    InstalledButNotEnabled {
        error: Box<AppError>,
        path: PathBuf,
    },
    Io(std::io::Error),
    InvalidGameDir {
        path: PathBuf,
        reason: String,
    },
    UnknownGameDir {
        path: PathBuf,
    },
    Config {
        path: PathBuf,
        reason: String,
    },
    NoActiveGameDir,
    Pack(crate::pack::PackError),
    Options(crate::options_txt::OptionsError),
    Stack(crate::resource_stack::StackError),
    OptionsSymlink {
        path: PathBuf,
    },
    World {
        reason: String,
    },
    LevelDat(crate::level_dat::LevelDatError),
    DataStack(crate::data_stack::DataStackError),
    WorldMismatch,
    LevelBackup {
        reason: String,
    },
    Modrinth(crate::modrinth::ModrinthError),
    Download(crate::download::DownloadError),
    Provenance(crate::provenance::ProvenanceError),
    Backup {
        reason: String,
    },
    GameRunning {
        running: crate::game_lock::GameRunning,
    },
}

impl AppError {
    pub(crate) fn with_context(self, message: String) -> Self {
        Self::Context {
            error: Box::new(self),
            message,
        }
    }

    pub fn code(&self) -> &'static str {
        self.error_code().as_str()
    }

    pub fn error_code(&self) -> crate::ErrorCode {
        match self {
            AppError::Context { error, .. } | AppError::InstalledButNotEnabled { error, .. } => {
                error.error_code()
            }
            AppError::Io(_) => crate::ErrorCode::Io,
            AppError::InvalidGameDir { .. } => crate::ErrorCode::InvalidGameDir,
            AppError::UnknownGameDir { .. } => crate::ErrorCode::UnknownGameDir,
            AppError::Config { .. } => crate::ErrorCode::Config,
            AppError::NoActiveGameDir => crate::ErrorCode::NoActiveGameDir,
            AppError::Pack(err) => err.error_code(),
            AppError::Options(err) => err.error_code(),
            AppError::Stack(err) => err.error_code(),
            AppError::OptionsSymlink { .. } => crate::ErrorCode::OptionsSymlink,
            AppError::World { .. } => crate::ErrorCode::World,
            AppError::LevelDat(err) => err.error_code(),
            AppError::DataStack(err) => err.error_code(),
            AppError::Modrinth(err) => err.error_code(),
            AppError::Download(err) => err.error_code(),
            AppError::Provenance(err) => err.error_code(),
            AppError::Backup { .. } => crate::ErrorCode::Backup,
            AppError::LevelBackup { .. } => crate::ErrorCode::LevelBackup,
            AppError::WorldMismatch => crate::ErrorCode::WorldMismatch,
            AppError::GameRunning { .. } => crate::ErrorCode::GameRunning,
        }
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AppError::Context { error, message } => write!(f, "{error}. {message}"),
            AppError::InstalledButNotEnabled { error, path } => write!(
                f,
                "{error}. The pack was installed at {} but not enabled. Enable it in Minecraft.",
                path.display()
            ),
            AppError::Io(err) => write!(f, "io error: {err}"),
            AppError::InvalidGameDir { path, reason } => {
                write!(f, "invalid game dir {}: {reason}", path.display())
            }
            AppError::UnknownGameDir { path } => {
                write!(f, "unknown game dir {}", path.display())
            }
            AppError::Config { path, reason } => {
                write!(f, "config error in {}: {reason}", path.display())
            }
            AppError::NoActiveGameDir => {
                write!(f, "no active game dir; pick one in Settings first")
            }
            AppError::Pack(err) => write!(f, "{err}"),
            AppError::Options(err) => write!(f, "{err}"),
            AppError::Stack(err) => write!(f, "{err}"),
            AppError::OptionsSymlink { path } => {
                write!(f, "{} is a symlink and cannot be written", path.display())
            }
            AppError::World { reason } => write!(f, "world error: {reason}"),
            AppError::LevelDat(err) => write!(f, "{err}"),
            AppError::DataStack(err) => write!(f, "{err}"),
            AppError::Modrinth(err) => write!(f, "{err}"),
            AppError::Download(err) => write!(f, "{err}"),
            AppError::Provenance(err) => write!(f, "{err}"),
            AppError::LevelBackup { reason } => write!(f, "level backup failed: {reason}"),
            AppError::WorldMismatch => write!(f, "backup belongs to a different world"),
            AppError::Backup { reason } => write!(f, "backup failed: {reason}"),
            AppError::GameRunning { running } => write!(
                f,
                "Minecraft is running ({}); close it before Mine Utils writes game files",
                running.detail
            ),
        }
    }
}

impl std::error::Error for AppError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            AppError::Context { error, .. } | AppError::InstalledButNotEnabled { error, .. } => {
                Some(error.as_ref())
            }
            AppError::Io(err) => Some(err),
            AppError::Pack(err) => Some(err),
            AppError::Options(err) => Some(err),
            AppError::Stack(err) => Some(err),
            AppError::LevelDat(err) => Some(err),
            AppError::DataStack(err) => Some(err),
            _ => None,
        }
    }
}

impl From<std::io::Error> for AppError {
    fn from(err: std::io::Error) -> Self {
        AppError::Io(err)
    }
}

impl From<crate::pack::PackError> for AppError {
    fn from(err: crate::pack::PackError) -> Self {
        AppError::Pack(err)
    }
}

impl From<crate::resource_stack::StackError> for AppError {
    fn from(err: crate::resource_stack::StackError) -> Self {
        AppError::Stack(err)
    }
}

impl From<crate::options_txt::OptionsError> for AppError {
    fn from(err: crate::options_txt::OptionsError) -> Self {
        AppError::Options(err)
    }
}

impl From<crate::modrinth::ModrinthError> for AppError {
    fn from(err: crate::modrinth::ModrinthError) -> Self {
        AppError::Modrinth(err)
    }
}

impl From<crate::download::DownloadError> for AppError {
    fn from(err: crate::download::DownloadError) -> Self {
        AppError::Download(err)
    }
}

impl From<crate::provenance::ProvenanceError> for AppError {
    fn from(err: crate::provenance::ProvenanceError) -> Self {
        AppError::Provenance(err)
    }
}

#[derive(Serialize)]
#[cfg_attr(feature = "wire-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "wire-types", ts(rename = "AppError"))]
pub struct AppErrorWire {
    code: crate::ErrorCode,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "wire-types", ts(optional))]
    installed_path: Option<String>,
}

impl Serialize for AppError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        AppErrorWire {
            code: self.error_code(),
            message: self.to_string(),
            installed_path: match self {
                Self::InstalledButNotEnabled { path, .. } => {
                    Some(path.to_string_lossy().into_owned())
                }
                _ => None,
            },
        }
        .serialize(serializer)
    }
}
