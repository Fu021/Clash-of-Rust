//! A GUI lifetime lock is independent from the core owner. It lets a console
//! stop keep an attached window usable, and lets updates close both owners.
use crate::config::{Store, atomic_write};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::{fs::File, path::PathBuf, sync::OnceLock};

static GUI: OnceLock<(PathBuf, String)> = OnceLock::new();
#[derive(Serialize, Deserialize)]
struct Record {
    pid: u32,
    nonce: String,
}
pub struct GuiSession {
    store: Store,
    nonce: String,
    _lock: File,
}
impl GuiSession {
    pub fn register(store: Store) -> Result<Self> {
        let lock = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(store.root.join("gui-session.lock"))?;
        lock.try_lock()?;
        let nonce = uuid::Uuid::new_v4().to_string();
        atomic_write(
            &store.root.join("gui-session.json"),
            &serde_json::to_vec(&Record {
                pid: std::process::id(),
                nonce: nonce.clone(),
            })?,
        )?;
        let _ = std::fs::remove_file(store.root.join("gui-exit"));
        let _ = GUI.set((store.root.clone(), nonce.clone()));
        Ok(Self {
            store,
            nonce,
            _lock: lock,
        })
    }
}
impl Drop for GuiSession {
    fn drop(&mut self) {
        if record(&self.store)
            .ok()
            .flatten()
            .is_some_and(|r| r.nonce == self.nonce)
        {
            let _ = std::fs::remove_file(self.store.root.join("gui-session.json"));
            let _ = std::fs::remove_file(self.store.root.join("gui-exit"));
        }
    }
}
fn record(store: &Store) -> Result<Option<Record>> {
    let path = store.root.join("gui-session.json");
    if !path.exists() {
        return Ok(None);
    }
    let bytes = std::fs::read(path)?;
    ensure!(bytes.len() <= 4096, "桌面运行状态无效");
    let record: Record = serde_json::from_slice(&bytes)?;
    ensure!(
        record.pid > 1 && record.nonce.len() == 36,
        "桌面运行状态无效"
    );
    Ok(Some(record))
}
pub fn gui_pid(store: &Store) -> Result<Option<u32>> {
    if !store.root.join("gui-session.lock").exists() {
        return Ok(None);
    }
    let lock = File::options()
        .read(true)
        .write(true)
        .open(store.root.join("gui-session.lock"))?;
    if lock.try_lock().is_ok() {
        return Ok(None);
    }
    Ok(record(store)?.map(|record| record.pid))
}
pub fn request_gui_exit(store: &Store) -> Result<()> {
    if gui_pid(store)?.is_some()
        && let Some(record) = record(store)?
    {
        atomic_write(&store.root.join("gui-exit"), record.nonce.as_bytes())?;
    }
    Ok(())
}
pub fn gui_exit_requested() -> bool {
    let Some((root, nonce)) = GUI.get() else {
        return false;
    };
    let path = root.join("gui-exit");
    if std::fs::read_to_string(&path).is_ok_and(|value| value == *nonce) {
        let _ = std::fs::remove_file(path);
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gui_lifetime_and_exit_requests_do_not_depend_on_the_core_owner() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::at(root.path().to_owned()).unwrap();
        assert_eq!(gui_pid(&store).unwrap(), None);
        let gui = GuiSession::register(store.clone()).unwrap();
        assert_eq!(gui_pid(&store).unwrap(), Some(std::process::id()));
        request_gui_exit(&store).unwrap();
        assert_eq!(
            std::fs::read_to_string(store.root.join("gui-exit")).unwrap(),
            gui.nonce
        );
        drop(gui);
        assert_eq!(gui_pid(&store).unwrap(), None);
    }
}
