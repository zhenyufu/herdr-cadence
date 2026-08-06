use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use fs2::FileExt;
use sha2::{Digest, Sha256};

use crate::model::Store;

pub struct StateStore {
    dir: PathBuf,
}

impl StateStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn read(&self) -> Result<Store> {
        self.with_lock(false, |store| Ok(store.clone()))
    }

    pub fn update<T>(&self, operation: impl FnOnce(&mut Store) -> Result<T>) -> Result<T> {
        self.with_lock(true, operation)
    }

    fn with_lock<T>(
        &self,
        write: bool,
        operation: impl FnOnce(&mut Store) -> Result<T>,
    ) -> Result<T> {
        fs::create_dir_all(&self.dir)?;
        let lock_path = self.dir.join("state.lock");
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(lock_path)?;
        if write {
            lock.lock_exclusive()?;
        } else {
            lock.lock_shared()?;
        }
        let mut store = self.load_unlocked()?;
        let output = operation(&mut store)?;
        if write {
            self.save_unlocked(&store)?;
        }
        FileExt::unlock(&lock)?;
        Ok(output)
    }

    fn load_unlocked(&self) -> Result<Store> {
        let path = self.dir.join("state.json");
        if !path.exists() {
            return Ok(Store {
                schema_version: 1,
                ..Store::default()
            });
        }
        let store: Store = serde_json::from_reader(
            File::open(&path).with_context(|| format!("cannot open {}", path.display()))?,
        )
        .with_context(|| format!("invalid state at {}", path.display()))?;
        anyhow::ensure!(store.schema_version == 1, "unsupported state schema");
        Ok(store)
    }

    fn save_unlocked(&self, store: &Store) -> Result<()> {
        let path = self.dir.join("state.json");
        let temp = self.dir.join(format!("state.{}.tmp", std::process::id()));
        let mut file = File::create(&temp)?;
        serde_json::to_writer_pretty(&mut file, store)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        fs::rename(&temp, &path)?;
        Ok(())
    }
}

pub fn project_key(root: &Path) -> String {
    let canonical = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let digest = Sha256::digest(canonical.to_string_lossy().as_bytes());
    digest[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persists_atomically() {
        let temp = tempfile::tempdir().unwrap();
        let state = StateStore::new(temp.path());
        state
            .update(|store| {
                store.schema_version = 1;
                Ok(())
            })
            .unwrap();
        assert_eq!(state.read().unwrap().schema_version, 1);
        assert!(!temp.path().join("state.0.tmp").exists());
    }
}
