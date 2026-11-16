//! Read and write `options.txt` while leaving every byte outside the
//! `resourcePacks` and `incompatibleResourcePacks` lines untouched.
//!
//! Minecraft writes `key:value` lines. Only the two target keys are ever
//! rewritten, and a target line is rewritten only when its parsed array
//! actually changes, so an untouched file round-trips byte for byte. Line
//! endings are preserved per line and a missing final newline stays missing.

use std::io::Write;
use std::path::{Path, PathBuf};

use crate::error::AppError;

pub const RESOURCE_PACKS_KEY: &str = "resourcePacks";
pub const INCOMPATIBLE_RESOURCE_PACKS_KEY: &str = "incompatibleResourcePacks";

const LAUNCH_HINT: &str = "launch Minecraft once so it writes the file, then try again";

/// Everything that can go wrong while reading `options.txt`. The app never
/// repairs the file, so a malformed file is an error the user resolves.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OptionsError {
    #[error("options.txt is missing at {}; {LAUNCH_HINT}", .path.display())]
    Missing { path: PathBuf },
    #[error("options.txt at {} is not valid UTF-8; {LAUNCH_HINT}", .path.display())]
    NotUtf8 { path: PathBuf },
    #[error("options.txt is malformed: {reason}; Mine Utils never rewrites a malformed file")]
    Malformed { reason: String },
}

impl OptionsError {
    pub fn code(&self) -> &'static str {
        self.error_code().as_str()
    }

    pub fn error_code(&self) -> crate::ErrorCode {
        crate::ErrorCode::OptionsTxt
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LineEnding {
    Lf,
    CrLf,
    /// No line ending at all; only possible on the last line.
    None,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Line {
    text: String,
    ending: LineEnding,
}

impl Line {
    fn render(&self, out: &mut String) {
        out.push_str(&self.text);
        match self.ending {
            LineEnding::Lf => out.push('\n'),
            LineEnding::CrLf => out.push_str("\r\n"),
            LineEnding::None => {}
        }
    }
}

/// A parsed `options.txt`. Holds the raw lines and the two decoded arrays.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptionsTxt {
    source: String,
    lines: Vec<Line>,
    resource_index: usize,
    incompatible_index: Option<usize>,
    resource_packs: Vec<String>,
    incompatible_packs: Vec<String>,
}

fn split_lines(text: &str) -> Vec<Line> {
    let mut lines = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        match rest.find('\n') {
            Some(index) => {
                let mut content = &rest[..index];
                let ending = if content.ends_with('\r') {
                    content = &content[..content.len() - 1];
                    LineEnding::CrLf
                } else {
                    LineEnding::Lf
                };
                lines.push(Line {
                    text: content.to_string(),
                    ending,
                });
                rest = &rest[index + 1..];
            }
            None => {
                lines.push(Line {
                    text: rest.to_string(),
                    ending: LineEnding::None,
                });
                rest = "";
            }
        }
    }
    lines
}

fn parse_array(value: &str, key: &str) -> Result<Vec<String>, OptionsError> {
    serde_json::from_str::<Vec<String>>(value).map_err(|err| OptionsError::Malformed {
        reason: format!("{key} is not a JSON array of strings: {err}"),
    })
}

impl OptionsTxt {
    /// Parse the whole file. `resourcePacks` must be present and well formed;
    /// a missing `incompatibleResourcePacks` is simply an empty list.
    pub fn parse(text: &str) -> Result<OptionsTxt, OptionsError> {
        let lines = split_lines(text);
        let mut resource_index = None;
        let mut incompatible_index = None;
        let mut resource_packs = None;
        let mut incompatible_packs = None;
        for (index, line) in lines.iter().enumerate() {
            // A UTF-8 BOM may precede the first key. It is ignored for key
            // comparison only; the line text keeps it so a rewrite preserves it.
            let content = line.text.strip_prefix('\u{feff}').unwrap_or(&line.text);
            let Some((key, value)) = content.split_once(':') else {
                continue;
            };
            match key {
                RESOURCE_PACKS_KEY => {
                    if resource_index.is_some() {
                        return Err(OptionsError::Malformed {
                            reason: format!("more than one {RESOURCE_PACKS_KEY} line"),
                        });
                    }
                    resource_index = Some(index);
                    resource_packs = Some(parse_array(value, RESOURCE_PACKS_KEY)?);
                }
                INCOMPATIBLE_RESOURCE_PACKS_KEY => {
                    if incompatible_index.is_some() {
                        return Err(OptionsError::Malformed {
                            reason: format!("more than one {INCOMPATIBLE_RESOURCE_PACKS_KEY} line"),
                        });
                    }
                    incompatible_index = Some(index);
                    incompatible_packs = Some(parse_array(value, INCOMPATIBLE_RESOURCE_PACKS_KEY)?);
                }
                _ => {}
            }
        }

        let resource_index = resource_index.ok_or_else(|| OptionsError::Malformed {
            reason: format!("no {RESOURCE_PACKS_KEY} line"),
        })?;

        Ok(OptionsTxt {
            source: text.to_string(),
            lines,
            resource_index,
            incompatible_index,
            resource_packs: resource_packs.expect("parsed with the line"),
            incompatible_packs: incompatible_packs.unwrap_or_default(),
        })
    }

    /// The exact bytes this parse read, so a writer can tell whether the file
    /// changed underneath it.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// The decoded `resourcePacks` array, load order low to high.
    pub fn resource_packs(&self) -> &[String] {
        &self.resource_packs
    }

    /// The decoded `incompatibleResourcePacks` array, empty when the line is
    /// missing.
    pub fn incompatible_packs(&self) -> &[String] {
        &self.incompatible_packs
    }

    /// Render the file with new arrays substituted. A target line is only
    /// rewritten when its array changed, so formatting Minecraft chose for an
    /// untouched line survives. A missing `incompatibleResourcePacks` line is
    /// added only when a mutation actually needs it.
    pub fn render(&self, resource_packs: &[String], incompatible_packs: &[String]) -> String {
        let mut lines = self.lines.clone();
        if resource_packs != self.resource_packs.as_slice() {
            replace_value(
                &mut lines[self.resource_index],
                RESOURCE_PACKS_KEY,
                resource_packs,
            );
        }
        match self.incompatible_index {
            Some(index) => {
                if incompatible_packs != self.incompatible_packs.as_slice() {
                    replace_value(
                        &mut lines[index],
                        INCOMPATIBLE_RESOURCE_PACKS_KEY,
                        incompatible_packs,
                    );
                }
            }
            None => {
                if !incompatible_packs.is_empty() {
                    append_line(
                        &mut lines,
                        INCOMPATIBLE_RESOURCE_PACKS_KEY,
                        incompatible_packs,
                    );
                }
            }
        }

        let mut out = String::new();
        for line in &lines {
            line.render(&mut out);
        }
        out
    }
}

fn replace_value(line: &mut Line, key: &str, value: &[String]) {
    // Keep whatever prefix the file used before the first colon, including a
    // leading BOM or any spaces, so only the value bytes change.
    let prefix = line
        .text
        .split_once(':')
        .map(|(prefix, _)| prefix)
        .unwrap_or(key);
    line.text = format!(
        "{prefix}:{}",
        serde_json::to_string(value).expect("strings serialize")
    );
}

fn append_line(lines: &mut Vec<Line>, key: &str, value: &[String]) {
    let text = format!(
        "{key}:{}",
        serde_json::to_string(value).expect("strings serialize")
    );
    // The separator follows the last line that actually has one, so a CRLF
    // file without a final newline stays CRLF. The inserted last line keeps
    // `None`, preserving the missing final newline.
    let separator = lines
        .iter()
        .rev()
        .map(|line| line.ending)
        .find(|ending| *ending != LineEnding::None)
        .unwrap_or(LineEnding::Lf);
    if let Some(last) = lines.last_mut() {
        if last.ending == LineEnding::None {
            last.ending = separator;
            lines.push(Line {
                text,
                ending: LineEnding::None,
            });
            return;
        }
    }
    lines.push(Line {
        text,
        ending: separator,
    });
}

/// Read and parse `options.txt`. A missing file or invalid UTF-8 asks the user
/// to launch Minecraft once; the app never creates or repairs the file.
pub fn read_options(path: &Path) -> Result<OptionsTxt, AppError> {
    let bytes = std::fs::read(path).map_err(|err| {
        if err.kind() == std::io::ErrorKind::NotFound {
            AppError::Options(OptionsError::Missing {
                path: path.to_path_buf(),
            })
        } else {
            AppError::Io(err)
        }
    })?;
    let text = std::str::from_utf8(&bytes).map_err(|_| {
        AppError::Options(OptionsError::NotUtf8 {
            path: path.to_path_buf(),
        })
    })?;
    OptionsTxt::parse(text).map_err(AppError::Options)
}

/// Write `options.txt` atomically: a temp file in the same directory, fsync,
/// rename, then a directory fsync. The target must already exist; a failed
/// write cleans its temp file and leaves the previous bytes in place. The
/// existence check is best-effort: a concurrent external delete between the
/// check and the rename cannot be prevented. Callers that must keep a missing
/// file missing re-read under their own write lock first.
pub fn write_options(path: &Path, text: &str) -> Result<(), AppError> {
    if !path.exists() {
        return Err(AppError::Options(OptionsError::Missing {
            path: path.to_path_buf(),
        }));
    }
    write_options_unchecked(path, text.as_bytes())
}

/// The same atomic write without the existence check. Only the backup restore
/// path uses it, so a restore can recreate a missing `options.txt`.
pub(crate) fn write_options_unchecked(path: &Path, bytes: &[u8]) -> Result<(), AppError> {
    let dir = path.parent().ok_or_else(|| {
        AppError::Options(OptionsError::Malformed {
            reason: format!("{} has no parent directory", path.display()),
        })
    })?;
    let mut tmp = dir.to_path_buf();
    tmp.push(format!(
        ".options.txt.{}.{}.tmp",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0)
    ));

    let write_result = (|| -> std::io::Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    })();
    if let Err(err) = write_result {
        let _ = std::fs::remove_file(&tmp);
        return Err(AppError::Io(err));
    }
    if let Ok(dir_handle) = std::fs::File::open(dir) {
        let _ = dir_handle.sync_all();
    }
    Ok(())
}

pub(crate) fn reject_symlink(path: &std::path::Path) -> Result<(), crate::error::AppError> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => Err(crate::error::AppError::World {
            reason: format!("{} is a symlink and cannot be written", path.display()),
        }),
        Ok(_) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err.into()),
    }
}
