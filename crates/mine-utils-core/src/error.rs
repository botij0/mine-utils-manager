use std::fmt;
use std::path::PathBuf;

use serde::{Serialize, Serializer};

/// Every fallible core operation returns this error. Tauri commands return it
/// unchanged, so the frontend sees one wire shape.
#[derive(Debug)]
pub enum AppError {
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
    Backup {
        reason: String,
    },
    GameRunning {
        running: crate::game_lock::GameRunning,
    },
}

impl AppError {
    pub fn code(&self) -> &'static str {
        match self {
            AppError::Io(_) => "io",
            AppError::InvalidGameDir { .. } => "invalid_game_dir",
            AppError::UnknownGameDir { .. } => "unknown_game_dir",
            AppError::Config { .. } => "config",
            AppError::NoActiveGameDir => "no_active_game_dir",
            AppError::Pack(err) => err.code(),
            AppError::Options(err) => err.code(),
            AppError::Stack(err) => err.code(),
            AppError::Backup { .. } => "backup",
            AppError::GameRunning { .. } => "game_running",
        }
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
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
            AppError::Backup { reason } => write!(f, "backup failed: {reason}"),
            AppError::GameRunning { running } => write!(
                f,
                "Minecraft is running ({}); close it before Mine Utils writes options.txt",
                running.detail
            ),
        }
    }
}

impl std::error::Error for AppError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            AppError::Io(err) => Some(err),
            AppError::Pack(err) => Some(err),
            AppError::Options(err) => Some(err),
            AppError::Stack(err) => Some(err),
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

#[derive(Serialize)]
struct AppErrorWire<'a> {
    code: &'a str,
    message: String,
}

impl Serialize for AppError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        AppErrorWire {
            code: self.code(),
            message: self.to_string(),
        }
        .serialize(serializer)
    }
}
