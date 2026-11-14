//! Autosaved editor content (spec §5.6).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Numbers the drafts of this process so each gets its own temp file.
static NEXT_DRAFT: AtomicU64 = AtomicU64::new(0);

/// The plugin's draft file, `<data_dir>/draft.json`.
pub struct Draft {
    path: PathBuf,
    /// Private to this handle: views in different windows may save at the same time.
    temp: PathBuf,
}

impl Draft {
    /// Draft stored at `<data_dir>/draft.json`.
    pub fn new(data_dir: &Path) -> Self {
        let number = NEXT_DRAFT.fetch_add(1, Ordering::Relaxed);
        Self {
            path: data_dir.join("draft.json"),
            temp: data_dir.join(format!("draft.json.{}-{number}.tmp", std::process::id())),
        }
    }

    /// Reads the draft; `Ok(None)` when none exists, `InvalidData` when it is not UTF-8.
    pub fn load(&self) -> io::Result<Option<String>> {
        match fs::read_to_string(&self.path) {
            Ok(text) => Ok(Some(text)),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(err),
        }
    }

    /// Replaces the draft atomically: writes a sibling temp file, then renames it
    /// over the draft, so a crash mid-write never leaves a truncated draft.
    pub fn save(&self, text: &str) -> io::Result<()> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        fs::write(&self.temp, text)?;
        fs::rename(&self.temp, &self.path)
    }
}
