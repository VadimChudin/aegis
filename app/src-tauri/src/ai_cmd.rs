use std::{
    path::PathBuf,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use aegis_core::{
    ai::{self, AiSettings, Decision, PaperEngine, PaperState, Snapshot},
    BrokerId, Timeframe,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Manager, State};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::{Child, Command},
    sync::Mutex,
};

use crate::AppState;

const SETTINGS: &str = "__ai";
const CANCELLED_BEFORE_SEND: &str = "Запрос отменён до отправки модели";
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[derive(Clone, Default, Serialize, Deserialize)]
struct Costs {
    day: u64,
    month: u64,
    day_spent: f64,
    month_spent: f64,
    requests_today: u32,
}

fn month_id(seconds: u64) -> u64 {
    let z = (seconds / 86400) as i64 + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let m = mp + if mp < 10 { 3 } else { -9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y * 12 + m) as u64
}

impl Costs {
    fn refund_unsent(&mut self, day: u64, month: u64, amount: f64) {
        if self.day == day {
            self.day_spent = (self.day_spent - amount).max(0.0);
            self.requests_today = self.requests_today.saturating_sub(1);
        }
        if self.month == month {
            self.month_spent = (self.month_spent - amount).max(0.0);
        }
    }
    fn reset_periods(&mut self) {
        let day = now() / 86400;
        let month = month_id(now());
        if day != self.day {
            self.day = day;
            self.day_spent = 0.0;
            self.requests_today = 0;
        }
        if month != self.month {
            self.month = month;
            self.month_spent = 0.0;
        }
    }
    fn reserve(&mut self, amount: f64, settings: &AiSettings) -> Result<(), String> {
        self.reset_periods();
        if !amount.is_finite()
            || amount < 0.0
            || self.day_spent + amount > settings.api_daily_budget_usd
            || self.month_spent + amount > settings.api_monthly_budget_usd
            || self.requests_today >= settings.max_cloud_requests_per_day
        {
            return Err("Лимит расходов или числа запросов OpenRouter исчерпан".into());
        }
        self.day_spent += amount;
        self.month_spent += amount;
        self.requests_today += 1;
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
struct Saved {
    paper: PaperState,
    costs: Costs,
    broker: Option<BrokerId>,
}
struct Runtime {
    paper: PaperEngine,
    costs: Costs,
    running: bool,
    broker: Option<BrokerId>,
    message: String,
    last_request: u64,
    setup: Value,
}
pub struct AiState {
    // Serialize process transitions independently of paper/budget state.
    server: Mutex<Option<Child>>,
    inner: Mutex<Runtime>,
    busy: AtomicBool,
    setup_busy: AtomicBool,
    epoch: AtomicU64,
    sequence: AtomicU64,
    path: PathBuf,
    recovery_error: Option<String>,
}
impl AiState {
    pub fn new(path: PathBuf) -> Self {
        let model_cached = path.parent().is_some_and(|p| {
            p.join("ai-runtime/runtime/bin/ollama").exists()
                && p.join("ai-runtime/models/manifests/registry.ollama.ai/library/qwen3/8b")
                    .is_file()
        });
        let settings = path
            .parent()
            .and_then(|p| std::fs::read(p.join("settings.json")).ok())
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .and_then(|v| serde_json::from_value::<AiSettings>(v["strategies"][SETTINGS].clone()).ok())
            .filter(|s| s.validate().is_ok())
            .unwrap_or_default();
        let saved_result = std::fs::read(&path)
            .map_err(|e| e.to_string())
            .and_then(|b| serde_json::from_slice::<Saved>(&b).map_err(|e| e.to_string()));
        let mut recovery_error = if path.exists() && saved_result.is_err() {
            Some("Файл Paper повреждён. Auto заблокирован; исходный файл не перезаписан.".into())
        } else {
            None
        };
        let saved = saved_result.ok();
        let (paper, costs, broker) = match saved {
            Some(s) => match PaperEngine::from_state(s.paper, &settings) {
                Ok(p) => (p, s.costs, s.broker),
                Err(_) => {
                    recovery_error = Some("Некорректное сохранённое состояние Paper. Auto заблокирован.".into());
                    (PaperEngine::new(&settings), s.costs, s.broker)
                }
            },
            None => (PaperEngine::new(&settings), Costs::default(), None),
        };
        Self {
            server: Mutex::new(None),
            inner: Mutex::new(Runtime {
                paper,
                costs,
                running: false,
                broker,
                message: "Только Paper. Авто выключено после запуска.".into(),
                last_request: 0,
                setup: if model_cached {
                    json!({"status":"ready","progress":100})
                } else {
                    json!({"status":"Не настроено", "progress":0})
                },
            }),
            busy: AtomicBool::new(false),
            setup_busy: AtomicBool::new(false),
            epoch: AtomicU64::new(0),
            sequence: AtomicU64::new(now() * 1000),
            path,
            recovery_error,
        }
    }
    fn save(&self, runtime: &Runtime) -> Result<(), String> {
        if let Some(e) = &self.recovery_error {
            return Err(e.clone());
        }
        let parent = self.path.parent().ok_or("Нет каталога AI")?;
        std::fs::create_dir_all(parent).map_err(|_| "Не удалось создать каталог AI")?;
        let data = serde_json::to_vec(&Saved {
            paper: runtime.paper.state(),
            costs: runtime.costs.clone(),
            broker: runtime.broker,
        })
        .map_err(|_| "Не удалось сериализовать Paper")?;
        let temp = self.path.with_extension("tmp");
        use std::io::Write;
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temp).map_err(|_| "Не удалось сохранить Paper")?;
        file.write_all(&data)
            .and_then(|_| file.sync_all())
            .map_err(|_| "Не удалось сохранить Paper")?;
        std::fs::rename(temp, &self.path).map_err(|_| "Не удалось заменить файл Paper".into())
    }
}

async fn settings(state: &AppState) -> AiSettings {
    state
        .settings
        .lock()
        .await
        .strategy(SETTINGS)
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default()
}

#[tauri::command]
pub async fn ai_status(state: State<'_, AppState>) -> Result<Value, String> {
    let s = settings(&state).await;
    let key = state.settings.lock().await.ai_key().is_some();
    let mut r = state.ai.inner.lock().await;
    r.costs.reset_periods();
    Ok(
        json!({"settings":s,"key_stored":key,"running":r.running,"busy":state.ai.busy.load(Ordering::SeqCst),
        "message":state.ai.recovery_error.as_ref().unwrap_or(&r.message),"paper":r.paper.state(),"costs":r.costs,"setup":r.setup}),
    )
}

#[tauri::command]
pub async fn ai_save(state: State<'_, AppState>, settings: AiSettings, api_key: Option<String>) -> Result<(), String> {
    settings.validate()?;
    if state.ai.busy.load(Ordering::SeqCst) {
        return Err("Дождитесь окончания запроса".into());
    }
    let r = state.ai.inner.lock().await;
    if r.running || state.ai.busy.load(Ordering::SeqCst) {
        return Err("Сначала остановите Auto".into());
    }
    let current = crate::ai_cmd::settings(&state).await;
    if !r.paper.state().positions.is_empty()
        && (current.max_risk_pct != settings.max_risk_pct
            || current.max_positions != settings.max_positions
            || current.max_leverage != settings.max_leverage
            || current.daily_loss_limit != settings.daily_loss_limit)
    {
        return Err("Сначала закройте Paper-позиции, затем меняйте лимиты риска".into());
    }
    if current.initial_equity != settings.initial_equity {
        return Err("Начальный баланс фиксирован; изменение баланса не сбрасывает риск".into());
    }
    let mut store = state.settings.lock().await;
    if let Some(key) = api_key.filter(|k| !k.trim().is_empty()) {
        if key.len() > 4096 {
            return Err("Ключ слишком длинный".into());
        }
        store.set_ai_key(key.trim());
    }
    store.set_strategy(
        SETTINGS,
        serde_json::to_value(settings).map_err(|_| "Некорректные настройки")?,
    );
    state.ai.epoch.fetch_add(1, Ordering::SeqCst);
    store.save().map_err(|_| "Не удалось сохранить AI-настройки".into())
}

#[tauri::command]
pub async fn ai_forget_key(state: State<'_, AppState>) -> Result<(), String> {
    let mut store = state.settings.lock().await;
    store.forget_ai_key();
    store.save().map_err(|_| "Не удалось удалить ключ".into())
}

async fn snapshot(state: &AppState, broker: BrokerId, include_candles: bool) -> Result<Snapshot, String> {
    let connector = state
        .sessions
        .lock()
        .await
        .get(&broker)
        .map(|s| s.connector.clone())
        .ok_or("Подключите выбранного брокера")?;
    let book = connector
        .order_book()
        .await
        .map_err(|_| "Нет актуальной пары bid/ask у выбранного брокера; синтетическое исполнение запрещено")?;
    let time = book.timestamp / 1000;
    if now().abs_diff(time) > 30 {
        return Err("Котировка устарела".into());
    }
    let candles = if include_candles {
        connector
            .candles(Timeframe::M5, 60)
            .await
            .map_err(|_| "Не удалось получить свечи")?
    } else {
        Vec::new()
    };
    let bid = book.bids.first().ok_or("Нет bid")?.price;
    let ask = book.asks.first().ok_or("Нет ask")?.price;
    Ok(Snapshot {
        id: state.ai.sequence.fetch_add(1, Ordering::SeqCst),
        broker: format!("{broker:?}"),
        symbol: connector.symbol().into(),
        time,
        bid,
        ask,
        candles,
    })
}

async fn cloud_reserve(s: &AiSettings) -> Result<f64, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "HTTP client failed")?;
    let response = client
        .get("https://openrouter.ai/api/v1/models")
        .send()
        .await
        .map_err(|_| "Не удалось получить тарифы OpenRouter")?;
    if !response.status().is_success() {
        return Err("Тарифы OpenRouter недоступны".into());
    }
    let models: Value = response.json().await.map_err(|_| "Некорректные тарифы OpenRouter")?;
    let model = models["data"]
        .as_array()
        .and_then(|v| v.iter().find(|m| m["id"] == s.cloud_model))
        .ok_or("Модель отсутствует в каталоге OpenRouter")?;
    let price = |key: &str| -> Result<f64, String> {
        let v = model["pricing"][key]
            .as_str()
            .unwrap_or("0")
            .parse::<f64>()
            .map_err(|_| "Некорректная цена")?;
        if !v.is_finite() || v < 0.0 {
            return Err("Некорректная цена".into());
        }
        Ok(v)
    };
    if model["pricing"]["prompt"].as_str().is_none() || model["pricing"]["completion"].as_str().is_none() {
        return Err("Нет подтверждённых тарифов".into());
    }
    // Byte count bounds token count for this bounded text-only request, with a margin.
    Ok((65536.0 * price("prompt")? + 1024.0 * price("completion")? + price("request")?) * 1.2)
}

async fn cancelable_request(
    state: &AppState,
    epoch: u64,
    settings: &AiSettings,
    snapshot: &Snapshot,
    paper: &PaperState,
    key: Option<&str>,
    cloud: bool,
) -> Result<(Decision, Option<f64>), String> {
    if state.ai.epoch.load(Ordering::SeqCst) != epoch {
        return Err(CANCELLED_BEFORE_SEND.into());
    }
    tokio::select! {
        result=ai::request_decision(settings,snapshot,paper,key,cloud)=>result,
        _=async {loop {tokio::time::sleep(Duration::from_millis(100)).await;
            if state.ai.epoch.load(Ordering::SeqCst)!=epoch {break;}
        }}=>Err(if cloud {
            "Запрос отменён остановкой Auto; уже принятый провайдером запрос может быть оплачен"
        }else{"Локальный запрос отменён остановкой Auto; облачный API не использовался"}.into()),
    }
}

async fn do_step(state: &AppState, broker: BrokerId, cloud_only: bool, expected_epoch: u64) -> Result<(), String> {
    if state.ai.busy.swap(true, Ordering::SeqCst) {
        return Err("AI-запрос уже выполняется".into());
    }
    let outcome = step_inner(state, broker, cloud_only, expected_epoch).await;
    state.ai.busy.store(false, Ordering::SeqCst);
    if let Err(e) = &outcome {
        let mut r = state.ai.inner.lock().await;
        r.message = e.clone();
    }
    outcome
}
async fn step_inner(state: &AppState, broker: BrokerId, cloud_only: bool, epoch: u64) -> Result<(), String> {
    if let Some(e) = &state.ai.recovery_error {
        return Err(e.clone());
    }
    if state.ai.epoch.load(Ordering::SeqCst) != epoch {
        return Err("Запрос отменён".into());
    }
    let s = settings(state).await;
    s.validate()?;
    let snap = snapshot(state, broker, true).await?;
    let paper = {
        let mut r = state.ai.inner.lock().await;
        if r.broker.is_some_and(|b| b != broker) && !r.paper.state().positions.is_empty() {
            return Err("Сначала закройте Paper-позиции текущего брокера".into());
        }
        r.broker = Some(broker);
        r.paper.mark(&snap, &s)?;
        state.ai.save(&r)?;
        if now().saturating_sub(r.last_request) < s.interval_seconds {
            return Err("Минимальный интервал AI-запросов ещё не прошёл".into());
        }
        r.last_request = now();
        r.paper.state()
    };
    if !cloud_only {
        ensure_server(state).await?;
    }
    let mut answer = if cloud_only {
        Decision::Consult {
            snapshot_id: snap.id,
            reason: "Проверка облачного решения".into(),
        }
    } else {
        cancelable_request(state, epoch, &s, &snap, &paper, None, false)
            .await?
            .0
    };
    if matches!(answer, Decision::Consult { .. }) {
        let key = state.settings.lock().await.ai_key().ok_or("Введите ключ OpenRouter")?;
        let reserve = cloud_reserve(&s).await?;
        let (reserve_day, reserve_month) = {
            let mut r = state.ai.inner.lock().await;
            if state.ai.epoch.load(Ordering::SeqCst) != epoch {
                return Err("Cloud-запрос отменён до отправки; бюджет не списан".into());
            }
            r.costs.reserve(reserve, &s)?;
            state.ai.save(&r)?;
            (r.costs.day, r.costs.month)
        };
        // Keep the reservation charged on failures/missing usage: no unbounded retries.
        let result = cancelable_request(state, epoch, &s, &snap, &paper, Some(&key), true).await;
        if result.as_ref().is_err_and(|e| e == CANCELLED_BEFORE_SEND) {
            let mut r = state.ai.inner.lock().await;
            r.costs.refund_unsent(reserve_day, reserve_month, reserve);
            state.ai.save(&r)?;
        }
        let (decision, cost) = result?;
        if let Some(cost) = cost.filter(|c| c.is_finite() && *c >= 0.0) {
            let mut r = state.ai.inner.lock().await;
            if r.costs.day == reserve_day {
                r.costs.day_spent -= reserve - cost;
            }
            if r.costs.month == reserve_month {
                r.costs.month_spent -= reserve - cost;
            }
            state.ai.save(&r)?;
        }
        answer = decision;
    }
    let r = state.ai.inner.lock().await;
    if state.ai.epoch.load(Ordering::SeqCst) != epoch {
        return Err("Ответ отменён остановкой Auto".into());
    }
    drop(r);
    let mut execution = snapshot(state, broker, false).await?;
    if now().saturating_sub(snap.time) > 180
        || ((execution.bid - snap.bid) / snap.bid).abs() > 0.0005
        || ((execution.ask - snap.ask) / snap.ask).abs() > 0.0005
    {
        return Err("Цена изменилась или решение устарело; требуется новый анализ".into());
    }
    execution.id = snap.id;
    let mut r = state.ai.inner.lock().await;
    if state.ai.epoch.load(Ordering::SeqCst) != epoch {
        return Err("Ответ отменён остановкой Auto".into());
    }
    r.paper.mark(&execution, &s)?;
    if let Err(error) = r.paper.apply(answer, &execution, &s, now()) {
        let _ = r.paper.record_rejection(&execution, &error, now());
        state.ai.save(&r)?;
        return Err(error);
    }
    r.message = "Paper-решение обработано; реальные ордера не отправляются".into();
    r.setup = json!({"status":"ready","progress":100});
    state.ai.save(&r)
}

#[tauri::command]
pub async fn ai_step(state: State<'_, AppState>, broker: BrokerId) -> Result<(), String> {
    let epoch = state.ai.epoch.load(Ordering::SeqCst);
    do_step(&state, broker, false, epoch).await
}

#[tauri::command]
pub async fn ai_close(state: State<'_, AppState>, broker: BrokerId, position_id: u64) -> Result<(), String> {
    let settings = settings(&state).await;
    let snapshot = snapshot(&state, broker, false).await?;
    let mut r = state.ai.inner.lock().await;
    state.ai.epoch.fetch_add(1, Ordering::SeqCst);
    r.paper.mark(&snapshot, &settings)?;
    r.paper.apply(
        Decision::Close {
            snapshot_id: snapshot.id,
            position_id,
            reason: "Ручное закрытие Paper".into(),
        },
        &snapshot,
        &settings,
        now(),
    )?;
    state.ai.save(&r)
}
#[tauri::command]
pub async fn ai_start(app: AppHandle, state: State<'_, AppState>, broker: BrokerId) -> Result<(), String> {
    if app.state::<crate::observer_cmd::ObserverState>().running() {
        return Err("Stop the RoboForex observer before starting legacy AI Paper".into());
    }
    if let Some(e) = &state.ai.recovery_error {
        return Err(e.clone());
    }
    settings(&state).await.validate()?;
    let snap = snapshot(&state, broker, false).await?;
    let mut r = state.ai.inner.lock().await;
    if r.broker.is_some_and(|b| b != broker) && !r.paper.state().positions.is_empty() {
        return Err("Paper-позиции принадлежат другому брокеру".into());
    }
    r.paper.mark(&snap, &settings(&state).await)?;
    r.broker = Some(broker);
    r.running = true;
    r.message = "Auto Paper включён. Деньги недоступны.".into();
    Ok(())
}
#[tauri::command]
pub async fn ai_stop(state: State<'_, AppState>) -> Result<(), String> {
    let mut r = state.ai.inner.lock().await;
    state.ai.epoch.fetch_add(1, Ordering::SeqCst);
    r.running = false;
    r.message = "Auto остановлен; защитные Paper-стопы продолжают проверяться, пока приложение открыто".into();
    Ok(())
}

pub async fn stop_auto(state: &AppState) {
    state.ai.epoch.fetch_add(1, Ordering::SeqCst);
    let mut runtime = state.ai.inner.lock().await;
    runtime.running = false;
    runtime.message = "Legacy Auto disabled while RoboForex Observer is active".into();
}

#[tauri::command]
pub async fn ai_test(state: State<'_, AppState>, cloud: bool) -> Result<String, String> {
    if cloud {
        let key = state.settings.lock().await.ai_key().ok_or("Введите ключ OpenRouter")?;
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| "HTTP client failed")?;
        let response = client
            .get("https://openrouter.ai/api/v1/key")
            .bearer_auth(key)
            .send()
            .await
            .map_err(|_| "OpenRouter недоступен")?;
        if !response.status().is_success() {
            return Err(format!("OpenRouter: HTTP {}", response.status().as_u16()));
        }
        return Ok("OpenRouter: ключ принят. Платный запрос модели не выполнялся.".into());
    }
    ensure_server(&state).await?;
    let s = settings(&state).await;
    s.validate()?;
    let url = reqwest::Url::parse(&s.local_url).map_err(|_| "Некорректный адрес")?;
    let base = format!(
        "{}://{}:{}/api/tags",
        url.scheme(),
        url.host_str().unwrap_or("127.0.0.1"),
        url.port_or_known_default().unwrap_or(11434)
    );
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "HTTP client failed")?;
    let response = client
        .get(base)
        .send()
        .await
        .map_err(|_| "Ollama ещё не запущена. Нажмите Настроить модель")?;
    let tags: Value = response
        .json()
        .await
        .map_err(|_| "Ollama вернула некорректный список моделей")?;
    if !tags["models"]
        .as_array()
        .is_some_and(|a| a.iter().any(|m| m["name"] == s.local_model))
    {
        return Err("Модель ещё не скачана".into());
    }
    Ok("Локальная модель доступна".into())
}

fn setup_script(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .resolve("bridges/ai_setup.py", tauri::path::BaseDirectory::Resource)
        .ok()
        .filter(|p| p.exists())
        .or_else(|| {
            let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../python/aegis_lab/bridges/ai_setup.py");
            p.exists().then_some(p)
        })
        .ok_or("Нет ai_setup.py в установке".into())
}
async fn ensure_server(state: &AppState) -> Result<(), String> {
    const ENDPOINT: &str = "http://127.0.0.1:11434";
    let root = state.ai.path.parent().ok_or("Нет каталога AI")?.join("ai-runtime");
    let mut server = state.ai.server.lock().await;
    if let Some(child) = server.as_mut() {
        if child
            .try_wait()
            .map_err(|e| format!("Cannot inspect Ollama: {e}"))?
            .is_some()
        {
            *server = None;
        }
    }
    // Reuse an external healthy server without trying to bind its occupied port.
    if server.is_none() && aegis_core::local_ai::server_ready(ENDPOINT).await {
        return Ok(());
    }
    if server.is_none() {
        let binary = root.join("runtime/bin/ollama");
        if !binary.is_file() {
            // The configured OpenAI-compatible local endpoint need not be Ollama.
            return Ok(());
        }
        let mut command = Command::new(binary);
        command
            .arg("serve")
            .env("OLLAMA_HOST", "127.0.0.1:11434")
            .env("OLLAMA_MODELS", root.join("models"))
            .env("OLLAMA_NO_CLOUD", "1")
            .env("OLLAMA_CONTEXT_LENGTH", "8192")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        #[cfg(unix)]
        command.process_group(0);
        #[cfg(windows)]
        command.creation_flags(0x08000000);
        *server = Some(
            command
                .spawn()
                .map_err(|e| format!("Не удалось запустить Ollama: {e}"))?,
        );
    }
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let child = server.as_mut().ok_or("Managed Ollama process disappeared")?;
            if child.try_wait().map_err(|e| e.to_string())?.is_some() {
                return Err("Managed Ollama exited before becoming ready".to_string());
            }
            if aegis_core::local_ai::server_ready(ENDPOINT).await {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    })
    .await
    .unwrap_or_else(|_| Err("Локальный runtime ещё не готов; повторите проверку подключения".into()));
    if result.is_err() {
        if let Some(child) = server.take() {
            aegis_core::local_ai::stop_process(child).await?;
        }
    }
    result
}

#[tauri::command]
pub async fn ai_setup_model(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let root = state.ai.path.parent().ok_or("Нет каталога AI")?.join("ai-runtime");
    if state.ai.setup_busy.swap(true, Ordering::SeqCst) {
        return Err("Настройка уже выполняется".into());
    }
    let script = match setup_script(&app) {
        Ok(s) => s,
        Err(e) => {
            state.ai.setup_busy.store(false, Ordering::SeqCst);
            return Err(e);
        }
    };
    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        let result = async {
            let mut child = Command::new("python3")
                .arg(script)
                .arg("setup")
                .arg("--root")
                .arg(root)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .kill_on_drop(true)
                .spawn()
                .map_err(|_| "Для настройки требуется Python 3")?;
            let mut lines = BufReader::new(child.stdout.take().ok_or("Нет вывода установщика")?).lines();
            while let Some(line) = lines.next_line().await.map_err(|_| "Сбой чтения установщика")?
            {
                if let Ok(v) = serde_json::from_str::<Value>(&line) {
                    state.ai.inner.lock().await.setup = v;
                }
            }
            if !child.wait().await.map_err(|_| "Сбой установщика")?.success() {
                return Err("Установка модели не завершена; проверьте свободное место и соединение");
            }
            Ok::<(), &str>(())
        }
        .await;
        if let Err(e) = result {
            state.ai.inner.lock().await.setup = json!({"status":e,"progress":0});
        } else {
            if let Err(error) = ensure_server(&state).await {
                state.ai.inner.lock().await.setup = json!({"status":error,"progress":0});
            }
        }
        state.ai.setup_busy.store(false, Ordering::SeqCst);
    });
    Ok(())
}

pub async fn run(app: AppHandle) {
    let mut timer = tokio::time::interval(Duration::from_secs(2));
    loop {
        timer.tick().await;
        let state = app.state::<AppState>();
        let (broker, running, last) = {
            let r = state.ai.inner.lock().await;
            (r.broker, r.running, r.last_request)
        };
        let Some(broker) = broker else {
            continue;
        };
        let s = settings(&state).await;
        match snapshot(&state, broker, false).await {
            Ok(snap) => {
                let mut r = state.ai.inner.lock().await;
                if let Err(e) = r.paper.mark(&snap, &s) {
                    r.message = e;
                } else if r.message.starts_with("Защита Paper ждёт свежую котировку") {
                    r.message = "Котировки восстановлены; Paper-защита активна".into();
                }
                let _ = state.ai.save(&r);
            }
            Err(error) => {
                state.ai.inner.lock().await.message = format!("Защита Paper ждёт свежую котировку: {error}");
            }
        }
        if running && !state.ai.busy.load(Ordering::SeqCst) && now().saturating_sub(last) >= s.interval_seconds {
            let app = app.clone();
            let epoch = state.ai.epoch.load(Ordering::SeqCst);
            tauri::async_runtime::spawn(async move {
                let state = app.state::<AppState>();
                if !state.ai.inner.lock().await.running {
                    return;
                }
                let _ = do_step(&state, broker, false, epoch).await;
            });
        }
    }
}
pub async fn shutdown(state: &AppState) {
    state.ai.epoch.fetch_add(1, Ordering::SeqCst);
    {
        let mut r = state.ai.inner.lock().await;
        r.running = false;
        let _ = state.ai.save(&r);
    }
    let child = state.ai.server.lock().await.take();
    if let Some(child) = child {
        if let Err(error) = aegis_core::local_ai::stop_process(child).await {
            eprintln!("Managed AI runtime shutdown failed: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn budget_fails_closed_and_periods_use_calendar_month() {
        let mut c = Costs::default();
        let s = AiSettings::default();
        assert!(c.reserve(0.8, &s).is_ok());
        assert!(c.reserve(0.3, &s).is_err());
        assert!(c.reserve(f64::NAN, &s).is_err());
        assert_eq!(month_id(0), 1970 * 12 + 1);
        let (day, month) = (c.day, c.month);
        c.refund_unsent(day, month, 0.8);
        assert_eq!(c.day_spent, 0.0);
        assert_eq!(c.month_spent, 0.0);
        assert_eq!(c.requests_today, 0);
    }

    #[tokio::test]
    async fn corrupted_paper_blocks_overwrite_and_decisions() {
        let path = std::env::temp_dir().join(format!("aegis-corrupt-ai-{}.json", std::process::id()));
        std::fs::write(&path, b"not-json").unwrap();
        let state = AiState::new(path.clone());
        assert!(state.recovery_error.is_some());
        let runtime = state.inner.lock().await;
        assert!(state.save(&runtime).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"not-json");
        std::fs::remove_file(path).unwrap();
    }

    #[tokio::test]
    async fn paper_and_budget_survive_restart_but_auto_does_not() {
        let root = std::env::temp_dir().join(format!("aegis-ai-restore-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("ai-paper.json");
        let first = AiState::new(path.clone());
        {
            let mut r = first.inner.lock().await;
            r.costs.reserve(0.5, &AiSettings::default()).unwrap();
            r.running = true;
            r.broker = Some(BrokerId::Binance);
            first.save(&r).unwrap();
        }
        let second = AiState::new(path);
        let r = second.inner.lock().await;
        assert!(!r.running);
        assert_eq!(r.costs.day_spent, 0.5);
        assert_eq!(r.broker, Some(BrokerId::Binance));
        assert_eq!(r.paper.state().equity, 10_000.0);
        std::fs::remove_dir_all(root).unwrap();
    }
}
