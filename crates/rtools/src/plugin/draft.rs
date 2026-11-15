//! Autosaved plugin text, one file per draft (spec §5.6).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use gpui_kit::component::input::Rope;
use gpui_kit::{App, AppContext as _, Task};

/// Pause after the last edit before the drafts are written.
const SAVE_DEBOUNCE: Duration = Duration::from_secs(1);

/// Numbers the drafts of this process so each gets its own temp file.
static NEXT_DRAFT: AtomicU64 = AtomicU64::new(0);

/// A plugin's draft file, `<data_dir>/<file_name>`.
pub struct Draft {
    path: PathBuf,
    /// Private to this handle: views in different windows may save at the same time.
    temp: PathBuf,
}

impl Draft {
    /// Draft stored at `<data_dir>/<file_name>`; plugins with several drafts
    /// give each its own name in the same directory.
    pub fn new(data_dir: &Path, file_name: &str) -> Self {
        let number = NEXT_DRAFT.fetch_add(1, Ordering::Relaxed);
        Self {
            path: data_dir.join(file_name),
            temp: data_dir.join(format!("{file_name}.{}-{number}.tmp", std::process::id())),
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

/// Writes a plugin's drafts a moment after the last edit, and at once when
/// the plugin goes away (app quit, view released). Failures are logged and
/// never touch the editors.
pub struct Autosave {
    shared: Arc<Pending>,
    /// Replacing it restarts the debounce.
    task: Task<()>,
}

/// What the debounce task and `flush` share.
struct Pending {
    /// Locked while writing, so the two never write the same temp file at once.
    drafts: Mutex<Vec<Draft>>,
    /// Texts edited since the last write, one per draft.
    texts: Mutex<Option<Vec<Rope>>>,
}

impl Pending {
    /// Writes the waiting texts, if any, into their drafts. The texts are taken
    /// under the write lock, so whichever writer runs last writes the newest
    /// edit and each edit is written once. Lock order: `drafts`, then `texts`.
    fn write(&self) {
        let drafts = self.drafts.lock().unwrap_or_else(PoisonError::into_inner);
        let texts = self
            .texts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        for (draft, text) in drafts.iter().zip(texts.into_iter().flatten()) {
            if let Err(err) = draft.save(&text.to_string()) {
                tracing::warn!("saving the draft {} failed: {}", draft.path.display(), err);
            }
        }
    }
}

impl FromIterator<Draft> for Autosave {
    /// Autosaves into these drafts, in order.
    fn from_iter<I: IntoIterator<Item = Draft>>(drafts: I) -> Self {
        Self {
            shared: Arc::new(Pending {
                drafts: Mutex::new(drafts.into_iter().collect()),
                texts: Mutex::default(),
            }),
            task: Task::ready(()),
        }
    }
}

impl Autosave {
    /// Reads every draft in order; a missing one, or one that cannot be read
    /// (logged), gives empty text.
    pub fn load(&self) -> Vec<String> {
        let drafts = self
            .shared
            .drafts
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        drafts
            .iter()
            .map(|draft| {
                draft
                    .load()
                    .unwrap_or_else(|err| {
                        tracing::warn!(
                            "reading the draft {} failed: {}",
                            draft.path.display(),
                            err
                        );
                        None
                    })
                    .unwrap_or_default()
            })
            .collect()
    }

    /// Records the texts after an edit, one per draft (cloning a rope is
    /// cheap), and writes them once no newer edit arrives within the pause.
    pub fn schedule(&mut self, texts: Vec<Rope>, cx: &App) {
        *self
            .shared
            .texts
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(texts);
        let shared = Arc::clone(&self.shared);
        let executor = cx.background_executor().clone();
        self.task = cx.background_spawn(async move {
            executor.timer(SAVE_DEBOUNCE).await;
            shared.write();
        });
    }

    /// Writes texts still waiting for the pause right away.
    pub fn flush(&mut self) {
        self.task = Task::ready(());
        self.shared.write();
    }
}
