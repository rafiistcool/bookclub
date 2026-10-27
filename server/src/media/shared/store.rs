//! Where media bytes live (docs/phase1/conventions.md §8): the filesystem now, S3 later.

use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, bail};
use bytes::Bytes;

use crate::auth::random;

/// Stores media objects by key (`media::shared::urls::object_key`).
#[async_trait::async_trait]
pub trait MediaStore: Send + Sync {
    async fn put(&self, key: &str, bytes: Bytes, content_type: &str) -> anyhow::Result<()>;
    /// `Ok(None)` when the object does not exist.
    async fn get(&self, key: &str) -> anyhow::Result<Option<Bytes>>;
    /// Deletes every object under `prefix` (a `storage_prefix`). Missing objects are fine.
    async fn delete_prefix(&self, prefix: &str) -> anyhow::Result<()>;
}

/// Objects as files under `MEDIA_DIR`. Writes go to a temporary file first and are renamed into
/// place, so a reader never sees half an object.
pub struct FsMediaStore {
    root: PathBuf,
}

impl FsMediaStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The file for `key`. Keys are relative paths of plain components; anything else (`..`,
    /// absolute paths, empty keys) is refused.
    fn path(&self, key: &str) -> anyhow::Result<PathBuf> {
        let relative = Path::new(key);
        let plain = relative
            .components()
            .all(|component| matches!(component, Component::Normal(_)));
        if key.is_empty() || !plain {
            bail!("invalid media key {key:?}");
        }
        Ok(self.root.join(relative))
    }
}

#[async_trait::async_trait]
impl MediaStore for FsMediaStore {
    async fn put(&self, key: &str, bytes: Bytes, _content_type: &str) -> anyhow::Result<()> {
        let path = self.path(key)?;
        let dir = path.parent().context("media key has no parent directory")?;
        tokio::fs::create_dir_all(dir)
            .await
            .with_context(|| format!("cannot create {}", dir.display()))?;
        let mut suffix = [0u8; 8];
        random::fill(&mut suffix);
        let file_name = path.file_name().context("media key has no file name")?;
        let temp = dir.join(format!(
            ".{}.{:016x}.tmp",
            file_name.to_string_lossy(),
            u64::from_le_bytes(suffix)
        ));
        tokio::fs::write(&temp, &bytes)
            .await
            .with_context(|| format!("cannot write {}", temp.display()))?;
        if let Err(error) = tokio::fs::rename(&temp, &path).await {
            let _ = tokio::fs::remove_file(&temp).await;
            return Err(error)
                .with_context(|| format!("cannot move media into {}", path.display()));
        }
        Ok(())
    }

    async fn get(&self, key: &str) -> anyhow::Result<Option<Bytes>> {
        let path = self.path(key)?;
        match tokio::fs::read(&path).await {
            Ok(bytes) => Ok(Some(Bytes::from(bytes))),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error).with_context(|| format!("cannot read {}", path.display())),
        }
    }

    async fn delete_prefix(&self, prefix: &str) -> anyhow::Result<()> {
        let path = self.path(prefix)?;
        let result = match tokio::fs::metadata(&path).await {
            Ok(meta) if meta.is_dir() => tokio::fs::remove_dir_all(&path).await,
            Ok(_) => tokio::fs::remove_file(&path).await,
            Err(error) => Err(error),
        };
        match result {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error).with_context(|| format!("cannot delete {}", path.display())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn put_get_and_delete_a_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let store = FsMediaStore::new(dir.path());
        store
            .put(
                "covers/ab/abc/l",
                Bytes::from_static(b"large"),
                "image/webp",
            )
            .await
            .unwrap();
        store
            .put(
                "covers/ab/abc/s",
                Bytes::from_static(b"small"),
                "image/webp",
            )
            .await
            .unwrap();
        assert_eq!(
            store.get("covers/ab/abc/l").await.unwrap().unwrap(),
            "large"
        );
        assert_eq!(store.get("covers/ab/abc/x").await.unwrap(), None);

        // Overwrites replace the object; no temporary files stay behind.
        store
            .put(
                "covers/ab/abc/l",
                Bytes::from_static(b"newer"),
                "image/webp",
            )
            .await
            .unwrap();
        assert_eq!(
            store.get("covers/ab/abc/l").await.unwrap().unwrap(),
            "newer"
        );
        let names: Vec<_> = std::fs::read_dir(dir.path().join("covers/ab/abc"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names.len(), 2, "{names:?}");

        store.delete_prefix("covers/ab/abc").await.unwrap();
        assert_eq!(store.get("covers/ab/abc/s").await.unwrap(), None);
        store.delete_prefix("covers/ab/abc").await.unwrap();
    }

    #[tokio::test]
    async fn refuses_keys_that_leave_the_root() {
        let dir = tempfile::tempdir().unwrap();
        let store = FsMediaStore::new(dir.path().join("media"));
        for key in ["../escape", "/etc/passwd", "", "covers/../../x", "./a"] {
            assert!(
                store.put(key, Bytes::new(), "image/webp").await.is_err(),
                "{key}"
            );
            assert!(store.get(key).await.is_err(), "{key}");
            assert!(store.delete_prefix(key).await.is_err(), "{key}");
        }
    }
}
