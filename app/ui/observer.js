"use strict";
(() => {
  const STRATEGIES = [
    { id: "density_bounce", title: "Отскок от плотности", en: "Density bounce", kk: "Тығыздықтан серпілу", prompt: "Ищи отскок от подтверждённой плотности лимитных заявок. Не входи только из-за касания: оцени реакцию цены, спред и контекст." },
    { id: "structural", title: "SMC / структурный разворот", en: "SMC / structural reversal", kk: "SMC / құрылымдық бұрылыс", prompt: "Ищи смену структуры рынка (BOS/CHOCH) и подтверждённую реакцию от зоны интереса. Не считай одиночный прокол подтверждением разворота." },
    { id: "breakout", title: "Пробой", en: "Breakout", kk: "Бұзып өту", prompt: "Ищи подтверждённый пробой значимого уровня. Отличай закрепление от ложного пробоя, учитывай спред и доступный объём." },
    { id: "liquidity_sweep", title: "Снятие ликвидности", en: "Liquidity sweep", kk: "Өтімділікті алу", prompt: "Ищи снятие стопов за локальными максимумами/минимумами и возврат цены за уровень. Без возврата и подтверждения не открывай позицию." },
  ];
  const FRAMES = [["1m", "M1"], ["5m", "M5"], ["15m", "M15"], ["1h", "H1"], ["4h", "H4"], ["1d", "D1"]];
  const DEFAULTS = {
    strictness: 100,
    strategies: STRATEGIES.map((s) => ({ id: s.id, enabled: true, prompt: s.prompt, timeframes: FRAMES.map(([id]) => id) })),
    risk_pct: 0.5,
    initial_equity: 10000,
    max_spread: 1,
    commission_per_oz: 0,
    slippage: 0,
  };
  const LABELS = {
    ru: {
      intro: "Автономное наблюдение и только Paper-симуляция для RoboForex MT5 · XAUUSD.", no_orders: "Только симуляция: реальные ордера и сделки не отправляются.", stop_protect: "Остановка прекращает новые AI-анализы. Открытая Paper-позиция остаётся под защитой и отслеживается по котировкам до штатного выхода.",
      strategies: "Стратегии", enabled: "Включена", prompt: "Инструкция стратегии для ИИ", frames: "Таймфреймы для анализа",
      strict: "Следование стратегиям", strict_help: "100 — строго по инструкциям стратегий; 0 — больше свободы модели. На малых значениях ИИ может анализировать и дополнительные таймфреймы.",
      strict_strategy: "Строго по стратегиям", free: "Свободнее", position_lock: "Есть открытая Paper-позиция; дождитесь её закрытия перед изменением настроек.", equity_locked: "Начальный капитал нельзя изменить после первого решения.", risk: "Риск на Paper-сделку, %", equity: "Начальный Paper-капитал, USD", spread: "Максимальный спред, USD/oz", commission: "Комиссия за одну сторону, USD/oz (round-trip ×2)", slippage: "Проскальзывание, USD/oz", costs: "Риск и издержки симуляции",
      save: "Сохранить настройки", start: "Запустить наблюдение", stop: "Остановить", refresh: "Обновить", defaults: "Восстановить исходные настройки", journal: "Скачать журнал JSONL", saved: "Настройки сохранены.", dirty: "Есть несохранённые изменения. Запуск сначала сохранит их.", loading: "Загрузка конфигурации…", running: "Наблюдение активно", stopped: "Остановлено", busy: "Выполняется операция…", error: "Ошибка", phaseCollecting: "Сбор таймфреймов RoboForex", phaseWaiting: "Ожидание свежих котировок MT5; новые входы приостановлены", phaseInference: "Ошибка анализа; новые входы приостановлены", phaseRecorded: "Решение записано; наблюдение продолжается", no_conn: "Для запуска подключите RoboForex MT5 и инструмент XAUUSD. Проверьте готовность локальной модели Qwen в настройках ИИ.", stopped_edit: "Чтобы менять конфигурацию, сначала остановите наблюдение.", quote: "Котировка XAUUSD", bid: "Bid", ask: "Ask", spread_now: "Спред", age: "Возраст котировки", book: "Стакан", no_book: "Стакан недоступен — используем имеющиеся котировки и данные.", book_yes: "Стакан доступен", volume: "Объём", ticks: "Тиковый объём CFD, не лента биржевых сделок", frames_live: "Сводки таймфреймов", decision: "Последнее решение ИИ", position: "Paper-позиция", none: "Нет открытой Paper-позиции", equity_now: "Paper-капитал", stats: "Счётчики", decisions: "Решения", outcomes: "Исходы", phase: "Фаза", candidate: "Решение", accepted: "Принято", rejected: "Отклонено", reason: "Причина", elapsed: "Время", entry: "Вход", units: "Количество, oz", pnl: "Нереализованный P/L", no_sample: "Ожидание рыночных данных…", no_decision: "Решений пока нет.", journal_empty: "Журнал пока пуст.", download_error: "Не удалось скачать журнал", preflight: "Перед запуском проверьте: подключён RoboForex MT5 с XAUUSD; локальная модель Qwen установлена и запущена. Backend также проверит все ограничения безопасности.", run_saved: "Конфигурация сохранена; запускаю наблюдение…", defaults_done: "Показаны исходные настройки. Сохраните их, чтобы применить.", unknown: "—", need_strategy: "Включите хотя бы одну стратегию.", prompt_invalid: "Инструкция каждой стратегии должна содержать от 1 до 512 символов.", timeframe_invalid: "Для каждой стратегии выберите хотя бы один таймфрейм.", config_invalid: "Проверьте риск (0–1%), капитал, спред, комиссию и проскальзывание.", phaseNames: { idle: "Ожидание", starting: "Запуск", observing: "Наблюдение", analyzing: "Анализ", error: "Ошибка" },
    },
    en: {
      intro: "Autonomous observation and Paper simulation only for RoboForex MT5 · XAUUSD.", no_orders: "Simulation only: no real orders or trades are sent.", phaseCollecting: "Collecting RoboForex timeframes", phaseWaiting: "Waiting for fresh MT5 quotes; new entries paused", phaseInference: "Analysis failed; new entries paused", phaseRecorded: "Decision recorded; observation continues", stop_protect: "Stopping ends new AI analysis. Any open Paper position remains protected and is monitored by quotes until its normal exit.", position_lock: "An open Paper position remains; wait for it to close before editing settings.", equity_locked: "Initial equity cannot change after the first decision.", strategies: "Strategies", enabled: "Enabled", prompt: "Strategy instructions for AI", frames: "Analysis timeframes", strict: "Strategy adherence", strict_help: "100 means strictly follow strategy instructions; 0 gives the model more freedom. At low values AI may also analyze additional timeframes.", strict_strategy: "Strict strategies", free: "More freedom", risk: "Paper trade risk, %", equity: "Initial Paper equity, USD", spread: "Maximum spread, USD/oz", commission: "Commission per side, USD/oz (round trip ×2)", slippage: "Slippage, USD/oz", costs: "Simulation risk and costs", save: "Save settings", start: "Start observation", stop: "Stop", refresh: "Refresh", defaults: "Restore defaults", journal: "Download JSONL journal", saved: "Settings saved.", dirty: "Unsaved changes. Start will save them first.", loading: "Loading configuration…", running: "Observation active", stopped: "Stopped", busy: "Operation in progress…", error: "Error", no_conn: "Connect RoboForex MT5 with XAUUSD to start. Check that the local Qwen model is ready in AI settings.", stopped_edit: "Stop observation before editing configuration.", quote: "XAUUSD quote", bid: "Bid", ask: "Ask", spread_now: "Spread", age: "Quote age", book: "Order book", no_book: "Order book unavailable — using available quotes and data.", book_yes: "Order book available", volume: "Volume", ticks: "CFD tick volume, not exchange time-and-sales", frames_live: "Timeframe summaries", decision: "Latest AI decision", position: "Paper position", none: "No open Paper position", equity_now: "Paper equity", stats: "Counters", decisions: "Decisions", outcomes: "Outcomes", phase: "Phase", candidate: "Decision", accepted: "Accepted", rejected: "Rejected", reason: "Reason", elapsed: "Time", entry: "Entry", units: "Quantity, oz", pnl: "Unrealized P/L", no_sample: "Waiting for market data…", no_decision: "No decisions yet.", journal_empty: "Journal is empty.", download_error: "Could not download journal", preflight: "Before starting, check that RoboForex MT5 with XAUUSD is connected and local Qwen is installed and running. The backend also enforces all safety limits.", run_saved: "Configuration saved; starting observation…", defaults_done: "Defaults loaded. Save them to apply.", unknown: "—", need_strategy: "Enable at least one strategy.", prompt_invalid: "Each strategy prompt must contain 1 to 512 characters.", timeframe_invalid: "Select at least one timeframe for every strategy.", config_invalid: "Check risk (0–1%), equity, spread, commission, and slippage.", phaseNames: { idle: "Idle", starting: "Starting", observing: "Observing", analyzing: "Analyzing", error: "Error" },
    },
    kk: {
      intro: "RoboForex MT5 · XAUUSD үшін автономды бақылау және тек Paper-симуляция.", no_orders: "Тек симуляция: нақты ордерлер жіберілмейді.", stop_protect: "Тоқтату жаңа ЖИ талдауын тоқтатады. Ашық Paper позициясы қорғалып, қалыпты жабылғанша баға арқылы бақыланады.", position_lock: "Ашық Paper позициясы бар; баптауларды өзгерту үшін оның жабылуын күтіңіз.", equity_locked: "Бірінші шешімнен кейін бастапқы капиталды өзгертуге болмайды.", strategies: "Стратегиялар", enabled: "Қосулы", prompt: "ЖИ стратегиясының нұсқауы", frames: "Талдау таймфреймдері", strict: "Стратегияны сақтау", strict_help: "100 — стратегия нұсқауын қатаң орындау; 0 — модель еркіндігі жоғары. Төмен мәндерде ЖИ қосымша таймфреймдерді талдауы мүмкін.", strict_strategy: "Стратегияны қатаң сақтау", free: "Еркінірек", risk: "Paper мәміле тәуекелі, %", equity: "Бастапқы Paper капиталы, USD", spread: "Ең үлкен спред, USD/oz", commission: "Бір жаққа комиссия, USD/oz (round-trip ×2)", slippage: "Сырғу, USD/oz", costs: "Симуляция тәуекелі мен шығындары", save: "Баптауларды сақтау", start: "Бақылауды бастау", stop: "Тоқтату", refresh: "Жаңарту", defaults: "Әдепкі баптауларды қалпына келтіру", journal: "JSONL журналын жүктеу", saved: "Баптаулар сақталды.", dirty: "Сақталмаған өзгерістер бар. Бастау алдында сақталады.", loading: "Конфигурация жүктелуде…", running: "Бақылау белсенді", stopped: "Тоқтатылды", busy: "Операция орындалуда…", error: "Қате", no_conn: "Бастау үшін XAUUSD бар RoboForex MT5 қосыңыз. ЖИ баптауларында жергілікті Qwen дайын екенін тексеріңіз.", stopped_edit: "Конфигурацияны өзгерту үшін алдымен бақылауды тоқтатыңыз.", quote: "XAUUSD бағасы", bid: "Bid", ask: "Ask", spread_now: "Спред", age: "Баға жасы", book: "Өтімділік кітабы", no_book: "Кітап қолжетімсіз — бар баға мен деректі қолданамыз.", book_yes: "Кітап қолжетімді", volume: "Көлем", ticks: "CFD тик көлемі, биржалық мәмілелер таспасы емес", frames_live: "Таймфрейм қорытындылары", decision: "ЖИ соңғы шешімі", position: "Paper позициясы", none: "Ашық Paper позициясы жоқ", equity_now: "Paper капиталы", stats: "Санағыштар", decisions: "Шешімдер", outcomes: "Нәтижелер", phase: "Кезең", candidate: "Шешім", accepted: "Қабылданды", rejected: "Қабылданбады", reason: "Себеп", elapsed: "Уақыт", entry: "Кіру", units: "Саны, oz", pnl: "Іске асырылмаған P/L", no_sample: "Нарық дерегі күтілуде…", no_decision: "Шешім әзірге жоқ.", journal_empty: "Журнал бос.", download_error: "Журналды жүктеу мүмкін болмады", preflight: "Бастамас бұрын XAUUSD бар RoboForex MT5 қосылғанын және жергілікті Qwen орнатылып, іске қосылғанын тексеріңіз. Backend қауіпсіздік шектеулерін де тексереді.", run_saved: "Конфигурация сақталды; бақылау басталуда…", defaults_done: "Әдепкі баптаулар көрсетілді. Қолдану үшін сақтаңыз.", unknown: "—", need_strategy: "Кемінде бір стратегияны қосыңыз.", prompt_invalid: "Әр стратегия нұсқауы 1–512 таңбадан тұруы керек.", timeframe_invalid: "Әр стратегия үшін кемінде бір таймфрейм таңдаңыз.", config_invalid: "Тәуекелді (0–1%), капиталды, спредті, комиссияны және сырғуды тексеріңіз.", phaseNames: { idle: "Күту", starting: "Іске қосу", observing: "Бақылау", analyzing: "Талдау", error: "Қате" },
    },
  };
  const STATES = new WeakMap();

  function makeState() {
    return { config: null, saved: null, status: null, loaded: false, busy: false, notice: "", error: "", bound: false, timer: null, loadToken: 0 };
  }
  function lang() {
    const value = window.I18N?.lang || "ru";
    return LABELS[value] ? value : "ru";
  }
  function text(key) {
    return LABELS[lang()][key] || LABELS.ru[key] || key;
  }
  function esc(value) {
    return (window.AEGIS?.esc || ((s) => String(s ?? "").replace(/[&<>"']/g, c => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c])))(value);
  }
  function clone(value) {
    return JSON.parse(JSON.stringify(value));
  }
  function jsonEqual(a, b) {
    return JSON.stringify(a) === JSON.stringify(b);
  }
  function invoke(command, args) {
    if (!window.AEGIS?.invoke) return Promise.reject(new Error("Tauri is not available"));
    return window.AEGIS.invoke(command, args);
  }
  function cfgFrom(source) {
    const input = source && typeof source === "object" ? source : {};
    const strategies = STRATEGIES.map((def) => {
      const item = (input.strategies || []).find((s) => s.id === def.id) || {};
      return { id: def.id, enabled: item.enabled == null ? true : !!item.enabled, prompt: typeof item.prompt === "string" ? item.prompt : def.prompt, timeframes: FRAMES.map(([id]) => id).filter(id => (Array.isArray(item.timeframes) ? item.timeframes : DEFAULTS.strategies.find(s => s.id === def.id).timeframes).includes(id)) };
    });
    const number = (key) => Number.isFinite(Number(input[key])) ? Number(input[key]) : DEFAULTS[key];
    return { ...input, strictness: Math.min(100, Math.max(0, Math.round(number("strictness")))), strategies, risk_pct: number("risk_pct"), initial_equity: number("initial_equity"), max_spread: number("max_spread"), commission_per_oz: number("commission_per_oz"), slippage: number("slippage") };
  }
  function configMarkup(config) {
    const lock = STATES.get(document.querySelector(".obs-root"));
    const disabled = lock?.status?.running || lock?.busy ? "disabled" : "";
    return `<form class="obs-form" data-testid="observer-config">
      <section class="obs-card"><h3>${esc(text("strategies"))}</h3><div class="obs-strategy-list">${config.strategies.map((strategy) => {
        const def = STRATEGIES.find(s => s.id === strategy.id);
        return `<fieldset class="obs-strategy" data-strategy="${esc(strategy.id)}" ${disabled}><legend>${esc(lang() === "ru" ? def.title : def[lang()] || def.en)}</legend>
          <label class="obs-toggle"><input type="checkbox" data-config="enabled" ${strategy.enabled ? "checked" : ""} ${disabled}><span>${esc(text("enabled"))}</span></label>
          <label class="obs-label">${esc(text("prompt"))}<textarea data-config="prompt" rows="3" maxlength="512" ${disabled}>${esc(strategy.prompt)}</textarea></label>
          <span class="obs-subtitle">${esc(text("frames"))}</span><div class="obs-frames">${FRAMES.map(([id, title]) => `<label><input type="checkbox" data-frame="${id}" ${strategy.timeframes.includes(id) ? "checked" : ""} ${disabled}>${title}</label>`).join("")}</div>
        </fieldset>`;
      }).join("")}</div></section>
      <section class="obs-card"><div class="obs-section-head"><h3>${esc(text("strict"))}</h3><output class="obs-range-value" data-strict-value>${config.strictness}</output></div>
        <div class="obs-range-labels"><span>${esc(text("free"))}</span><span>${esc(text("strict_strategy"))}</span></div><input class="obs-range" type="range" min="0" max="100" step="1" value="${config.strictness}" data-config="strictness" aria-label="${esc(text("strict"))}" ${disabled}>
        <p class="obs-help">${esc(text("strict_help"))}</p>
      </section>
      <section class="obs-card"><h3>${esc(text("costs"))}</h3><div class="obs-numbers">${[["risk_pct", "risk", .01, 1, .01], ["initial_equity", "equity", .01, 100000000, "any"], ["max_spread", "spread", .01, 100, .01], ["commission_per_oz", "commission", 0, 100, .01], ["slippage", "slippage", 0, 100, .01]].map(([key, label, min, max, step]) => `<label class="obs-label">${esc(text(label))}${key === "initial_equity" ? `<small class="obs-help" data-equity-lock ${lock?.status?.decisions > 0 ? "" : "hidden"}>${esc(text("equity_locked"))}</small>` : ""}<input type="number" data-config="${key}" min="${min}" max="${max}" step="${step}" value="${esc(config[key])}" required ${disabled} ${key === "initial_equity" && lock?.status?.decisions > 0 ? "disabled" : ""}></label>`).join("")}</div></section>
      <p class="obs-edit-notice" data-edit-notice role="status"></p>
      <div class="obs-actions"><button class="cta" type="submit" data-action="save" ${disabled}>${esc(text("save"))}</button><button class="ghost" type="button" data-action="defaults" ${disabled}>${esc(text("defaults"))}</button><span class="obs-inline-error" data-form-error role="alert"></span></div>
    </form>`;
  }
  function field(label, value) {
    return `<div class="obs-metric"><span>${esc(label)}</span><strong>${esc(value)}</strong></div>`;
  }
  function number(value, digits = 2) {
    const n = Number(value);
    return Number.isFinite(n) ? n.toLocaleString(lang() === "en" ? "en-US" : lang() === "kk" ? "kk-KZ" : "ru-RU", { maximumFractionDigits: digits }) : text("unknown");
  }
  function asTime(timestamp) {
    if (timestamp == null) return null;
    const value = typeof timestamp === "number" ? new Date(timestamp < 1e12 ? timestamp * 1000 : timestamp) : new Date(timestamp);
    return Number.isNaN(value.getTime()) ? null : value;
  }
  function elapsed(timestamp) {
    const date = asTime(timestamp);
    if (!date) return text("unknown");
    const age = Math.max(0, Math.floor((Date.now() - date.getTime()) / 1000));
    return age < 60 ? `${age} s` : `${Math.floor(age / 60)} min`;
  }
  function sampleMarkup(sample) {
    if (!sample) return `<p class="obs-muted">${esc(text("no_sample"))}</p>`;
    const quote = sample.quote || sample;
    const bid = sample.bid ?? sample.bid_price ?? quote.bid;
    const ask = sample.ask ?? sample.ask_price ?? quote.ask;
    const spread = sample.spread ?? (Number.isFinite(Number(bid)) && Number.isFinite(Number(ask)) ? Number(ask) - Number(bid) : null);
    const stamp = sample.timestamp ?? sample.observed_at_ms ?? sample.time_ms ?? sample.time ?? sample.at ?? sample.ts ?? quote.time_ms;
    const book = sample.book ?? sample.order_book ?? sample.depth ?? sample.has_book;
    const hasBook = book === true || (book && typeof book === "object");
    const ticks = Array.isArray(sample.ticks) ? sample.ticks : [];
    const knownVolumes = ticks.map(tick => tick.volume).filter(value => value != null && Number.isFinite(Number(value)));
    const volume = sample.tick_volume ?? sample.volume ?? sample.volume_value ?? (knownVolumes.length ? knownVolumes.reduce((sum, value) => sum + Number(value), 0) : null);
    const volumeKind = sample.volume_kind === "mt5_ticks_not_exchange_tape" ? text("ticks") : sample.volume_kind;
    const bookNote = sample.book_note;
    const quoteAge = elapsed(sample.quote?.time_ms ?? stamp);
    return `<div class="obs-metrics">${field(text("bid"), number(bid, 3))}${field(text("ask"), number(ask, 3))}${field(text("spread_now"), spread == null ? text("unknown") : `${number(spread, 3)} USD/oz`)}${field(text("age"), quoteAge)}${field(text("book"), hasBook ? text("book_yes") : `${text("no_book")}${bookNote ? ` (${bookNote})` : ""}`)}${volume == null && !volumeKind ? "" : field(text("volume"), `${volume == null ? text("unknown") : number(volume, 0)}${volumeKind ? ` · ${volumeKind}` : ""}`)}</div>`;
  }
  function framesMarkup(frames) {
    if (!Array.isArray(frames) || !frames.length) return `<p class="obs-muted">${esc(text("unknown"))}</p>`;
    return `<div class="obs-frame-summaries">${frames.map(frame => {
      if (typeof frame === "string") return `<span class="obs-frame-chip">${esc(frame)}</span>`;
      const name = frame.timeframe ?? frame.frame ?? frame.tf ?? frame.id ?? text("unknown");
      const summary = frame.summary ?? frame.trend ?? frame.direction ?? frame.state ?? frame.status;
      const close = frame.last_closed?.close ?? frame.close ?? frame.price;
      const extra = [summary, close == null ? null : number(close, 3), frame.support == null ? null : `S ${number(frame.support, 3)}`, frame.resistance == null ? null : `R ${number(frame.resistance, 3)}`].filter(value => value != null).map(value => typeof value === "object" ? JSON.stringify(value) : String(value)).join(" · ");
      return `<div class="obs-frame-summary"><strong>${esc(name)}</strong><span>${esc(extra || text("unknown"))}</span></div>`;
    }).join("")}</div>`;
  }
  function decisionMarkup(item) {
    if (!item) return `<p class="obs-muted">${esc(text("no_decision"))}</p>`;
    const decision = item.decision || {};
    const accepted = !!item.accepted;
    const strat = STRATEGIES.find(s => s.id === item.strategy_id);
    const title = strat ? (lang() === "ru" ? strat.title : strat[lang()] || strat.en) : (item.strategy_id || text("unknown"));
    const action = decision.action ?? decision.side ?? decision.signal ?? text("unknown");
    return `<div class="obs-decision"><div class="obs-decision-head"><strong>${esc(title)}</strong><span class="obs-pill ${accepted ? "is-accepted" : "is-rejected"}">${esc(text(accepted ? "accepted" : "rejected"))}</span></div>${field(text("candidate"), action)}${field(text("reason"), item.reason ?? decision.reason ?? text("unknown"))}${item.elapsed_ms == null ? "" : field(text("elapsed"), `${number(item.elapsed_ms, 0)} ms`)}</div>`;
  }
  function positionMarkup(position) {
    if (!position) return `<p class="obs-muted">${esc(text("none"))}</p>`;
    return `<div class="obs-metrics">${field(text("candidate"), position.side ?? position.direction ?? position.action ?? text("unknown"))}${field(text("entry"), `${number(position.entry_price ?? position.entry ?? position.price, 3)} USD/oz`)}${field(text("units"), number(position.quantity_oz ?? position.quantity ?? position.units, 4))}${position.unrealized_pnl == null && position.pnl == null ? "" : field(text("pnl"), `${number(position.unrealized_pnl ?? position.pnl, 2)} USD`)}</div>`;
  }
  function phaseLabel(phase, running) {
    if (!phase) return running ? text("running") : text("stopped");
    const value = String(phase).toLowerCase();
    if (value.includes("collecting")) return text("phaseCollecting");
    if (value.includes("waiting for fresh")) return text("phaseWaiting");
    if (value.includes("inference failed")) return text("phaseInference");
    if (value.includes("analysing") || value.includes("analyzing")) {
      const strategy = STRATEGIES.find(item => value.includes(item.id));
      return `${text("strict")}: ${strategy ? (lang() === "ru" ? strategy.title : strategy[lang()] || strategy.en) : "Qwen"}`;
    }
    if (value.includes("decision recorded")) return text("phaseRecorded");
    if (value.includes("stopped")) return text("stopped");
    if (value.includes("observing")) return text("running");
    return phase;
  }
  function updateStatus(root, state) {
    const status = state.status || {};
    const running = !!status.running;
    const backendBusy = !!status.busy || ["starting", "saving", "stopping", "busy"].includes(status.phase);
    const busy = state.busy || backendBusy;
    const phase = phaseLabel(status.phase, running);
    const stateNode = root.querySelector("[data-status-line]");
    const statusError = status.error || state.error;
    if (stateNode) stateNode.innerHTML = `<span class="obs-status-dot ${running ? "is-running" : ""}"></span><strong>${esc(phase)}</strong>${statusError ? `<span class="obs-status-error">${esc(statusError)}</span>` : ""}`;
    const action = root.querySelector('[data-action="run"]');
    if (action) {
      action.textContent = running ? text("stop") : text("start");
      action.dataset.run = running ? "stop" : "start";
      action.classList.toggle("is-stop", running);
      action.disabled = (busy && !running) || !state.loaded;
    }
    const hasPosition = !!status.position;
    const lock = running || busy || hasPosition;
    root.querySelectorAll(".obs-form input, .obs-form textarea, .obs-form button").forEach(node => {
      if (node.type === "range" || node.dataset.config || node.dataset.frame || node.type === "checkbox" || node.dataset.action === "save" || node.dataset.action === "defaults") node.disabled = lock;
    });
    const equity = root.querySelector('[data-config="initial_equity"]');
    if (equity) equity.disabled = lock || status.decisions > 0;
    const equityLock = root.querySelector("[data-equity-lock]");
    if (equityLock) equityLock.hidden = !(status.decisions > 0);
    const notice = root.querySelector("[data-edit-notice]");
    if (notice) notice.textContent = running ? text("stopped_edit") : hasPosition ? text("position_lock") : busy ? text("busy") : state.config && !jsonEqual(state.config, state.saved) ? text("dirty") : state.notice;
    const save = root.querySelector('[data-action="save"]');
    if (save) save.disabled = lock || !state.loaded;
    const start = root.querySelector('[data-action="run"]');
    if (start) start.disabled = (busy && !running) || !state.loaded;
    root.querySelector("[data-quote]").innerHTML = `<h3>${esc(text("quote"))}</h3>${sampleMarkup(status.sample)}`;
    root.querySelector("[data-frames]").innerHTML = `<h3>${esc(text("frames_live"))}</h3>${framesMarkup(status.frames)}`;
    root.querySelector("[data-decision]").innerHTML = `<h3>${esc(text("decision"))}</h3>${decisionMarkup(status.last_decision)}`;
    root.querySelector("[data-position]").innerHTML = `<h3>${esc(text("position"))}</h3>${positionMarkup(status.position)}`;
    root.querySelector("[data-stats]").innerHTML = `<h3>${esc(text("stats"))}</h3><div class="obs-metrics">${field(text("equity_now"), `${number(status.equity, 2)} USD`)}${field(text("decisions"), number(status.decisions, 0))}${field(text("outcomes"), number(status.outcomes, 0))}</div>`;
    root.dataset.running = running ? "true" : "false";
  }
  function readForm(root, state) {
    const config = clone(state.config);
    root.querySelectorAll("[data-config]").forEach(node => {
      const key = node.dataset.config;
      if (key === "enabled" || key === "prompt") {
        const item = config.strategies.find(s => s.id === node.closest("[data-strategy]")?.dataset.strategy);
        if (item) item[key] = node.type === "checkbox" ? node.checked : node.value;
      } else if (key === "strictness") config.strictness = Number(node.value);
      else if (node.value !== "") config[key] = Number(node.value);
    });
    root.querySelectorAll("[data-frame]").forEach(node => {
      const item = config.strategies.find(s => s.id === node.closest("[data-strategy]")?.dataset.strategy);
      if (!item) return;
      const set = new Set(item.timeframes);
      if (node.checked) set.add(node.dataset.frame); else set.delete(node.dataset.frame);
      item.timeframes = FRAMES.map(([id]) => id).filter(id => set.has(id));
    });
    state.config = config;
    const out = root.querySelector("[data-strict-value]");
    if (out) out.value = String(config.strictness);
    const notice = root.querySelector("[data-edit-notice]");
    if (notice) notice.textContent = !jsonEqual(state.config, state.saved) ? text("dirty") : "";
  }
  function validateConfig(config) {
    if (!config.strategies.some(strategy => strategy.enabled)) return text("need_strategy");
    for (const strategy of config.strategies) {
      if (!strategy.prompt.trim() || strategy.prompt.length > 512) return text("prompt_invalid");
      if (!strategy.timeframes.length) return text("timeframe_invalid");
    }
    if (!(config.risk_pct > 0 && config.risk_pct <= 1) || !(config.initial_equity > 0) || !(config.max_spread > 0) || config.commission_per_oz < 0 || config.slippage < 0) return text("config_invalid");
    return "";
  }
  async function saveConfig(root, state) {
    const formError = root.querySelector("[data-form-error]");
    formError.textContent = "";
    const invalid = validateConfig(state.config);
    if (invalid) { formError.textContent = invalid; throw new Error(invalid); }
    const form = root.querySelector(".obs-form");
    if (form && !form.reportValidity()) throw new Error("Invalid configuration fields");
    const wasBusy = state.busy;
    state.busy = true;
    updateStatus(root, state);
    try {
      const config = await invoke("observer_save", { config: state.config });
      state.config = cfgFrom(config);
      state.saved = clone(state.config);
      state.notice = text("saved");
      root.querySelector("[data-form-error]").textContent = "";
      root.querySelector("[data-config-form]").innerHTML = configMarkup(state.config);
    } catch (error) {
      root.querySelector("[data-form-error]").textContent = String(error?.message || error);
      throw error;
    } finally {
      state.busy = wasBusy;
      updateStatus(root, state);
    }
  }
  async function refreshStatus(root, state) {
    if (state.refreshing) return;
    state.refreshing = true;
    try {
      state.status = await invoke("observer_status");
      state.error = "";
    } catch (error) {
      state.error = String(error?.message || error);
      const node = root.querySelector("[data-status-line]");
      if (node) node.innerHTML = `<span class="obs-status-dot"></span><strong>${esc(text("error"))}</strong><span class="obs-status-error">${esc(state.error)}</span>`;
    }
    state.refreshing = false;
    if (root.isConnected) updateStatus(root, state);
  }
  function bind(root, state) {
    if (state.bound) return;
    state.bound = true;
    root.addEventListener("input", event => {
      if (event.target.matches("[data-config], [data-frame]")) readForm(root, state);
    });
    root.addEventListener("change", event => {
      if (event.target.matches("[data-config], [data-frame]")) readForm(root, state);
    });
    root.addEventListener("submit", async event => {
      if (!event.target.matches(".obs-form")) return;
      event.preventDefault();
      try { await saveConfig(root, state); } catch { /* Error is shown beside the form. */ }
    });
    root.addEventListener("click", async event => {
      const button = event.target.closest("[data-action]");
      if (!button || button.disabled) return;
      const action = button.dataset.action;
      if (action === "refresh") return refreshStatus(root, state);
      if (action === "journal") {
        button.disabled = true;
        try {
          const data = await invoke("observer_journal");
          const content = typeof data === "string" ? data : JSON.stringify(data, null, 2);
          if (!content) { state.notice = text("journal_empty"); updateStatus(root, state); return; }
          const url = URL.createObjectURL(new Blob([content], { type: "application/x-ndjson;charset=utf-8" }));
          const link = document.createElement("a"); link.href = url; link.download = "aegis-observer.jsonl"; link.click(); setTimeout(() => URL.revokeObjectURL(url), 1000);
        } catch (error) { state.error = `${text("download_error")}: ${String(error?.message || error)}`; updateStatus(root, state); }
        finally { button.disabled = false; }
      }
      if (action === "defaults") {
        state.config = cfgFrom(state.defaults || DEFAULTS);
        state.notice = text("defaults_done");
        root.querySelector("[data-config-form]").innerHTML = configMarkup(state.config);
        updateStatus(root, state);
      }
      if (action === "run") {
        state.busy = true;
        state.error = "";
        root.querySelector("[data-action-error]").textContent = "";
        updateStatus(root, state);
        try {
          if (button.dataset.run === "stop") state.status = await invoke("observer_stop");
          else {
            if (!jsonEqual(state.config, state.saved)) await saveConfig(root, state);
            state.notice = text("run_saved");
            state.status = await invoke("observer_start");
          }
        } catch (error) {
          state.error = String(error?.message || error);
          const node = root.querySelector("[data-action-error]");
          if (node) node.textContent = state.error;
        } finally {
          state.busy = false;
          updateStatus(root, state);
          refreshStatus(root, state);
        }
      }
    });
  }
  async function load(root, state) {
    const token = ++state.loadToken;
    try {
      const info = await invoke("observer_info");
      if (!root.isConnected || token !== state.loadToken) return;
      const config = cfgFrom(info.config);
      state.defaults = info.defaults ? cfgFrom(info.defaults) : clone(DEFAULTS);
      state.config = config;
      state.saved = clone(config);
      state.status = info.status || null;
      state.loaded = true;
      root.querySelector("[data-config-form]").innerHTML = configMarkup(config);
      root.querySelector("[data-preflight]").textContent = text("preflight");
      updateStatus(root, state);
      await refreshStatus(root, state);
    } catch (error) {
      if (!root.isConnected || token !== state.loadToken) return;
      state.error = String(error?.message || error);
      root.querySelector("[data-status-line]").innerHTML = `<strong>${esc(text("error"))}</strong><span class="obs-status-error">${esc(state.error)}</span>`;
      root.querySelector("[data-config-form]").innerHTML = `<p class="obs-status-error" role="alert">${esc(state.error)}</p>`;
    }
  }
  function render(container, focus) {
    if (!container) return;
    let root = container.querySelector(":scope > .obs-root");
    let state = root && STATES.get(root);
    if (!root) {
      root = document.createElement("div");
      root.className = "obs-root";
      root.dataset.testid = "observer-panel";
      container.replaceChildren(root);
      state = makeState();
      STATES.set(root, state);
      root.innerHTML = `<div class="obs-heading"><div><span class="obs-kicker">AI · PAPER</span><h2>AI Observer</h2><p>${esc(text("intro"))}</p></div><button class="ghost sm" type="button" data-action="refresh">${esc(text("refresh"))}</button></div>
        <div class="obs-safety" role="note">${esc(text("no_orders"))}</div><p class="obs-help obs-stop-note">${esc(text("stop_protect"))}</p><p class="obs-preflight" data-preflight>${esc(text("preflight"))}</p>
        <section class="obs-card obs-status-card"><div class="obs-status-line" data-status-line role="status" aria-live="polite"><span class="obs-status-dot"></span><strong>${esc(text("loading"))}</strong></div><div class="obs-actions obs-run-actions"><button class="cta" type="button" data-action="run" data-run="start" disabled>${esc(text("start"))}</button><button class="ghost" type="button" data-action="journal">${esc(text("journal"))}</button><span class="obs-inline-error" data-action-error role="alert"></span></div></section>
        <div data-config-form><p class="obs-muted">${esc(text("loading"))}</p></div>
        <section class="obs-card" data-quote><h3>${esc(text("quote"))}</h3><p class="obs-muted">${esc(text("no_sample"))}</p></section>
        <section class="obs-card" data-frames><h3>${esc(text("frames_live"))}</h3></section><section class="obs-card" data-decision><h3>${esc(text("decision"))}</h3></section>
        <section class="obs-card" data-position><h3>${esc(text("position"))}</h3></section><section class="obs-card" data-stats><h3>${esc(text("stats"))}</h3></section>`;
      bind(root, state);
      load(root, state);
    } else if (focus) {
      root.querySelector(`[data-strategy="${CSS.escape(focus)}"]`)?.scrollIntoView({ behavior: "smooth", block: "center" });
    }
    if (state?.timer) clearInterval(state.timer);
    if (state) {
      state.timer = setInterval(() => {
        if (!root.isConnected || (window.AEGIS?.S?.panel && window.AEGIS.S.panel !== "observer")) {
          clearInterval(state.timer); state.timer = null; return;
        }
        refreshStatus(root, state);
      }, 1000);
    }
  }
  window.AEGIS = window.AEGIS || {};
  window.AEGIS.observer = { render };
})();
