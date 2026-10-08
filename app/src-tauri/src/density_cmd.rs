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
    density::{Density, Observation, Tracker},
    BrokerId,
};
use serde::Serialize;
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, State, WebviewUrl, WebviewWindowBuilder, WindowEvent,
};
use tokio::sync::Mutex;

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
        }
    }
}

#[derive(Default)]
struct Monitor {
    preferred: Option<BrokerId>,
    snapshot: Snapshot,
    tracker: Tracker,
}

pub struct DensityState {
    monitor: Mutex<Monitor>,
    pub docked: AtomicBool,
    quitting: AtomicBool,
    recorder: Arc<std::sync::Mutex<Recorder>>,
}

impl DensityState {
    pub fn new(directory: PathBuf) -> Self {
        Self {
            monitor: Mutex::new(Monitor::default()),
            docked: AtomicBool::new(true),
            quitting: AtomicBool::new(false),
            recorder: Arc::new(std::sync::Mutex::new(Recorder::new(directory))),
        }
    }

    pub async fn invalidate(&self, broker: BrokerId) {
        let mut monitor = self.monitor.lock().await;
        if monitor.snapshot.broker == Some(broker) {
            monitor.tracker.reset();
            monitor.snapshot.rows.clear();
            monitor.snapshot.mid_price = None;
            monitor.snapshot.status = "waiting";
            monitor.snapshot.updated_at = None;
            monitor.snapshot.revision += 1;
        }
    }
}

struct Recorder {
    directory: PathBuf,
    start: u64,
    part: u32,
    bytes: u64,
    file: Option<File>,
}

impl Recorder {
    fn new(directory: PathBuf) -> Self {
        Self {
            directory,
            start: now(),
            part: 0,
            bytes: 0,
            file: None,
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
}

#[derive(Serialize)]
struct Record<'a> {
    schema: u8,
    observed_at: u64,
    book_timestamp: Option<u64>,
    snapshot: &'a Snapshot,
    events: &'a [Observation],
}

async fn record(state: &DensityState, snapshot: &mut Snapshot, timestamp: Option<u64>, events: &[Observation]) {
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
            tokio::task::spawn_blocking(move || recorder.lock().unwrap_or_else(|e| e.into_inner()).append(&line))
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
        monitor.tracker.reset();
        let revision = monitor.snapshot.revision + 1;
        monitor.snapshot = Snapshot {
            broker,
            revision,
            ..Default::default()
        };
        let _ = app.emit("density_update", &monitor.snapshot);
    }
    Ok(())
}

pub async fn run(app: AppHandle) {
    let mut previous_status = String::new();
    loop {
        let state = app.state::<AppState>();
        let saved = state.settings.lock().await.public().chart_broker;
        let preferred = state.density.monitor.lock().await.preferred;
        let selected = {
            let sessions = state.sessions.lock().await;
            preferred
                .or(saved)
                .filter(|id| sessions.contains_key(id))
                .or_else(|| sessions.keys().copied().min())
                .and_then(|id| sessions.get(&id).map(|s| (id, s.connector.clone())))
        };
        let revision = {
            let mut monitor = state.density.monitor.lock().await;
            let id = selected.as_ref().map(|(id, _)| *id);
            if monitor.snapshot.broker != id {
                monitor.tracker.reset();
                let revision = monitor.snapshot.revision + 1;
                monitor.snapshot = Snapshot {
                    broker: id,
                    symbol: selected.as_ref().map(|(_, c)| c.symbol().to_string()),
                    revision,
                    ..Default::default()
                };
            }
            monitor.snapshot.revision
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
        if monitor.snapshot.revision != revision {
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
                let (rows, observations) = monitor.tracker.sample(time, &bids, &asks);
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
        if monitor.snapshot.status == "live" || status != previous_status || !monitor.snapshot.recording {
            record(&state.density, &mut monitor.snapshot, book_timestamp, &events).await;
            previous_status = status;
        }
        let _ = app.emit("density_update", &monitor.snapshot);
        drop(monitor);
        tokio::time::sleep(Duration::from_secs(1)).await;
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
    let popup = WebviewWindowBuilder::new(app, "densities", WebviewUrl::App("densities.html".into()))
        .title("AEGIS · Gold densities")
        .inner_size(520.0, 840.0)
        .min_inner_size(440.0, 480.0)
        .visible(true)
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
    popup.show()?;
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
pub fn density_set_docked(app: AppHandle, state: State<'_, AppState>, docked: bool) -> Result<(), String> {
    state.density.docked.store(docked, Ordering::SeqCst);
    dock(&app);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[tokio::test]
    async fn recording_failure_is_visible() {
        let path = std::env::temp_dir().join(format!("aegis-density-blocked-{}-{}", std::process::id(), now()));
        fs::write(&path, b"not a directory").unwrap();
        let state = DensityState::new(path.clone());
        let mut snapshot = Snapshot::default();
        record(&state, &mut snapshot, None, &[]).await;
        assert!(!snapshot.recording);
        assert!(snapshot.recording_error.is_some());
        fs::remove_file(path).unwrap();
    }
}
