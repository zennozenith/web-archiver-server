//! Filesystem storage for blobs too big (or too numerous) to keep inline in SQLite.
//!
//! Layout:  <root>/ab/cd/<64-hex blake3 of the ORIGINAL bytes>
//!          <root>/tmp/            in-flight writes, renamed into place when complete
//!
//! `content.storage_ref` holds the *relative* path ("ab/cd/abcd…"), so the root can be moved
//! without touching the database. File bytes are exactly what `data` would have held
//! (possibly brotli/gzip compressed); `content.compression_id` says how.

use std::{
    path::{Component, Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime},
};

use tokio::{fs, io::AsyncWriteExt};

use crate::error::{AppError, Result};

use super::hash::Blake3Hash;

static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone)]
pub struct BlobFiles {
    root: Arc<Path>,
    /// Stored blobs larger than this go to disk; 0 = everything on disk.
    inline_threshold: usize,
}

impl BlobFiles {
    pub async fn open(root: impl Into<PathBuf>, inline_threshold: usize) -> Result<Self> {
        let root = root.into();
        fs::create_dir_all(root.join("tmp")).await?;
        let root = fs::canonicalize(&root).await?;
        Ok(Self {
            root: Arc::from(root),
            inline_threshold,
        })
    }

    pub const fn inline_threshold(&self) -> usize {
        self.inline_threshold
    }

    /// Two levels of fan-out keep directories small (256 * 256 buckets).
    #[expect(clippy::string_slice, reason = "hash is 32 bytes")]
    pub fn rel_path(hash: &Blake3Hash) -> String {
        let hex = hash.to_hex();
        format!("{}/{}/{}", &hex[0..2], &hex[2..4], hex)
    }

    /// Refuses absolute paths and `..` so a bad `storage_ref` can never escape the root.
    fn resolve(&self, rel: &str) -> Result<PathBuf> {
        let p = Path::new(rel);
        if rel.is_empty() || p.components().any(|c| !matches!(c, Component::Normal(_))) {
            return Err(AppError::internal(format!("invalid storage_ref: {rel:?}")));
        }
        Ok(self.root.join(p))
    }

    /// Atomically write `data` (write to tmp, fsync, rename). Returns the `storage_ref`.
    /// Always overwrites, so a leftover file from a failed earlier insert can't disagree
    /// with the compression recorded in the database.
    pub async fn put(&self, hash: &Blake3Hash, data: Vec<u8>) -> Result<String> {
        let rel = Self::rel_path(hash);
        let dest = self.resolve(&rel)?;
        let tmp = self.root.join("tmp").join(format!(
            "{}.{}.{}",
            hash,
            std::process::id(),
            TMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));

        let res = async {
            let mut f = fs::File::create(&tmp).await?;
            f.write_all(&data).await?;
            f.sync_all().await?;
            drop(f);
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent).await?;
            }
            fs::rename(&tmp, &dest).await
        }
        .await;

        if let Err(e) = res {
            let _err = fs::remove_file(&tmp).await;
            return Err(e.into());
        }
        Ok(rel)
    }

    pub async fn open_file(&self, rel: &str) -> Result<fs::File> {
        Ok(fs::File::open(self.resolve(rel)?).await?)
    }

    pub async fn read(&self, rel: &str) -> Result<Vec<u8>> {
        Ok(fs::read(self.resolve(rel)?).await?)
    }

    /// Idempotent: a missing file is fine.
    pub async fn remove(&self, rel: &str) -> Result<()> {
        match fs::remove_file(self.resolve(rel)?).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }

    /// Delete abandoned temp files (crashed mid-write). Best effort.
    pub async fn clean_tmp(&self, older_than: Duration) -> usize {
        let mut removed: usize = 0;
        let Ok(mut dir) = fs::read_dir(self.root.join("tmp")).await else {
            return 0;
        };
        while let Ok(Some(entry)) = dir.next_entry().await {
            let old = entry
                .metadata()
                .await
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| SystemTime::now().duration_since(t).ok())
                .is_some_and(|age| age > older_than);
            if old && fs::remove_file(entry.path()).await.is_ok() {
                removed = removed.saturating_add(1);
            }
        }
        removed
    }
}
