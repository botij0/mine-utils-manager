pub mod discover;
pub mod game_dirs;
pub mod game_guard;
pub mod packs;
pub mod resource_stack;
pub mod worlds;

pub(crate) fn join_error(
    err: impl std::error::Error + Send + Sync + 'static,
) -> mine_utils_core::AppError {
    mine_utils_core::AppError::Io(std::io::Error::other(err))
}
