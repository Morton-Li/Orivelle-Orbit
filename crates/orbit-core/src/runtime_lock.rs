use crate::{
    error::{Result, fail},
    store::{private_directory, private_file},
};
use fs2::FileExt;
use std::{
    fs::{File, OpenOptions},
    path::Path,
};

/// Held for the entire GUI process lifetime, before opening SQLite or any vault.
/// It deliberately has a different path from the helper/update coordination lock.
pub struct RuntimeLock {
    file: File,
}
impl RuntimeLock {
    pub fn acquire(root: &Path) -> Result<Self> {
        if !root.is_absolute() {
            return Err(fail("local_storage_unavailable"));
        }
        private_directory(root)?;
        let locks = root.join("locks");
        private_directory(&locks)?;
        let path = locks.join("gui-runtime.lock");
        if let Ok(metadata) = std::fs::symlink_metadata(&path) {
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(fail("local_storage_unavailable"));
            }
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .inspect_err(|error| {
                eprintln!("Orbit could not open the runtime lock ({}).", error.kind())
            })
            .map_err(|_| fail("local_storage_unavailable"))?;
        private_file(&path)?;
        file.try_lock_exclusive()
            .map_err(|_| fail("instance_already_running"))?;
        Ok(Self { file })
    }
}
impl Drop for RuntimeLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_one_gui_can_open_database_and_start_recovery() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("data");
        let first = RuntimeLock::acquire(&root).unwrap();
        let started = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        let other = started.clone();
        let second_root = root.clone();
        let thread = std::thread::spawn(move || match RuntimeLock::acquire(&second_root) {
            Ok(_lock) => {
                other.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                crate::store::Store::open(&second_root).unwrap();
            }
            Err(error) => assert_eq!(error.code, "instance_already_running"),
        });
        thread.join().unwrap();
        assert_eq!(started.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert!(!root.join("orbit.sqlite3").exists());
        drop(first);
        let _reopened = RuntimeLock::acquire(&root).unwrap();
        crate::store::Store::open(&root).unwrap();
    }
}
