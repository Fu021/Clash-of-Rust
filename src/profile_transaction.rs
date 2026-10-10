//! Disk-backed subscription commits. An interrupted commit is rolled back before
//! settings/profiles are loaded on the next launch.
use crate::config::{atomic_write, persist_file};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

const JOURNAL: &str = "profile-transaction.json";
#[derive(Serialize, Deserialize)]
struct Entry {
    target: PathBuf,
    existed: bool,
    #[serde(default)]
    remove: bool,
}
#[derive(Serialize, Deserialize)]
struct Journal {
    directory: String,
    entries: Vec<Entry>,
}

pub(crate) struct Transaction {
    root: PathBuf,
    directory: PathBuf,
    temporary: Option<tempfile::TempDir>,
    journal: Journal,
    prepared: bool,
}
fn allowed(path: &Path) -> bool {
    if ["settings.json", "profiles.json", "runtime/config.yaml"]
        .iter()
        .any(|target| path == Path::new(target))
    {
        return true;
    }
    path.parent() == Some(Path::new("profiles"))
        && path.extension().is_some_and(|s| s == "yaml")
        && path
            .file_stem()
            .and_then(|s| s.to_str())
            .is_some_and(|s| uuid::Uuid::parse_str(s).is_ok())
}
fn copy_atomic(source: &Path, target: &Path) -> Result<()> {
    let parent = target.parent().context("无效配置路径")?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    std::io::copy(&mut fs::File::open(source)?, &mut temporary)?;
    temporary.as_file().sync_all()?;
    persist_file(temporary, target)
        .with_context(|| format!("提交配置文件失败：{}", target.display()))
}
impl Transaction {
    pub(crate) fn new(root: &Path) -> Result<Self> {
        ensure!(
            !root.join(JOURNAL).exists(),
            "上次配置提交未恢复，请重新启动客户端"
        );
        let temporary = tempfile::Builder::new()
            .prefix(".profile-transaction-")
            .tempdir_in(root)?;
        let directory = temporary.path().to_owned();
        let journal = Journal {
            directory: directory
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            entries: vec![],
        };
        Ok(Self {
            root: root.into(),
            directory,
            temporary: Some(temporary),
            journal,
            prepared: false,
        })
    }
    pub(crate) fn stage(&mut self, target: &Path, bytes: &[u8]) -> Result<()> {
        self.stage_entry(target, Some(bytes))
    }
    pub(crate) fn stage_remove(&mut self, target: &Path) -> Result<()> {
        self.stage_entry(target, None)
    }
    fn stage_entry(&mut self, target: &Path, bytes: Option<&[u8]>) -> Result<()> {
        ensure!(!self.prepared, "配置事务已经准备完成");
        let relative = target
            .strip_prefix(&self.root)
            .context("配置路径不在数据目录内")?;
        ensure!(allowed(relative), "无效的配置事务目标");
        ensure!(
            !self
                .journal
                .entries
                .iter()
                .any(|entry| entry.target == relative),
            "重复的配置事务目标"
        );
        ensure!(self.journal.entries.len() < 4, "配置事务目标过多");
        fs::create_dir_all(target.parent().context("无效配置路径")?)?;
        let existed = match fs::metadata(target) {
            Ok(metadata) => {
                ensure!(metadata.is_file(), "配置目标不是普通文件");
                true
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => return Err(error.into()),
        };
        let index = self.journal.entries.len();
        if existed {
            let backup = self.directory.join(format!("{index}.old"));
            fs::copy(target, &backup)?;
            fs::OpenOptions::new()
                .write(true)
                .open(backup)?
                .sync_all()?;
        }
        if let Some(bytes) = bytes {
            let mut staged = fs::File::create(self.directory.join(format!("{index}.new")))?;
            staged.write_all(bytes)?;
            staged.sync_all()?;
        }
        self.journal.entries.push(Entry {
            target: relative.into(),
            existed,
            remove: bytes.is_none(),
        });
        Ok(())
    }
    pub(crate) fn prepare(&mut self) -> Result<()> {
        ensure!(
            !self.prepared && !self.journal.entries.is_empty(),
            "无效的配置事务"
        );
        // Record recovery before any live file or running core is changed.
        atomic_write(
            &self.root.join(JOURNAL),
            &serde_json::to_vec(&self.journal)?,
        )?;
        self.directory = self.temporary.take().unwrap().keep();
        self.prepared = true;
        Ok(())
    }
    pub(crate) fn commit(&mut self) -> Result<()> {
        ensure!(self.prepared, "配置事务未准备");
        for (index, entry) in self.journal.entries.iter().enumerate() {
            if entry.remove {
                remove_if_present(&self.root.join(&entry.target))?;
            } else {
                copy_atomic(
                    &self.directory.join(format!("{index}.new")),
                    &self.root.join(&entry.target),
                )?;
            }
        }
        self.finish()
    }
    pub(crate) fn rollback(&mut self) -> Result<()> {
        ensure!(self.prepared, "配置事务未准备");
        restore(&self.root, &self.directory, &self.journal)?;
        self.finish()
    }
    fn finish(&mut self) -> Result<()> {
        fs::remove_file(self.root.join(JOURNAL)).context("无法完成配置事务，保留恢复记录")?;
        self.prepared = false;
        let _ = fs::remove_dir_all(&self.directory);
        Ok(())
    }
}
fn remove_if_present(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}
fn restore(root: &Path, directory: &Path, journal: &Journal) -> Result<()> {
    // Check recovery sources before modifying any target.
    for (index, entry) in journal.entries.iter().enumerate() {
        if entry.existed {
            ensure!(
                directory.join(format!("{index}.old")).is_file(),
                "配置恢复文件缺失"
            );
        }
    }
    for (index, entry) in journal.entries.iter().enumerate().rev() {
        let target = root.join(&entry.target);
        if entry.existed {
            copy_atomic(&directory.join(format!("{index}.old")), &target)?;
        } else {
            remove_if_present(&target)?;
        }
    }
    Ok(())
}
pub(crate) fn recover(root: &Path) -> Result<bool> {
    let path = root.join(JOURNAL);
    let file = match fs::File::open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    let mut bytes = Vec::new();
    file.take(64 * 1024 + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 64 * 1024, "配置恢复记录超过限制");
    let journal: Journal = serde_json::from_slice(&bytes)?;
    ensure!(
        journal.directory.starts_with(".profile-transaction-")
            && Path::new(&journal.directory).components().count() == 1,
        "配置恢复目录无效"
    );
    ensure!(
        (1..=4).contains(&journal.entries.len())
            && journal.entries.iter().all(|entry| allowed(&entry.target)),
        "配置恢复目标无效"
    );
    let directory = root.join(&journal.directory);
    restore(root, &directory, &journal).context("恢复上次中断的订阅提交失败")?;
    fs::remove_file(path)?;
    let _ = fs::remove_dir_all(directory);
    Ok(true)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deleted_profile_is_restored_after_an_interrupted_commit() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir(root.join("profiles")).unwrap();
        let path = root
            .join("profiles")
            .join(format!("{}.yaml", uuid::Uuid::new_v4()));
        fs::write(&path, b"old profile").unwrap();
        let mut tx = Transaction::new(root).unwrap();
        tx.stage_remove(&path).unwrap();
        tx.prepare().unwrap();
        fs::remove_file(&path).unwrap();
        drop(tx);
        assert!(recover(root).unwrap());
        assert_eq!(fs::read(&path).unwrap(), b"old profile");
        let mut tx = Transaction::new(root).unwrap();
        tx.stage_remove(&path).unwrap();
        tx.prepare().unwrap();
        tx.commit().unwrap();
        assert!(!path.exists());
    }
    #[test]
    fn abandoned_partial_commit_restores_files_and_removes_new_targets() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::write(root.join("settings.json"), b"old settings").unwrap();
        let mut tx = Transaction::new(root).unwrap();
        tx.stage(&root.join("settings.json"), b"new settings")
            .unwrap();
        tx.stage(&root.join("profiles.json"), b"new profiles")
            .unwrap();
        tx.prepare().unwrap();
        fs::write(root.join("settings.json"), b"partial settings").unwrap();
        fs::write(root.join("profiles.json"), b"partial profiles").unwrap();
        drop(tx);
        assert!(recover(root).unwrap());
        assert_eq!(
            fs::read(root.join("settings.json")).unwrap(),
            b"old settings"
        );
        assert!(!root.join("profiles.json").exists());
        assert!(!recover(root).unwrap());
    }
    #[test]
    fn failed_commit_keeps_recovery_material_and_can_roll_back() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::write(root.join("settings.json"), b"old").unwrap();
        let mut tx = Transaction::new(root).unwrap();
        tx.stage(&root.join("settings.json"), b"new").unwrap();
        tx.stage(&root.join("profiles.json"), b"profiles").unwrap();
        tx.prepare().unwrap();
        fs::create_dir(root.join("profiles.json")).unwrap();
        assert!(tx.commit().is_err());
        assert_eq!(fs::read(root.join("settings.json")).unwrap(), b"new");
        assert!(root.join(JOURNAL).exists());
        fs::remove_dir(root.join("profiles.json")).unwrap();
        tx.rollback().unwrap();
        assert_eq!(fs::read(root.join("settings.json")).unwrap(), b"old");
    }
    #[test]
    fn successful_commit_and_invalid_targets() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let mut tx = Transaction::new(root).unwrap();
        assert!(tx.stage(&root.join("../outside"), b"bad").is_err());
        tx.stage(&root.join("settings.json"), b"new").unwrap();
        tx.prepare().unwrap();
        tx.commit().unwrap();
        assert_eq!(fs::read(root.join("settings.json")).unwrap(), b"new");
        assert!(!recover(root).unwrap());
    }

    #[test]
    fn runtime_path_uses_native_platform_components() {
        assert!(allowed(&Path::new("runtime").join("config.yaml")));
        assert!(!allowed(&Path::new("runtime").join("other.yaml")));
        assert!(!allowed(Path::new("runtime/../settings.json")));
    }
}
