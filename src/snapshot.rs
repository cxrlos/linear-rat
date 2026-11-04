//! The snapshot cache: the store of the last session as JSON under the XDG
//! cache dir, one file per workspace and viewer, so the next launch paints
//! before any request returns (ADR 0003). It holds workspace data and never
//! the API key, so files are written with mode 0600.

use std::fs::{self, DirBuilder, OpenOptions};
use std::io::{ErrorKind, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use anyhow::{Context, bail};
use etcetera::{BaseStrategy, choose_base_strategy};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use crate::app::Store;

/// The shape of the snapshot. A file of any other version is thrown away,
/// so the store's types can change without migrations: bump this when they
/// do.
pub const VERSION: u32 = 1;

/// Names the snapshot file the last session wrote.
const LAST_SESSION: &str = "last-session";

/// Where snapshots live and how they are read and written. Clones share the
/// order of [`SnapshotCache::save_in_order`].
#[derive(Debug, Clone)]
pub struct SnapshotCache {
    dir: PathBuf,
    /// The last place in the order handed out by [`SnapshotCache::next_save`].
    issued: Arc<AtomicU64>,
    /// The place of the last save written. Held while writing, so saves on
    /// other threads never interleave.
    written: Arc<Mutex<u64>>,
}

/// A save's place in the order the app asked for saves in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct SaveTicket(u64);

/// What a snapshot gives back.
#[derive(Debug, Clone, PartialEq)]
pub struct Saved {
    pub store: Store,
    /// When the session that wrote it last synced, by the local clock.
    pub synced_at: Option<Timestamp>,
}

#[derive(Serialize)]
struct FileRef<'a> {
    version: u32,
    workspace_id: &'a str,
    viewer_id: &'a str,
    synced_at: Option<Timestamp>,
    store: &'a Store,
}

#[derive(Deserialize)]
struct File {
    version: u32,
    workspace_id: String,
    viewer_id: String,
    synced_at: Option<Timestamp>,
    store: Store,
}

impl SnapshotCache {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            issued: Arc::default(),
            written: Arc::default(),
        }
    }

    /// The next place in the order saves are written in.
    pub fn next_save(&self) -> SaveTicket {
        SaveTicket(self.issued.fetch_add(1, Ordering::SeqCst) + 1)
    }

    /// [`SnapshotCache::save`], unless a save with a later `ticket` has been
    /// written already, so a save that runs late never puts an older store
    /// back.
    pub fn save_in_order(
        &self,
        ticket: SaveTicket,
        store: &Store,
        synced_at: Option<Timestamp>,
    ) -> anyhow::Result<()> {
        // A writer that panicked left at most a stray temporary file, since
        // the rename is atomic, so the order it guards is still good.
        let mut written = self.written.lock().unwrap_or_else(PoisonError::into_inner);
        if *written >= ticket.0 {
            return Ok(());
        }
        *written = ticket.0;
        self.save(store, synced_at)
    }

    /// `$XDG_CACHE_HOME/lrat` (by default `~/.cache/lrat`, on macOS too).
    pub fn in_cache_dir() -> anyhow::Result<Self> {
        let base = choose_base_strategy().context("finding the home directory")?;
        Ok(Self::new(base.cache_dir().join("lrat")))
    }

    /// The store the last session saved, or `None` when there is none or it
    /// cannot be used: unreadable, of another version, or naming another
    /// workspace or viewer than its file. Why is logged.
    pub fn load(&self) -> Option<Saved> {
        match self.read() {
            Ok(saved) => saved,
            Err(error) => {
                tracing::warn!("discarding the snapshot: {error:#}");
                None
            }
        }
    }

    /// Writes `store` as the snapshot of its workspace and viewer, and makes
    /// it the one the next launch loads. A store that has not loaded its
    /// viewer and workspace yet has nothing to name it by and is skipped.
    pub fn save(&self, store: &Store, synced_at: Option<Timestamp>) -> anyhow::Result<()> {
        let (Some(workspace), Some(viewer)) = (store.workspace(), store.viewer()) else {
            return Ok(());
        };
        let name = file_name(&workspace.id, &viewer.id)?;
        let file = FileRef {
            version: VERSION,
            workspace_id: &workspace.id,
            viewer_id: &viewer.id,
            synced_at,
            store,
        };
        let json = serde_json::to_vec(&file).context("encoding the snapshot")?;
        DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&self.dir)
            .with_context(|| format!("creating {}", self.dir.display()))?;
        write_atomically(&self.dir.join(&name), &json)?;
        write_atomically(&self.dir.join(LAST_SESSION), name.as_bytes())
    }

    fn read(&self) -> anyhow::Result<Option<Saved>> {
        let pointer = self.dir.join(LAST_SESSION);
        let name = match fs::read_to_string(&pointer) {
            Ok(name) => name,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(error).with_context(|| format!("reading {}", pointer.display()));
            }
        };
        let path = self.dir.join(name.trim());
        let text = fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
        let file: File = serde_json::from_slice(&text)
            .with_context(|| format!("decoding {}", path.display()))?;
        if file.version != VERSION {
            bail!(
                "{} is version {}, not {VERSION}",
                path.display(),
                file.version
            );
        }
        let matches = file_name(&file.workspace_id, &file.viewer_id)? == name.trim()
            && file.store.workspace().map(|w| w.id.as_str()) == Some(file.workspace_id.as_str())
            && file.store.viewer().map(|v| v.id.as_str()) == Some(file.viewer_id.as_str());
        if !matches {
            bail!(
                "{} holds another workspace or viewer than it names",
                path.display()
            );
        }
        Ok(Some(Saved {
            store: file.store,
            synced_at: file.synced_at,
        }))
    }
}

/// `snapshot.<workspace>.<viewer>.json`. Ids come from Linear, so anything
/// but letters, digits, `-` and `_` is refused rather than put in a path.
fn file_name(workspace_id: &str, viewer_id: &str) -> anyhow::Result<String> {
    let safe = |id: &str| {
        !id.is_empty()
            && id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    };
    if !safe(workspace_id) || !safe(viewer_id) {
        bail!("a workspace or viewer id is not safe in a file name");
    }
    Ok(format!("snapshot.{workspace_id}.{viewer_id}.json"))
}

/// Writes `contents` to a temporary file with mode 0600 next to `path`, then
/// renames it over `path`, so a reader sees the old file or the new one and
/// never half of one.
fn write_atomically(path: &Path, contents: &[u8]) -> anyhow::Result<()> {
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(format!(".tmp-{}", std::process::id()));
    let temporary = PathBuf::from(temporary);
    let written = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&temporary)
        .and_then(|mut file| {
            // A temporary file left by a crash keeps its old mode on open.
            file.set_permissions(fs::Permissions::from_mode(0o600))?;
            file.write_all(contents)?;
            file.sync_all()
        })
        .and_then(|()| fs::rename(&temporary, path));
    if let Err(error) = written {
        // Best effort: the rename failed or never happened.
        let _ = fs::remove_file(&temporary);
        return Err(error).with_context(|| format!("writing {}", path.display()));
    }
    Ok(())
}
