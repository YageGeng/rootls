//! Transactional storage and reconstruction of a complete clipboard offer.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::time::Duration;

use rusqlite::{Connection, OptionalExtension as _, TransactionBehavior, params};
use tempfile::TempDir;
use url::Url;

use super::content::{BYTE_LIMIT, Group, Kind, Payload};
use super::{Error, Result};

/// Each operation gets a short-lived connection; SQLite serializes concurrent writers.
#[derive(Clone)]
pub(super) struct History {
    root: Option<PathBuf>,
    max_groups: usize,
    max_bytes: usize,
}

/// Search results contain only metadata, so scrolling never opens an image.
pub(super) struct Summary {
    pub id: i64,
    pub title: String,
    pub total: usize,
    pub images: usize,
    pub missing: usize,
    pub has_paths: bool,
}

/// One original image and its position in the unfiltered group.
pub(super) struct SavedPicture {
    pub position: i64,
    pub label: String,
    pub bytes: Vec<u8>,
}

/// The selection a user wants to publish.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Selection {
    All,
    One(i64),
    Paths,
}

/// A failed publish drops its staged files; a successful one must keep them alive.
pub(super) struct Prepared {
    pub payload: Payload,
    pub exports: Option<TempDir>,
}

impl History {
    /// Locates a history database without touching disk until the first operation.
    pub fn new(root: impl Into<Option<PathBuf>>) -> Self {
        Self {
            root: root.into(),
            max_groups: 1000,
            max_bytes: BYTE_LIMIT,
        }
    }

    /// Fails closed when the OS cannot identify this user's persistent data directory.
    pub fn directory(&self) -> Result<&Path> {
        self.root.as_deref().ok_or(Error::DataDirectory)
    }

    /// Creates the current user's private database and enables per-connection constraints.
    fn connect(&self) -> Result<Connection> {
        let root = self.directory()?;
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::{DirBuilderExt as _, PermissionsExt as _};
            builder.mode(0o700).create(root)?;
            // The launcher may have created the plugin directory with a more permissive umask.
            if fs::metadata(root)?.permissions().mode() & 0o077 != 0 {
                fs::set_permissions(root, fs::Permissions::from_mode(0o700))?;
            }
        }
        #[cfg(not(unix))]
        builder.create(root)?;
        let db = Connection::open(root.join("history.sqlite3"))?;
        db.busy_timeout(Duration::from_secs(5))?;
        db.execute_batch(
            "PRAGMA foreign_keys=ON;
            PRAGMA secure_delete=ON;
            CREATE TABLE IF NOT EXISTS history(
                id INTEGER PRIMARY KEY AUTOINCREMENT, digest BLOB UNIQUE NOT NULL,
                recent INTEGER NOT NULL, copied_at INTEGER NOT NULL, size INTEGER NOT NULL);
            CREATE TABLE IF NOT EXISTS member(
                parent INTEGER NOT NULL REFERENCES history(id) ON DELETE CASCADE,
                pos INTEGER NOT NULL, kind INTEGER NOT NULL, caption TEXT NOT NULL,
                uri TEXT NOT NULL, data BLOB NOT NULL, thumb BLOB NOT NULL,
                PRIMARY KEY(parent,pos));",
        )?;
        Ok(db)
    }

    /// Commits only if this capture still belongs to the active generation.
    pub fn save(&self, group: Group, expected: u64, generation: &Arc<AtomicU64>) -> Result<bool> {
        if group.parts.is_empty() || generation.load(Ordering::Acquire) != expected {
            return Ok(false);
        }
        if group.size > self.max_bytes {
            return Err(Error::Limit);
        }
        let members = group.parts.len();
        let mut db = self.connect()?;
        // SQLite runs this hook after obtaining its commit lock. Opt-out can veto a write
        // that was already waiting on a reader without ever blocking the UI thread.
        let epoch = Arc::clone(generation);
        db.commit_hook(Some(move || epoch.load(Ordering::Acquire) != expected));
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let recent: i64 =
            tx.query_row("SELECT coalesce(max(recent),0)+1 FROM history", [], |r| {
                r.get(0)
            })?;
        if tx.execute(
            "UPDATE history SET recent=?1,copied_at=unixepoch() WHERE digest=?2",
            params![recent, group.digest],
        )? == 0
        {
            tx.execute(
                "INSERT INTO history(digest,recent,copied_at,size) VALUES(?1,?2,unixepoch(),?3)",
                params![group.digest, recent, group.size],
            )?;
            let id = tx.last_insert_rowid();
            // Older databases require the thumbnail column; previews now use the stored original.
            let mut stmt = tx.prepare(
                "INSERT INTO member(parent,pos,kind,caption,uri,data,thumb)
                VALUES(?1,?2,?3,?4,?5,?6,x'')",
            )?;
            for (pos, part) in group.parts.into_iter().enumerate() {
                stmt.execute(params![
                    id,
                    pos,
                    part.kind as i64,
                    part.caption,
                    part.uri,
                    part.bytes
                ])?;
            }
        }
        tx.execute(
            "DELETE FROM history WHERE id IN (
            SELECT id FROM (SELECT id,row_number() OVER w AS n,sum(size) OVER w AS bytes
            FROM history WINDOW w AS (ORDER BY recent DESC)) WHERE n>?1 OR bytes>?2)",
            params![self.max_groups, self.max_bytes],
        )?;
        match tx.commit() {
            Err(rusqlite::Error::SqliteFailure(code, _))
                if code.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_COMMITHOOK =>
            {
                return Ok(false);
            }
            result => result?,
        }
        tracing::debug!("saved clipboard history group with {} members", members);
        Ok(true)
    }

    /// Matches any member literally while returning each group exactly once.
    pub fn search(&self, query: &str, images_only: bool) -> Result<Vec<Summary>> {
        let db = self.connect()?;
        // AUTOINCREMENT preserves first creation order across deduplication and clock changes.
        let mut stmt = db.prepare(
            "SELECT h.id,
            (SELECT substr(caption,1,160) FROM member WHERE parent=h.id ORDER BY pos LIMIT 1),
            count(*),sum(m.kind=1),sum(m.kind=3),max(length(m.uri)>0)
            FROM history h JOIN member m ON m.parent=h.id
            GROUP BY h.id HAVING (?1='' OR EXISTS (SELECT 1 FROM member s WHERE s.parent=h.id
                AND instr(lower(s.caption),lower(?1))>0))
            AND (?2=0 OR sum(m.kind=1)>0) ORDER BY h.id DESC",
        )?;
        let rows = stmt.query_map(params![query, images_only], |r| {
            Ok(Summary {
                id: r.get(0)?,
                title: r.get(1)?,
                total: r.get(2)?,
                images: r.get(3)?,
                missing: r.get(4)?,
                has_paths: r.get(5)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Reads only the requested image in original member order, skipping text and file paths.
    pub fn picture(&self, id: i64, index: usize) -> Result<SavedPicture> {
        let db = self.connect()?;
        db.query_row(
            "SELECT pos,caption,data FROM member WHERE parent=?1 AND kind=?2 ORDER BY pos LIMIT 1 OFFSET ?3",
            params![id, Kind::Image as i64, index],
            |r| Ok(SavedPicture { position: r.get(0)?, label: r.get(1)?, bytes: r.get(2)? }),
        ).optional()?.ok_or(Error::Missing)
    }

    /// Prepares a complete payload before modifying the system clipboard.
    pub fn prepare(
        &self,
        id: i64,
        selection: Selection,
        confirmed: Option<&[i64]>,
    ) -> Result<Prepared> {
        let mut db = self.connect()?;
        let tx = db.transaction()?;
        let position = match selection {
            Selection::One(pos) => Some(pos),
            _ => None,
        };
        let mut stmt = tx.prepare("SELECT pos,kind,caption,uri FROM member WHERE parent=?1 AND (?2 IS NULL OR pos=?2) ORDER BY pos")?;
        let members = stmt
            .query_map(params![id, position], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if members.is_empty() {
            return Err(Error::Missing);
        }
        let mut exports = None;
        let mut paths = Vec::new();
        let mut available = Vec::new();
        let path_text =
            selection == Selection::Paths || members.iter().all(|m| m.1 == Kind::Path as i64);
        let all_text = members.iter().all(|m| m.1 == Kind::Text as i64);
        let mut text = Vec::new();
        for (pos, tag, caption, uri) in &members {
            let kind = Kind::try_from(*tag)?;
            if path_text || (members.len() == 1 && kind == Kind::Missing) {
                if !uri.is_empty() {
                    text.push(caption.clone());
                }
                continue;
            }
            let path = Url::parse(uri).ok().and_then(|url| url.to_file_path().ok());
            match kind {
                Kind::Text if all_text => text.push(caption.clone()),
                Kind::Image | Kind::Text => {
                    let bytes: Vec<u8> = if kind == Kind::Text {
                        caption.as_bytes().to_vec()
                    } else {
                        tx.query_row(
                            "SELECT data FROM member WHERE parent=?1 AND pos=?2",
                            params![id, pos],
                            |r| r.get(0),
                        )?
                    };
                    if members.len() == 1 {
                        return Ok(Prepared {
                            payload: Payload::Image(bytes),
                            exports: None,
                        });
                    }
                    let folder = match &exports {
                        Some(dir) => dir,
                        None => {
                            let root = self.directory()?.join("exports");
                            fs::create_dir_all(&root)?;
                            exports
                                .insert(tempfile::Builder::new().prefix("clip-").tempdir_in(root)?)
                        }
                    };
                    let folder = folder.path().join(pos.to_string());
                    fs::create_dir(&folder)?;
                    let fallback = if kind == Kind::Text {
                        "text.txt"
                    } else {
                        caption
                    };
                    let name = path
                        .as_deref()
                        .unwrap_or(Path::new(fallback))
                        .file_name()
                        .ok_or(Error::Invalid)?;
                    let mut target = folder.join(name);
                    // A fallback raster can use a different encoding from its vanished source.
                    if kind == Kind::Image {
                        let format = image::guess_format(&bytes)?;
                        if image::ImageFormat::from_path(&target).ok() != Some(format) {
                            target.set_extension(format.extensions_str()[0]);
                        }
                    }
                    fs::write(&target, bytes)?;
                    paths.push(target);
                    available.push(*pos);
                }
                Kind::Path if path.as_ref().is_some_and(|p| p.exists()) => {
                    paths.push(path.ok_or(Error::Invalid)?);
                    available.push(*pos);
                }
                Kind::Missing | Kind::Path => {}
            }
        }
        if !text.is_empty() {
            return Ok(Prepared {
                payload: Payload::Text(text.join("\n")),
                exports: None,
            });
        }
        if paths.is_empty() {
            return Err(Error::Missing);
        }
        // A previously confirmed subset can change if another source file disappears or returns.
        if confirmed.map_or(available.len() < members.len(), |previous| {
            previous != available
        }) {
            return Err(Error::Partial(available));
        }
        Ok(Prepared {
            payload: Payload::Files(paths),
            exports,
        })
    }

    /// Removes only exports created by this plugin that the current offer no longer references.
    pub fn clean_exports(&self, keep: &[PathBuf]) -> Result<bool> {
        let root = self.directory()?.join("exports");
        let entries = match fs::read_dir(&root) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(e.into()),
        };
        // A file manager may return the resolved path for an export reached through a symlink.
        let resolved = keep
            .iter()
            .filter_map(|p| fs::canonicalize(p).ok())
            .collect::<Vec<_>>();
        let mut remaining = false;
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            if entry.file_name().to_string_lossy().starts_with("clip-")
                && entry.file_type()?.is_dir()
            {
                let canonical = fs::canonicalize(&path)?;
                if keep.iter().any(|p| p.starts_with(&path))
                    || resolved.iter().any(|p| p.starts_with(&canonical))
                {
                    remaining = true;
                } else {
                    fs::remove_dir_all(path)?;
                }
            }
        }
        Ok(remaining)
    }

    /// Returns the digest of a deleted entry so already-started captures can be excluded.
    pub fn remove(&self, id: Option<i64>) -> Result<Option<Vec<u8>>> {
        let db = self.connect()?;
        if let Some(id) = id {
            return Ok(db
                .query_row(
                    "DELETE FROM history WHERE id=?1 RETURNING digest",
                    [id],
                    |r| r.get(0),
                )
                .optional()?);
        }
        db.execute("DELETE FROM history", [])?;
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::super::content::Input;
    use super::*;
    use image::ImageFormat;
    use std::io::Cursor;

    /// Creates distinctly named snapshots that survive deleting both source images.
    #[test]
    fn group_survives_reopen_search_and_source_deletion() {
        let temp = tempfile::tempdir().unwrap();
        let left = temp.path().join("left");
        let right = temp.path().join("右侧");
        fs::create_dir(&left).unwrap();
        fs::create_dir(&right).unwrap();
        let a = left.join("相同 图.png");
        let b = right.join("相同 图.png");
        let doc = right.join("设计%.pdf");
        let red = include_bytes!("../../../../../assets/icons/rtools-22.png");
        let blue = include_bytes!("../../../../../assets/icons/rtools-44.png");
        fs::write(&a, red).unwrap();
        fs::write(&b, blue).unwrap();
        fs::write(&doc, b"not an image").unwrap();
        let root = temp.path().join("history");
        let history = History::new(root.clone());
        let epoch = Arc::new(AtomicU64::new(2));
        let group = || {
            Group::try_from(vec![
                Input::file(a.clone()),
                Input::file(doc.clone()),
                Input::file(b.clone()),
            ])
            .unwrap()
        };
        assert!(history.save(group(), 2, &epoch).unwrap());
        assert!(history.save(group(), 2, &epoch).unwrap());
        let rows = history.search("右侧", true).unwrap();
        assert_eq!(rows.len(), 1);
        let id = rows[0].id;
        assert_eq!((rows[0].total, rows[0].images), (3, 2));
        assert_eq!(history.search("%", false).unwrap().len(), 1);
        drop(history);
        fs::remove_file(a).unwrap();
        fs::remove_file(b).unwrap();
        let reopened = History::new(root);
        let batch = reopened.prepare(id, Selection::All, None).unwrap();
        let Payload::Files(ref files) = batch.payload else {
            panic!("expected three file references")
        };
        assert_eq!(files.len(), 3);
        assert_eq!(files[0].file_name(), files[2].file_name());
        assert_ne!(files[0], files[2]);
        assert_eq!(fs::read(&files[0]).unwrap(), red);
        assert_eq!(files[1], doc);
        assert_eq!(fs::read(&files[2]).unwrap(), blue);
        assert!(
            matches!(reopened.prepare(id,Selection::One(2),None).unwrap().payload,Payload::Image(ref data) if data==blue)
        );
        fs::remove_file(&doc).unwrap();
        let last = reopened.picture(id, 1).unwrap();
        assert_eq!((last.position, last.bytes.as_slice()), (2, blue.as_slice()));
        assert!(matches!(reopened.picture(id, 2), Err(Error::Missing)));
        assert!(matches!(
            reopened.prepare(id, Selection::All, None),
            Err(Error::Partial(positions)) if positions == [0, 2]
        ));
        assert!(matches!(
            reopened.prepare(id, Selection::All, Some(&[0, 1])),
            Err(Error::Partial(positions)) if positions == [0, 2]
        ));
        let partial = reopened.prepare(id, Selection::All, Some(&[0, 2])).unwrap();
        assert!(matches!(partial.payload,Payload::Files(ref list) if list.len()==2));
    }

    /// A corrupt sibling remains visible, while valid JPEG bytes and repeated positions survive.
    #[test]
    fn jpeg_and_failed_member_keep_order_and_individual_recovery() {
        let temp = tempfile::tempdir().unwrap();
        let mut jpeg = Vec::new();
        image::load_from_memory(include_bytes!("../../../../../assets/icons/rtools-22.png"))
            .unwrap()
            .to_rgb8()
            .write_to(&mut Cursor::new(&mut jpeg), ImageFormat::Jpeg)
            .unwrap();
        let bad = temp.path().join("missing.jpg");
        let history = History::new(temp.path().join("history"));
        let epoch = Arc::new(AtomicU64::new(0));
        for index in [1, 0] {
            let mut inputs = vec![Input::Image(jpeg.clone()), Input::Image(jpeg.clone())];
            inputs.insert(index, Input::file(bad.clone()));
            history
                .save(Group::try_from(inputs).unwrap(), 0, &epoch)
                .unwrap();
        }
        let rows = history.search("", false).unwrap();
        assert_eq!(rows.len(), 2);
        for (page, position) in [1, 2].into_iter().enumerate() {
            let saved = history.picture(rows[0].id, page).unwrap();
            assert_eq!(saved.position, position);
            assert_eq!(saved.bytes, jpeg);
        }
        assert_eq!(rows[0].missing, 1);
        assert!(
            matches!(history.prepare(rows[0].id,Selection::One(2),None).unwrap().payload,Payload::Image(ref data) if *data==jpeg)
        );
        assert!(
            matches!(history.prepare(rows[0].id,Selection::Paths,None).unwrap().payload,Payload::Text(ref s) if s==&bad.to_string_lossy())
        );
    }

    /// The recovery action keeps both source paths in order even if every image failed to load.
    #[test]
    fn missing_image_group_recovers_all_original_paths() {
        let temp = tempfile::tempdir().unwrap();
        let paths = ["second.jpg", "first.png"].map(|name| temp.path().join(name));
        let history = History::new(temp.path().join("history"));
        let group = Group::try_from(Vec::from(paths.clone().map(Input::file))).unwrap();
        history.save(group, 0, &Arc::default()).unwrap();

        let row = &history.search("", false).unwrap()[0];
        assert_eq!((row.images, row.missing, row.has_paths), (0, 2, true));
        assert!(matches!(
            history.prepare(row.id, Selection::All, None),
            Err(Error::Missing)
        ));
        let payload = history
            .prepare(row.id, Selection::Paths, None)
            .unwrap()
            .payload;
        let expected = paths.map(|path| path.display().to_string()).join("\n");
        assert!(matches!(payload, Payload::Text(text) if text == expected));
    }

    /// Recopying an older group refreshes retention without changing its position in search.
    #[test]
    fn history_lists_newest_created_first_even_after_a_duplicate() {
        let temp = tempfile::tempdir().unwrap();
        let history = History::new(temp.path().to_owned());
        let gate = Arc::new(AtomicU64::new(0));
        for label in ["match old", "match new", "match old"] {
            let group = Group::try_from(vec![Input::Text(label.into())]).unwrap();
            history.save(group, 0, &gate).unwrap();
        }
        for query in ["", "match"] {
            let rows = history.search(query, false).unwrap();
            assert_eq!(
                rows.into_iter().map(|row| row.title).collect::<Vec<_>>(),
                ["match new", "match old"]
            );
        }
    }

    /// A panorama with a small grayscale raster survives removal of its source file.
    #[test]
    fn large_grayscale_image_keeps_its_original_and_preview_source() {
        use image::{
            ExtendedColorType, ImageEncoder as _,
            codecs::png::{CompressionType, FilterType, PngEncoder},
        };

        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("panorama.png");
        let (width, height) = (8500, 8000);
        let mut png = Vec::new();
        PngEncoder::new_with_quality(&mut png, CompressionType::Fast, FilterType::NoFilter)
            .write_image(
                &vec![0; width as usize * height as usize],
                width,
                height,
                ExtendedColorType::L8,
            )
            .unwrap();
        fs::write(&path, &png).unwrap();
        let group = Group::try_from(vec![Input::file(path.clone())]).unwrap();
        let history = History::new(temp.path().join("history"));
        history
            .save(group, 0, &Arc::new(AtomicU64::new(0)))
            .unwrap();
        fs::remove_file(path).unwrap();
        let row = history.search("panorama", true).unwrap().pop().unwrap();
        assert_eq!(history.picture(row.id, 0).unwrap().bytes, png);
        assert!(
            matches!(history.prepare(row.id, Selection::All, None).unwrap().payload, Payload::Image(bytes) if bytes == png)
        );
    }

    /// A rejected stale or oversized group cannot evict valid older history.
    #[test]
    fn quota_and_capture_generation_are_atomic() {
        let temp = tempfile::tempdir().unwrap();
        let history = History {
            root: Some(temp.path().into()),
            max_groups: 2,
            max_bytes: 5,
        };
        let gate = Arc::new(AtomicU64::new(7));
        let group = |s: &str| Group::try_from(vec![Input::Text(s.into())]).unwrap();
        for s in ["ab", "cd", "ab", "ef"] {
            assert!(history.save(group(s), 7, &gate).unwrap());
        }
        assert!(history.search("cd", false).unwrap().is_empty());
        assert!(!history.save(group("stale"), 6, &gate).unwrap());
        assert!(matches!(
            history.save(group("123456"), 7, &gate),
            Err(Error::Limit)
        ));
        assert_eq!(history.search("", false).unwrap().len(), 2);
        assert!(history.save(group("12345"), 7, &gate).unwrap());
        assert_eq!(history.search("", false).unwrap().len(), 1);
        gate.store(8, Ordering::Release);
        history.remove(None).unwrap();
        assert!(!history.save(group("old"), 7, &gate).unwrap());
        assert!(history.search("", false).unwrap().is_empty());
    }

    /// Multi-item native text offers preserve ordering both alone and alongside a picture.
    #[test]
    fn multiple_text_items_restore_in_their_original_order() {
        let temp = tempfile::tempdir().unwrap();
        let history = History::new(temp.path().to_owned());
        let epoch = Arc::new(AtomicU64::new(0));
        let texts = || vec![Input::Text("first".into()), Input::Text("next".into())];
        history
            .save(Group::try_from(texts()).unwrap(), 0, &epoch)
            .unwrap();
        let id = history.search("next", false).unwrap()[0].id;
        assert!(
            matches!(history.prepare(id, Selection::All, None).unwrap().payload,
            Payload::Text(text) if text == "first\nnext")
        );
        let image = include_bytes!("../../../../../assets/icons/rtools-22.png");
        let mut mixed = texts();
        mixed.insert(1, Input::Image(image.to_vec()));
        history
            .save(Group::try_from(mixed).unwrap(), 0, &epoch)
            .unwrap();
        let id = history.search("next", true).unwrap()[0].id;
        let batch = history.prepare(id, Selection::All, None).unwrap();
        let Payload::Files(paths) = &batch.payload else {
            panic!("expected three files")
        };
        assert_eq!(fs::read(&paths[0]).unwrap(), b"first");
        assert_eq!(fs::read(&paths[1]).unwrap(), image);
        assert_eq!(fs::read(&paths[2]).unwrap(), b"next");
    }

    /// An interrupted insert rolls back the whole group and preserves previously saved history.
    #[test]
    fn storage_failure_rolls_back_all_members() {
        let temp = tempfile::tempdir().unwrap();
        let history = History::new(temp.path().to_owned());
        let epoch = Arc::new(AtomicU64::new(0));
        let first = Group::try_from(vec![Input::Text("existing".into())]).unwrap();
        history.save(first, 0, &epoch).unwrap();
        history
            .connect()
            .unwrap()
            .execute_batch(
                "CREATE TRIGGER fail_second BEFORE INSERT ON member WHEN NEW.pos=1
             BEGIN SELECT RAISE(ABORT,'simulated write failure'); END;",
            )
            .unwrap();
        let next =
            Group::try_from(vec![Input::Text("one".into()), Input::Text("two".into())]).unwrap();
        assert!(matches!(
            history.save(next, 0, &epoch),
            Err(Error::Database(_))
        ));
        let rows = history.search("", false).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].title, "existing");
    }

    /// Retiring published exports preserves referenced files and unrelated directories.
    #[test]
    fn exports_survive_reopening_until_the_clipboard_is_replaced() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("history");
        let history = History::new(root.clone());
        let epoch = Arc::new(AtomicU64::new(0));
        let png = include_bytes!("../../../../../assets/icons/rtools-22.png");
        let group =
            Group::try_from(vec![Input::Image(png.to_vec()), Input::Image(png.to_vec())]).unwrap();
        history.save(group, 0, &epoch).unwrap();
        let id = history.search("", true).unwrap()[0].id;
        let published = history.prepare(id, Selection::All, None).unwrap();
        let Payload::Files(paths) = published.payload else {
            panic!("expected exports")
        };
        let dir = published.exports.unwrap().keep();
        let other = root.join("exports/unrelated");
        fs::create_dir(&other).unwrap();
        let reopened = History::new(root);
        assert!(reopened.clean_exports(&paths[1..]).unwrap());
        assert_eq!(fs::read(&paths[0]).unwrap(), png);
        assert!(!reopened.clean_exports(&[]).unwrap());
        assert!(!dir.exists());
        assert!(other.is_dir());
    }
}
