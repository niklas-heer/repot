//! Narrow native reads, with Git remaining the compatibility fallback.
//!
//! Only repository layout is read here. Status, refs, transport and mutation
//! remain in Git. The isolated opener ignores environment path overrides and
//! external configuration; unsupported formats fall back to Git.

use std::path::{Path, PathBuf};

pub const OPERATION_MARKERS: [&str; 8] = [
    "MERGE_HEAD",
    "CHERRY_PICK_HEAD",
    "REVERT_HEAD",
    "BISECT_LOG",
    "rebase-merge",
    "rebase-apply",
    "sequencer",
    "info/grafts",
];

#[derive(Debug)]
pub struct Layout {
    git_dir: PathBuf,
    common_dir: PathBuf,
}

impl Layout {
    pub fn open(path: &Path) -> Option<Self> {
        let repository = gix::open_opts(
            path,
            gix::open::Options::isolated()
                .strict_config(true)
                .bail_if_untrusted(true),
        )
        .ok()?;
        Some(Self {
            git_dir: repository.git_dir().into(),
            common_dir: repository.common_dir().into(),
        })
    }

    pub fn marker_path(&self, marker: &str) -> PathBuf {
        if marker == "info/grafts" {
            self.common_dir.join(marker)
        } else {
            self.git_dir.join(marker)
        }
    }

    /// An uncertain layout or filesystem error requires the Git fallback.
    pub fn operation_in_progress(&self) -> Option<bool> {
        for marker in OPERATION_MARKERS {
            if self.marker_path(marker).try_exists().ok()? {
                return Some(true);
            }
        }
        Some(false)
    }
}
