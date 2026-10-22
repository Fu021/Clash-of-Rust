//! Remember ordinary logical dimensions and native screen position independently of core settings.
use super::runtime::Size;
use super::{DEFAULT_WINDOW_SIZE, MIN_WINDOW_SIZE};
use anyhow::{Context, Result};
use clash_of_rust::config::{Store, atomic_write};
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex as SyncMutex},
};
use tokio::sync::Mutex;

#[cfg(any(target_os = "macos", test))]
pub(super) fn fit_size(desired: Size, available: Size) -> Size {
    let fit = |value: f32, limit: f32, minimum: f32| {
        if limit.is_finite() && limit >= minimum {
            value.min(limit.floor())
        } else {
            value
        }
    };
    Size::new(
        fit(desired.width, available.width, MIN_WINDOW_SIZE.width),
        fit(desired.height, available.height, MIN_WINDOW_SIZE.height),
    )
}

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

    fn size(self) -> Size {
        Size::new(f32::from(self.width), f32::from(self.height))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct Geometry {
    #[serde(flatten)]
    dimensions: Dimensions,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    position: Option<(i32, i32)>,
}

fn valid_position(position: (i32, i32)) -> bool {
    position.0.unsigned_abs() <= 1_000_000 && position.1.unsigned_abs() <= 1_000_000
}

struct GeometryState {
    normal: Geometry,
    saved: Geometry,
}
struct Inner {
    geometry: SyncMutex<GeometryState>,
    path: Option<PathBuf>,
    writer: Mutex<()>,
}

#[derive(Clone)]
pub(super) struct State(Arc<Inner>);

impl Default for State {
    fn default() -> Self {
        Self::new(DEFAULT_WINDOW_SIZE, None, None)
    }
}

impl State {
    fn new(size: Size, position: Option<(i32, i32)>, path: Option<PathBuf>) -> Self {
        let geometry = Geometry {
            dimensions: Dimensions::from_size(size).expect("valid initial window size"),
            position: position.filter(|p| valid_position(*p)),
        };
        Self(Arc::new(Inner {
            geometry: SyncMutex::new(GeometryState {
                normal: geometry,
                saved: geometry,
            }),
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
            .and_then(|bytes| serde_json::from_slice::<Geometry>(&bytes).ok())
            .filter(|geometry| Dimensions::from_size(geometry.dimensions.size()).is_some());
        Self::new(
            stored.map_or(DEFAULT_WINDOW_SIZE, |g| g.dimensions.size()),
            stored.and_then(|g| g.position),
            Some(path),
        )
    }

    pub(super) fn size(&self) -> Size {
        self.0.geometry.lock().unwrap().normal.dimensions.size()
    }

    pub(super) fn position(&self) -> Option<(i32, i32)> {
        self.0.geometry.lock().unwrap().normal.position
    }

    pub(super) fn dirty(&self) -> bool {
        let geometry = self.0.geometry.lock().unwrap();
        geometry.normal != geometry.saved
    }

    pub(super) fn capture(&self, size: Size, maximized: bool, minimized: Option<bool>) -> bool {
        if maximized || minimized == Some(true) {
            return false;
        }
        let Some(dimensions) = Dimensions::from_size(size) else {
            return false;
        };
        let mut geometry = self.0.geometry.lock().unwrap();
        let changed = geometry.normal.dimensions != dimensions;
        geometry.normal.dimensions = dimensions;
        changed
    }

    pub(super) fn capture_position(
        &self,
        position: Option<(i32, i32)>,
        maximized: bool,
        minimized: bool,
        fullscreen: bool,
    ) -> bool {
        // Wayland may not expose or permit positioning; do not replace a useful
        // saved position with a fabricated origin. Maximized coordinates also
        // must not replace the last ordinary window position.
        if maximized || minimized || fullscreen {
            return false;
        }
        let Some(position) = position.filter(|p| valid_position(*p)) else {
            return false;
        };
        let mut geometry = self.0.geometry.lock().unwrap();
        let changed = geometry.normal.position != Some(position);
        geometry.normal.position = Some(position);
        changed
    }

    pub(super) async fn save(&self) -> Result<()> {
        let Some(path) = &self.0.path else {
            return Ok(());
        };
        let _writer = self.0.writer.lock().await;
        loop {
            // Read both fields after acquiring the writer. A delayed save must
            // never overwrite a newer move or resize with an older snapshot.
            let value = {
                let geometry = self.0.geometry.lock().unwrap();
                if geometry.normal == geometry.saved {
                    return Ok(());
                }
                geometry.normal
            };
            let path = path.clone();
            tokio::task::spawn_blocking(move || atomic_write(&path, &serde_json::to_vec(&value)?))
                .await
                .context("保存窗口状态任务失败")??;
            self.0.geometry.lock().unwrap().saved = value;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_size_respects_the_available_content_area_on_small_screens() {
        assert_eq!(
            fit_size(DEFAULT_WINDOW_SIZE, Size::new(1440.0, 670.0)),
            Size::new(950.0, 670.0)
        );
        assert_eq!(
            fit_size(DEFAULT_WINDOW_SIZE, Size::new(1440.0, 900.0)),
            DEFAULT_WINDOW_SIZE
        );
        assert_eq!(
            fit_size(DEFAULT_WINDOW_SIZE, Size::new(f32::NAN, 0.0)),
            DEFAULT_WINDOW_SIZE
        );
    }

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

    #[tokio::test]
    async fn ordinary_position_survives_relaunch_and_special_states_do_not_replace_it() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::at(directory.path().to_owned()).unwrap();
        let path = store.root.join("window-state.json");
        // Existing installations only stored dimensions.
        std::fs::write(&path, br#"{"width":900,"height":500}"#).unwrap();
        let state = State::load(&store);
        assert_eq!(state.size(), Size::new(900.0, 500.0));
        assert_eq!(state.position(), None);
        assert!(state.capture_position(Some((-1800, 120)), false, false, false));
        assert!(!state.capture_position(Some((0, 0)), true, false, false));
        assert!(!state.capture_position(Some((0, 0)), false, true, false));
        assert!(!state.capture_position(Some((0, 0)), false, false, true));
        assert!(!state.capture_position(None, false, false, false));
        assert!(!state.capture_position(Some((i32::MIN, 0)), false, false, false));
        state.save().await.unwrap();
        let restored = State::load(&store);
        assert_eq!(restored.position(), Some((-1800, 120)));
        assert_eq!(restored.size(), Size::new(900.0, 500.0));
        assert!(!restored.dirty());
        state.capture_position(Some((220, 180)), false, false, false);
        let delayed = state.clone();
        let writer = tokio::spawn(async move { delayed.save().await });
        state.capture(Size::new(1100.0, 700.0), false, None);
        state.capture_position(Some((340, 200)), false, false, false);
        state.save().await.unwrap();
        writer.await.unwrap().unwrap();
        let latest = State::load(&store);
        assert_eq!(latest.position(), Some((340, 200)));
        assert_eq!(latest.size(), Size::new(1100.0, 700.0));
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
