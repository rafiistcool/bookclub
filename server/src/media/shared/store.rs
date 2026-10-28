//! Where media bytes live (docs/phase1/conventions.md §8): the filesystem now, S3 later.

use std::collections::BTreeMap;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

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

    /// The file for `key`.
    fn path(&self, key: &str) -> anyhow::Result<PathBuf> {
        check_key(key)?;
        Ok(self.root.join(key))
    }
}

/// Keys are relative paths of plain components; anything else (`..`, absolute paths, empty
/// keys) is refused by every store.
fn check_key(key: &str) -> anyhow::Result<()> {
    let plain = Path::new(key)
        .components()
        .all(|component| matches!(component, Component::Normal(_)));
    if key.is_empty() || !plain {
        bail!("invalid media key {key:?}");
    }
    Ok(())
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

/// Objects in memory, for tests. Keys are checked like [`FsMediaStore`]'s.
#[derive(Default)]
pub struct MemMediaStore {
    objects: Mutex<BTreeMap<String, (Bytes, String)>>,
}

impl MemMediaStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn objects(&self) -> MutexGuard<'_, BTreeMap<String, (Bytes, String)>> {
        self.objects
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Every stored key, sorted.
    pub fn keys(&self) -> Vec<String> {
        self.objects().keys().cloned().collect()
    }

    /// The content type `key` was stored with.
    pub fn content_type(&self, key: &str) -> Option<String> {
        self.objects()
            .get(key)
            .map(|(_, content_type)| content_type.clone())
    }
}

#[async_trait::async_trait]
impl MediaStore for MemMediaStore {
    async fn put(&self, key: &str, bytes: Bytes, content_type: &str) -> anyhow::Result<()> {
        check_key(key)?;
        self.objects()
            .insert(key.to_owned(), (bytes, content_type.to_owned()));
        Ok(())
    }

    async fn get(&self, key: &str) -> anyhow::Result<Option<Bytes>> {
        check_key(key)?;
        Ok(self.objects().get(key).map(|(bytes, _)| bytes.clone()))
    }

    async fn delete_prefix(&self, prefix: &str) -> anyhow::Result<()> {
        check_key(prefix)?;
        let below = format!("{prefix}/");
        self.objects()
            .retain(|key, _| key != prefix && !key.starts_with(&below));
        Ok(())
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
    async fn fs_store_round_trips_binary_objects_under_nested_keys() {
        let dir = tempfile::tempdir().unwrap();
        let store = FsMediaStore::new(dir.path().join("does/not/exist/yet"));
        let bytes: Vec<u8> = (0..=255).cycle().take(70_000).collect();
        let key = "avatars/ab/abcdefghijklmnopqrstuvwxyz/a";
        store
            .put(key, Bytes::from(bytes.clone()), "image/jpeg")
            .await
            .unwrap();
        assert_eq!(store.get(key).await.unwrap().unwrap(), bytes);
        // A prefix that is one file deletes just that file.
        store.delete_prefix(key).await.unwrap();
        assert_eq!(store.get(key).await.unwrap(), None);
        // Deleting a prefix that never existed is fine.
        store.delete_prefix("covers/zz/nothing").await.unwrap();
    }

    #[tokio::test]
    async fn mem_store_behaves_like_the_fs_store() {
        let store = MemMediaStore::new();
        store
            .put("covers/ab/abc/l", Bytes::from_static(b"l"), "image/webp")
            .await
            .unwrap();
        store
            .put("covers/ab/abc/s", Bytes::from_static(b"s"), "image/webp")
            .await
            .unwrap();
        store
            .put(
                "covers/ab/abcd/l",
                Bytes::from_static(b"other"),
                "image/webp",
            )
            .await
            .unwrap();
        assert_eq!(store.get("covers/ab/abc/l").await.unwrap().unwrap(), "l");
        assert_eq!(
            store.content_type("covers/ab/abc/s").as_deref(),
            Some("image/webp")
        );
        store.delete_prefix("covers/ab/abc").await.unwrap();
        assert_eq!(
            store.keys(),
            ["covers/ab/abcd/l"],
            "a sibling prefix survives"
        );
        assert!(store.put("../x", Bytes::new(), "image/webp").await.is_err());
        assert!(store.get("/etc/passwd").await.is_err());
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
