//! Remember ordinary window dimensions in logical pixels, independently of core settings.
use super::{DEFAULT_WINDOW_SIZE, MIN_WINDOW_SIZE};
use anyhow::{Context, Result};
use clash_of_rust::config::{Store, atomic_write};
use iced::Size;
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
};
use tokio::sync::Mutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct Dimensions {
    width: u16,
    height: u16,
}

impl Dimensions {
    fn from_size(size: Size) -> Option<Self> {
        let width = size.width.round();
        let height = size.height.round();
        if !width.is_finite()
            || !height.is_finite()
            || width < MIN_WINDOW_SIZE.width
            || height < MIN_WINDOW_SIZE.height
            || width > 16384.0
            || height > 16384.0
        {
            return None;
        }
        Some(Self {
            width: width as u16,
            height: height as u16,
        })
    }

    fn packed(self) -> u32 {
        u32::from(self.width) | (u32::from(self.height) << 16)
    }

    fn unpack(value: u32) -> Self {
        Self {
            width: value as u16,
            height: (value >> 16) as u16,
        }
    }

    fn size(self) -> Size {
        Size::new(f32::from(self.width), f32::from(self.height))
    }
}

struct Inner {
    normal: AtomicU32,
    saved: AtomicU32,
    path: Option<PathBuf>,
    writer: Mutex<()>,
}

#[derive(Clone)]
pub(super) struct State(Arc<Inner>);

impl Default for State {
    fn default() -> Self {
        Self::new(DEFAULT_WINDOW_SIZE, None)
    }
}

impl State {
    fn new(size: Size, path: Option<PathBuf>) -> Self {
        let dimensions = Dimensions::from_size(size).expect("valid initial window size");
        Self(Arc::new(Inner {
            normal: AtomicU32::new(dimensions.packed()),
            saved: AtomicU32::new(dimensions.packed()),
            path,
            writer: Mutex::new(()),
        }))
    }

    pub(super) fn load(store: &Store) -> Self {
        let path = store.root.join("window-state.json");
        let stored = std::fs::metadata(&path)
            .ok()
            .filter(|metadata| metadata.len() <= 1024)
            .and_then(|_| std::fs::read(&path).ok())
            .and_then(|bytes| serde_json::from_slice::<Dimensions>(&bytes).ok())
            .and_then(|dimensions| Dimensions::from_size(dimensions.size()));
        Self::new(
            stored.map_or(DEFAULT_WINDOW_SIZE, Dimensions::size),
            Some(path),
        )
    }

    pub(super) fn size(&self) -> Size {
        Dimensions::unpack(self.0.normal.load(Ordering::Acquire)).size()
    }

    pub(super) fn capture(&self, size: Size, maximized: bool, minimized: Option<bool>) -> bool {
        if maximized || minimized == Some(true) {
            return false;
        }
        let Some(dimensions) = Dimensions::from_size(size) else {
            return false;
        };
        self.0.normal.swap(dimensions.packed(), Ordering::AcqRel) != dimensions.packed()
    }

    pub(super) async fn save(&self) -> Result<()> {
        let Some(path) = &self.0.path else {
            return Ok(());
        };
        let _writer = self.0.writer.lock().await;
        loop {
            // Always read the newest dimensions after acquiring the writer.
            // Delayed saves and exit flushing must never restore stale data.
            let value = self.0.normal.load(Ordering::Acquire);
            if value == self.0.saved.load(Ordering::Acquire) {
                return Ok(());
            }
            let path = path.clone();
            tokio::task::spawn_blocking(move || {
                atomic_write(&path, &serde_json::to_vec(&Dimensions::unpack(value))?)
            })
            .await
            .context("保存窗口尺寸任务失败")??;
            self.0.saved.store(value, Ordering::Release);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_valid_ordinary_logical_dimensions_replace_the_saved_size() {
        let state = State::default();
        let normal = Size::new(900.0, 500.0);
        assert!(state.capture(normal, false, Some(false)));
        assert!(!state.capture(Size::new(1920.0, 1080.0), true, Some(false)));
        assert!(!state.capture(Size::ZERO, false, Some(true)));
        for invalid in [
            Size::new(f32::NAN, 700.0),
            Size::new(950.0, f32::INFINITY),
            Size::new(16385.0, 700.0),
            Size::new(799.0, 450.0),
        ] {
            assert!(!state.capture(invalid, false, Some(false)));
        }
        assert_eq!(state.size(), normal);
        // Unsupported minimized queries on Unix do not prevent ordinary saves.
        assert!(state.capture(Size::new(1000.0, 650.0), false, None));
    }

    #[test]
    fn dpi_pixel_rounding_preserves_logical_dimensions_including_the_minimum() {
        for desired in [Size::new(943.0, 631.0), MIN_WINDOW_SIZE] {
            for scale in [1.0, 1.25, 1.5, 1.75, 2.0] {
                let reported = Size::new(
                    (desired.width * scale).round() / scale,
                    (desired.height * scale).round() / scale,
                );
                let state = State::default();
                state.capture(reported, false, Some(false));
                assert_eq!(state.size(), desired);
            }
        }
    }

    #[tokio::test]
    async fn relaunch_restores_last_normal_dimensions_without_changing_core_settings() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::at(directory.path().to_owned()).unwrap();
        let settings = store.load_settings().unwrap();
        let before = std::fs::read(store.root.join("settings.json")).unwrap();
        let state = State::load(&store);
        assert_eq!(state.size(), DEFAULT_WINDOW_SIZE);
        assert!(state.capture(Size::new(900.0, 500.0), false, Some(false)));
        state.save().await.unwrap();
        assert_eq!(State::load(&store).size(), Size::new(900.0, 500.0));
        assert_eq!(
            std::fs::read(store.root.join("settings.json")).unwrap(),
            before
        );
        assert_eq!(store.load_settings().unwrap().secret, settings.secret);
    }

    #[tokio::test]
    async fn concurrent_saves_cannot_overwrite_the_latest_normal_size() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::at(directory.path().to_owned()).unwrap();
        let state = State::load(&store);
        state.capture(Size::new(1000.0, 650.0), false, None);
        state.save().await.unwrap();
        state.capture(Size::new(1200.0, 800.0), false, None);
        let delayed = state.clone();
        let writer = tokio::spawn(async move { delayed.save().await });
        state.capture(DEFAULT_WINDOW_SIZE, false, None);
        state.save().await.unwrap();
        writer.await.unwrap().unwrap();
        assert_eq!(State::load(&store).size(), DEFAULT_WINDOW_SIZE);
    }

    #[test]
    fn invalid_or_oversized_geometry_does_not_prevent_startup() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::at(directory.path().to_owned()).unwrap();
        for bytes in [
            b"{broken".as_slice(),
            br#"{"width":0,"height":0}"#,
            br#"{"width":20000,"height":700}"#,
            &[b' '; 1025],
        ] {
            std::fs::write(store.root.join("window-state.json"), bytes).unwrap();
            assert_eq!(State::load(&store).size(), DEFAULT_WINDOW_SIZE);
        }
    }
}
