use std::path::PathBuf;

use crate::{
    Config, CoreInstallOutcome, DataStack, DatapackImportOutcome, GameDirEntry, GameRunning,
    GameStatus, ImportOutcome, PackListing, SearchHit,
};

#[derive(Debug, Clone, serde::Serialize)]
#[cfg_attr(feature = "wire-types", derive(ts_rs::TS))]
pub struct GameDirState {
    pub config: Config,
    pub entries: Vec<GameDirEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[cfg_attr(feature = "wire-types", derive(ts_rs::TS))]
pub struct GameGuard {
    pub running: Option<GameRunning>,
    pub stale_locks: Vec<PathBuf>,
    pub detection_supported: bool,
    pub process_probe_available: bool,
}

impl From<GameStatus> for GameGuard {
    fn from(status: GameStatus) -> Self {
        GameGuard {
            running: status.running,
            stale_locks: status.stale_locks,
            detection_supported: status.supported,
            process_probe_available: status.process_probe_available,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
#[cfg_attr(feature = "wire-types", derive(ts_rs::TS))]
pub struct DatapackListing {
    pub packs: PackListing,
    pub stack: DataStack,
}

#[derive(Debug, Clone, serde::Serialize)]
#[cfg_attr(feature = "wire-types", derive(ts_rs::TS))]
pub struct DatapackDeleteOutcome {
    pub removed_references: bool,
    pub stack: DataStack,
}

pub const SEARCH_PAGE_SIZE: u8 = 20;

#[derive(Debug, Clone, serde::Serialize)]
#[cfg_attr(feature = "wire-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "wire-types", ts(rename = "ModrinthSearchPage"))]
pub struct SearchPage {
    pub hits: Vec<SearchHit>,
    pub has_more: bool,
}

impl SearchPage {
    pub fn new(hits: Vec<SearchHit>) -> Self {
        SearchPage {
            has_more: hits.len() == usize::from(SEARCH_PAGE_SIZE),
            hits,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
#[cfg_attr(feature = "wire-types", derive(ts_rs::TS))]
pub struct ModrinthTargets {
    pub installed_versions: Vec<String>,
    pub preferred_launcher: Option<String>,
    pub world_version: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[cfg_attr(feature = "wire-types", derive(ts_rs::TS))]
pub enum ModrinthInstallOutcome {
    #[serde(rename = "resourcepack")]
    ResourcePack {
        outcome: ImportOutcome,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "wire-types", ts(optional))]
        provenance_warning: Option<String>,
    },
    Datapack {
        outcome: DatapackImportOutcome,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "wire-types", ts(optional))]
        provenance_warning: Option<String>,
    },
}

impl From<CoreInstallOutcome> for ModrinthInstallOutcome {
    fn from(outcome: CoreInstallOutcome) -> Self {
        match outcome {
            CoreInstallOutcome::ResourcePack {
                outcome,
                provenance_warning,
            } => ModrinthInstallOutcome::ResourcePack {
                outcome,
                provenance_warning,
            },
            CoreInstallOutcome::Datapack {
                outcome,
                provenance_warning,
            } => ModrinthInstallOutcome::Datapack {
                outcome,
                provenance_warning,
            },
        }
    }
}
