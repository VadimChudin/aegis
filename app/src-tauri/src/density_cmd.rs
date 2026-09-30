use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use aegis_core::{
    density::{Density, Observation, Thresholds, Tracker},
    settings::DensitySettings,
    BrokerId,
};
use serde::Serialize;
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, State, WebviewUrl, WebviewWindowBuilder, WindowEvent,
};
use tokio::sync::{Mutex, Notify};

use crate::AppState;

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[derive(Clone, Serialize)]
pub struct Snapshot {
    broker: Option<BrokerId>,
    symbol: Option<String>,
    status: &'static str,
    error: Option<String>,
    recording: bool,
    recording_error: Option<String>,
    updated_at: Option<u64>,
    mid_price: Option<f64>,
    rows: Vec<Density>,
    revision: u64,
    docked: bool,
    thresholds: Thresholds,
}

impl Default for Snapshot {
    fn default() -> Self {
        Self {
            broker: None,
            symbol: None,
            status: "waiting",
            error: None,
            recording: false,
            recording_error: None,
            updated_at: None,
            mid_price: None,
            rows: Vec::new(),
            revision: 0,
            docked: true,
            thresholds: Thresholds::default(),
        }
    }
}

#[derive(Default)]
struct Monitor {
    preferred: Option<BrokerId>,
    snapshot: Snapshot,
    tracker: Tracker,
    settings_revision: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct RequestRevision {
    source: u64,
    settings: u64,
}

impl Monitor {
    fn revision(&self) -> RequestRevision {
        RequestRevision {
            source: self.snapshot.revision,
            settings: self.settings_revision,
        }
    }

    fn reset_source(&mut self, broker: Option<BrokerId>, symbol: Option<String>) {
        self.tracker.reset();
        self.snapshot = Snapshot {
            broker,
            symbol,
            revision: self.snapshot.revision + 1,
            docked: self.snapshot.docked,
            ..Default::default()
        };
    }
}

fn choose_broker(preferred: Option<BrokerId>, saved: Option<BrokerId>, connected: &[BrokerId]) -> Option<BrokerId> {
    [preferred, saved]
        .into_iter()
        .flatten()
        .find(|id| connected.contains(id))
        .or_else(|| connected.iter().copied().min())
}

pub struct DensityState {
    monitor: Mutex<Monitor>,
    pub docked: AtomicBool,
    quitting: AtomicBool,
    recorder: Arc<std::sync::Mutex<Recorder>>,
    changed: Notify,
}

impl DensityState {
    pub fn new(directory: PathBuf) -> Self {
        Self {
            monitor: Mutex::new(Monitor::default()),
            docked: AtomicBool::new(true),
            quitting: AtomicBool::new(false),
            recorder: Arc::new(std::sync::Mutex::new(Recorder::new(directory))),
            changed: Notify::new(),
        }
    }

    pub async fn invalidate(&self, broker: BrokerId) {
        let mut monitor = self.monitor.lock().await;
        if monitor.snapshot.broker == Some(broker) {
            let symbol = monitor.snapshot.symbol.clone();
            monitor.reset_source(Some(broker), symbol);
        }
        self.changed.notify_one();
    }
}

struct Recorder {
    directory: PathBuf,
    start: u64,
    part: u32,
    bytes: u64,
    file: Option<File>,
    last_cleanup: u64,
}

impl Recorder {
    fn new(directory: PathBuf) -> Self {
        Self {
            directory,
            start: now(),
            part: 0,
            bytes: 0,
            file: None,
            last_cleanup: 0,
        }
    }

    fn append(&mut self, line: &[u8]) -> io::Result<()> {
        if self.bytes + line.len() as u64 > 16 * 1024 * 1024 {
            self.file = None;
            self.part += 1;
            self.bytes = 0;
        }
        if self.file.is_none() {
            fs::create_dir_all(&self.directory)?;
            let path = self
                .directory
                .join(format!("densities-{}-{:04}.jsonl", self.start, self.part));
            self.file = Some(OpenOptions::new().create(true).append(true).open(path)?);
        }
        let result = self.file.as_mut().unwrap().write_all(line);
        if result.is_err() {
            self.file = None;
        } else {
            self.bytes += line.len() as u64;
        }
        result
    }

    fn cleanup(&mut self, settings: &DensitySettings, time: u64) -> io::Result<()> {
        if settings.retention_days == 0 && settings.max_history_mb == 0 {
            self.last_cleanup = time;
            return Ok(());
        }
        let entries = match fs::read_dir(&self.directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error),
        };
        let active = format!("densities-{}-{:04}.jsonl", self.start, self.part);
        let mut files = Vec::new();
        let mut total = 0_u64;
        for entry in entries {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(stem) = name.strip_prefix("densities-").and_then(|v| v.strip_suffix(".jsonl")) else {
                continue;
            };
            let Some((start, part)) = stem.split_once('-') else {
                continue;
            };
            let (Ok(start), Ok(part)) = (start.parse::<u64>(), part.parse::<u32>()) else {
                continue;
            };
            let metadata = fs::symlink_metadata(entry.path())?;
            if !metadata.file_type().is_file() {
                continue;
            }
            let size = metadata.len();
            let modified = metadata
                .modified()?
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            let expired = settings.retention_days > 0
                && time.saturating_sub(modified) > settings.retention_days as u64 * 86_400_000;
            if name != active && expired {
                fs::remove_file(entry.path())?;
                continue;
            }
            total = total.saturating_add(size);
            if name != active {
                files.push((start, part, entry.path(), size));
            }
        }
        files.sort_by_key(|file| (file.0, file.1));
        if settings.max_history_mb > 0 {
            let limit = settings.max_history_mb as u64 * 1024 * 1024;
            for (_, _, path, size) in files {
                if total <= limit {
                    break;
                }
                fs::remove_file(path)?;
                total = total.saturating_sub(size);
            }
        }
        self.last_cleanup = time;
        Ok(())
    }
}

#[derive(Serialize)]
struct Record<'a> {
    schema: u8,
    observed_at: u64,
    book_timestamp: Option<u64>,
    snapshot: &'a Snapshot,
    events: &'a [Observation],
}

async fn record(
    state: &DensityState,
    snapshot: &mut Snapshot,
    timestamp: Option<u64>,
    events: &[Observation],
    settings: &DensitySettings,
) {
    snapshot.recording = true;
    snapshot.recording_error = None;
    let result = serde_json::to_vec(&Record {
        schema: 1,
        observed_at: now(),
        book_timestamp: timestamp,
        snapshot,
        events,
    });
    let result = match result {
        Ok(mut line) => {
            line.push(b'\n');
            let recorder = state.recorder.clone();
            let settings = settings.clone();
            tokio::task::spawn_blocking(move || {
                let mut recorder = recorder.lock().unwrap_or_else(|e| e.into_inner());
                let previous_part = recorder.part;
                recorder.append(&line)?;
                let time = now();
                if time.saturating_sub(recorder.last_cleanup) >= 60_000 || previous_part != recorder.part {
                    recorder.cleanup(&settings, time)?;
                }
                Ok::<_, io::Error>(())
            })
            .await
            .map_err(|e| e.to_string())
            .and_then(|r| r.map_err(|e| e.to_string()))
        }
        Err(e) => Err(e.to_string()),
    };
    if let Err(error) = result {
        snapshot.recording = false;
        snapshot.recording_error = Some(error);
    }
}

#[tauri::command]
pub async fn density_settings_get(state: State<'_, AppState>) -> Result<DensitySettings, String> {
    Ok(state.settings.lock().await.public().density)
}

#[tauri::command]
pub async fn density_settings_save(
    app: AppHandle,
    state: State<'_, AppState>,
    settings: DensitySettings,
) -> Result<DensitySettings, String> {
    save_density_settings(&app, &state, |value| *value = settings).await
}

async fn save_density_settings(
    app: &AppHandle,
    state: &AppState,
    update: impl FnOnce(&mut DensitySettings),
) -> Result<DensitySettings, String> {
    let mut monitor = state.density.monitor.lock().await;
    let settings = {
        let mut store = state.settings.lock().await;
        let previous = store.public().density;
        let mut next = previous.clone();
        update(&mut next);
        store.set_density(next);
        if let Err(error) = store.save() {
            store.set_density(previous);
            return Err(error.to_string());
        }
        store.public().density
    };
    monitor.settings_revision += 1;
    state.density.docked.store(settings.docked, Ordering::SeqCst);
    state
        .density
        .recorder
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .last_cleanup = 0;
    state.density.changed.notify_one();
    let _ = app.emit("density_settings", &settings);
    drop(monitor);
    dock(app);
    Ok(settings)
}

#[tauri::command]
pub async fn density_snapshot(state: State<'_, AppState>) -> Result<Snapshot, String> {
    let mut snapshot = state.density.monitor.lock().await.snapshot.clone();
    snapshot.docked = state.density.docked.load(Ordering::SeqCst);
    Ok(snapshot)
}

#[tauri::command]
pub async fn density_select(
    app: AppHandle,
    state: State<'_, AppState>,
    broker: Option<BrokerId>,
) -> Result<(), String> {
    if let Some(id) = broker {
        if !state.sessions.lock().await.contains_key(&id) {
            return Err(format!("{} is not connected", id.name()));
        }
    }
    let mut monitor = state.density.monitor.lock().await;
    monitor.preferred = broker;
    if monitor.snapshot.broker != broker {
        monitor.reset_source(broker, None);
        let _ = app.emit("density_update", &monitor.snapshot);
    }
    state.density.changed.notify_one();
    Ok(())
}

pub async fn run(app: AppHandle) {
    let mut previous_status = String::new();
    loop {
        let state = app.state::<AppState>();
        let (preferred, initial_revision) = {
            let monitor = state.density.monitor.lock().await;
            (monitor.preferred, monitor.revision())
        };
        let public = state.settings.lock().await.public();
        let saved = public.chart_broker;
        let settings = public.density;
        let selected = {
            let sessions = state.sessions.lock().await;
            let connected: Vec<_> = sessions.keys().copied().collect();
            choose_broker(preferred, saved, &connected)
                .and_then(|id| sessions.get(&id).map(|s| (id, s.connector.clone())))
        };
        let revision = {
            let mut monitor = state.density.monitor.lock().await;
            if monitor.revision() != initial_revision {
                continue;
            }
            let id = selected.as_ref().map(|(id, _)| *id);
            if monitor.snapshot.broker != id {
                monitor.reset_source(id, selected.as_ref().map(|(_, c)| c.symbol().to_string()));
            }
            monitor.revision()
        };
        let result = if let Some((_, connector)) = &selected {
            Some(connector.order_book().await)
        } else {
            None
        };
        // A reconnect can replace a connector while its previous request is still in flight.
        if let Some((id, connector)) = &selected {
            if !state
                .sessions
                .lock()
                .await
                .get(id)
                .is_some_and(|s| Arc::ptr_eq(&s.connector, connector))
            {
                continue;
            }
        }
        let mut monitor = state.density.monitor.lock().await;
        if monitor.revision() != revision {
            continue;
        }
        let mut events = Vec::new();
        let mut book_timestamp = None;
        monitor.snapshot.docked = state.density.docked.load(Ordering::SeqCst);
        match result {
            Some(Ok(book)) if now().abs_diff(book.timestamp) <= 5_000 => {
                let bids: Vec<_> = book.bids.iter().map(|v| (v.price, v.quantity)).collect();
                let asks: Vec<_> = book.asks.iter().map(|v| (v.price, v.quantity)).collect();
                let time = now();
                let (rows, observations) = monitor.tracker.sample_with_settings(time, &bids, &asks, &settings);
                monitor.snapshot.thresholds = monitor.tracker.thresholds;
                monitor.snapshot.rows = rows;
                monitor.snapshot.mid_price = Some((bids[0].0 + asks[0].0) / 2.0);
                monitor.snapshot.symbol = Some(book.symbol);
                monitor.snapshot.status = "live";
                monitor.snapshot.error = None;
                monitor.snapshot.updated_at = Some(time);
                book_timestamp = Some(book.timestamp);
                events = observations;
            }
            other => {
                monitor.tracker.reset();
                monitor.snapshot.thresholds = Thresholds::default();
                monitor.snapshot.rows.clear();
                monitor.snapshot.mid_price = None;
                monitor.snapshot.updated_at = Some(now());
                monitor.snapshot.status = if other.is_none() { "waiting" } else { "unavailable" };
                monitor.snapshot.error = match other {
                    Some(Err(error)) => Some(error.to_string()),
                    Some(Ok(_)) => Some("Order book timestamp is stale or the local clock is out of sync".into()),
                    None => None,
                };
            }
        }
        let status = format!(
            "{:?}:{}:{:?}",
            monitor.snapshot.broker, monitor.snapshot.status, monitor.snapshot.error
        );
        if !settings.recording {
            monitor.snapshot.recording = false;
            monitor.snapshot.recording_error = None;
        } else if monitor.snapshot.status == "live" || status != previous_status || !monitor.snapshot.recording {
            record(
                &state.density,
                &mut monitor.snapshot,
                book_timestamp,
                &events,
                &settings,
            )
            .await;
            previous_status = status;
        }
        let _ = app.emit("density_update", &monitor.snapshot);
        drop(monitor);
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs_f64(settings.poll_seconds)) => {},
            _ = state.density.changed.notified() => {},
        }
    }
}

fn adjacent(app: &AppHandle) -> Option<(PhysicalPosition<i32>, PhysicalSize<u32>)> {
    let main = app.get_webview_window("main")?;
    let popup = app.get_webview_window("densities")?;
    let origin = main.outer_position().ok()?;
    let main_size = main.outer_size().ok()?;
    let monitor = main.current_monitor().ok()??;
    let scale = monitor.scale_factor();
    let popup_inner = popup.inner_size().ok()?;
    let width = popup.outer_size().ok()?.width;
    let width = if width == 0 { (520.0 * scale) as u32 } else { width };
    let screen = monitor.position();
    let screen_size = monitor.size();
    let gap = (8.0 * monitor.scale_factor()) as i32;
    let right = origin.x + main_size.width as i32 + gap;
    let left = origin.x - width as i32 - gap;
    let x = if right + width as i32 <= screen.x + screen_size.width as i32 {
        right
    } else if left >= screen.x {
        left
    } else {
        (screen.x + screen_size.width as i32 - width as i32).max(screen.x)
    };
    let height = main
        .inner_size()
        .ok()?
        .height
        .max((480.0 * scale) as u32)
        .min(screen_size.height);
    let y = origin
        .y
        .clamp(screen.y, screen.y + screen_size.height.saturating_sub(height) as i32);
    Some((
        PhysicalPosition::new(x, y),
        PhysicalSize::new(
            if popup_inner.width == 0 {
                (520.0 * scale) as u32
            } else {
                popup_inner.width
            },
            height,
        ),
    ))
}

pub fn dock(app: &AppHandle) {
    if !app.state::<AppState>().density.docked.load(Ordering::SeqCst) {
        return;
    }
    if let (Some(main), Some(popup)) = (app.get_webview_window("main"), app.get_webview_window("densities")) {
        if let (Ok(Some(screen)), Ok(size), Ok(position), Ok(popup_size)) = (
            main.current_monitor(),
            main.outer_size(),
            main.outer_position(),
            popup.outer_size(),
        ) {
            let gap = (8.0 * screen.scale_factor()) as u32;
            let combined = size.width + popup_size.width + gap;
            let right_fits = position.x + combined as i32 <= screen.position().x + screen.size().width as i32;
            let left_fits = position.x - popup_size.width as i32 - gap as i32 >= screen.position().x;
            if combined <= screen.size().width && !right_fits && !left_fits {
                let x = screen.position().x + (screen.size().width - combined) as i32;
                let _ = main.set_position(PhysicalPosition::new(x, position.y));
            }
        }
    }
    if let (Some(popup), Some((position, size))) = (app.get_webview_window("densities"), adjacent(app)) {
        if popup.outer_position().ok() != Some(position) {
            let _ = popup.set_position(position);
        }
        if popup.inner_size().ok() != Some(size) {
            let _ = popup.set_size(size);
        }
    }
}

pub fn setup_window(app: &AppHandle) -> tauri::Result<()> {
    let preferences = app.state::<AppState>().settings.blocking_lock().public().density;
    app.state::<AppState>()
        .density
        .docked
        .store(preferences.docked, Ordering::SeqCst);
    let popup = WebviewWindowBuilder::new(app, "densities", WebviewUrl::App("densities.html".into()))
        .title("AEGIS · Gold densities")
        .inner_size(520.0, 840.0)
        .min_inner_size(440.0, 480.0)
        .visible(preferences.auto_open)
        .build()?;
    if let Some(main) = app.get_webview_window("main") {
        if let (Ok(Some(screen)), Ok(size), Ok(position), Ok(popup_size)) = (
            main.current_monitor(),
            main.outer_size(),
            main.outer_position(),
            popup.outer_size(),
        ) {
            let combined = size.width + popup_size.width + (8.0 * screen.scale_factor()) as u32;
            if combined <= screen.size().width {
                let max_x = screen.position().x + (screen.size().width - combined) as i32;
                let _ = main.set_position(PhysicalPosition::new(
                    position.x.clamp(screen.position().x, max_x),
                    position.y,
                ));
            }
        }
        let handle = app.clone();
        main.on_window_event(move |event| match event {
            WindowEvent::Moved(_) | WindowEvent::Resized(_) => dock(&handle),
            WindowEvent::CloseRequested { .. } => {
                handle
                    .state::<AppState>()
                    .density
                    .quitting
                    .store(true, Ordering::SeqCst);
                if let Some(popup) = handle.get_webview_window("densities") {
                    let _ = popup.close();
                }
            }
            _ => {}
        });
    }
    let handle = app.clone();
    popup.on_window_event(move |event| match event {
        WindowEvent::Resized(_) => dock(&handle),
        WindowEvent::CloseRequested { api, .. } => {
            if !handle.state::<AppState>().density.quitting.load(Ordering::SeqCst) {
                api.prevent_close();
                if let Some(window) = handle.get_webview_window("densities") {
                    let _ = window.hide();
                }
            }
        }
        WindowEvent::Moved(position) => {
            if let Some((target, _)) = adjacent(&handle) {
                let state = handle.state::<AppState>();
                let near = position.x.abs_diff(target.x) < 24 && position.y.abs_diff(target.y) < 24;
                if !state.density.docked.load(Ordering::SeqCst) && near {
                    state.density.docked.store(true, Ordering::SeqCst);
                    dock(&handle);
                } else if state.density.docked.load(Ordering::SeqCst) && !near {
                    state.density.docked.store(false, Ordering::SeqCst);
                }
            }
        }
        _ => {}
    });
    dock(app);
    if preferences.auto_open {
        popup.show()?;
    }
    Ok(())
}

#[tauri::command]
pub fn density_open(app: AppHandle) -> Result<(), String> {
    let popup = app
        .get_webview_window("densities")
        .ok_or("Density window is unavailable")?;
    dock(&app);
    popup.show().map_err(|e| e.to_string())?;
    popup.set_focus().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn density_hide(app: AppHandle) -> Result<(), String> {
    app.get_webview_window("densities")
        .ok_or("Density window is unavailable")?
        .hide()
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn density_set_docked(app: AppHandle, state: State<'_, AppState>, docked: bool) -> Result<(), String> {
    save_density_settings(&app, &state, |value| value.docked = docked).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_flight_requests_are_invalidated_by_source_or_settings_changes() {
        let mut monitor = Monitor::default();
        let before_source = monitor.revision();
        monitor.reset_source(Some(BrokerId::Binance), Some("XAUUSDT".into()));
        assert!(monitor.revision() != before_source);
        let before_settings = monitor.revision();
        monitor.settings_revision += 1;
        assert!(monitor.revision() != before_settings);
    }

    #[test]
    fn resetting_source_clears_errors_and_thresholds() {
        let mut monitor = Monitor::default();
        monitor.snapshot.error = Some("old source failure".into());
        monitor.snapshot.recording_error = Some("old recording failure".into());
        monitor.snapshot.thresholds.bid = 10.0;
        monitor.snapshot.docked = false;
        monitor.reset_source(Some(BrokerId::Bybit), None);
        assert!(monitor.snapshot.error.is_none());
        assert!(monitor.snapshot.recording_error.is_none());
        assert_eq!(monitor.snapshot.thresholds.bid, 0.0);
        assert!(!monitor.snapshot.docked);
    }

    #[test]
    fn disconnected_preference_falls_back_to_saved_connected_source() {
        assert_eq!(
            choose_broker(
                Some(BrokerId::Roboforex),
                Some(BrokerId::Bybit),
                &[BrokerId::Binance, BrokerId::Bybit]
            ),
            Some(BrokerId::Bybit)
        );
        assert_eq!(
            choose_broker(
                Some(BrokerId::Bybit),
                Some(BrokerId::Binance),
                &[BrokerId::Binance, BrokerId::Bybit]
            ),
            Some(BrokerId::Bybit)
        );
        assert_eq!(choose_broker(None, None, &[]), None);
    }

    #[test]
    fn recording_appends_and_rotates_without_overwriting() {
        let directory = std::env::temp_dir().join(format!("aegis-density-record-{}-{}", std::process::id(), now()));
        let mut recorder = Recorder::new(directory.clone());
        recorder.append(b"{\"schema\":1}\n").unwrap();
        recorder.append(b"{\"schema\":2}\n").unwrap();
        let first = directory.join(format!("densities-{}-0000.jsonl", recorder.start));
        assert_eq!(fs::read_to_string(first).unwrap().lines().count(), 2);
        recorder.bytes = 16 * 1024 * 1024;
        recorder.append(b"{\"schema\":3}\n").unwrap();
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 2);
        drop(recorder);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn cleanup_deletes_only_inactive_owned_logs_and_preserves_active_file() {
        let directory = std::env::temp_dir().join(format!("aegis-density-cleanup-{}-{}", std::process::id(), now()));
        let mut recorder = Recorder::new(directory.clone());
        recorder.append(b"active\n").unwrap();
        let active = directory.join(format!("densities-{}-0000.jsonl", recorder.start));
        let owned = directory.join("densities-1-0000.jsonl");
        let unrelated = directory.join("densities-not-owned-0000.jsonl");
        let malformed = directory.join("densities-2-not-a-part.jsonl");
        fs::write(&owned, vec![0_u8; 17 * 1024 * 1024]).unwrap();
        fs::write(&unrelated, b"keep").unwrap();
        fs::write(&malformed, b"keep").unwrap();

        recorder.cleanup(&DensitySettings::default(), now()).unwrap();
        assert!(owned.exists());

        let settings = DensitySettings {
            max_history_mb: 16,
            ..DensitySettings::default()
        };
        recorder.cleanup(&settings, now()).unwrap();

        assert!(!owned.exists());
        assert!(active.exists());
        assert!(unrelated.exists());
        assert!(malformed.exists());

        let expired = directory.join("densities-3-0000.jsonl");
        fs::write(&expired, b"expired").unwrap();
        let retention = DensitySettings {
            retention_days: 1,
            ..DensitySettings::default()
        };
        recorder.cleanup(&retention, now() + 2 * 86_400_000).unwrap();
        assert!(!expired.exists());
        assert!(active.exists());
        assert!(unrelated.exists());
        drop(recorder);
        fs::remove_dir_all(directory).unwrap();
    }

    #[tokio::test]
    async fn recording_failure_is_visible() {
        let path = std::env::temp_dir().join(format!("aegis-density-blocked-{}-{}", std::process::id(), now()));
        fs::write(&path, b"not a directory").unwrap();
        let state = DensityState::new(path.clone());
        let mut snapshot = Snapshot::default();
        record(&state, &mut snapshot, None, &[], &DensitySettings::default()).await;
        assert!(!snapshot.recording);
        assert!(snapshot.recording_error.is_some());
        fs::remove_file(path).unwrap();
    }
}
