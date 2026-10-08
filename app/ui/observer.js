"use strict";
(() => {
  const STRATEGIES = [
    { id: "density_bounce", title: "Bounce", en: "Bounce", kk: "Bounce", prompt: "Ищи отбой цены от подтверждённого уровня. При наличии стакана учитывай реальную плотность; иначе явно отмечай ценовой прокси. Вход только после закрытого бара с отбоем; иначе жди." },
    { id: "structural", title: "Structural", en: "Structural", kk: "Structural", prompt: "Ищи структурный разворот: вынос прошлого swing-экстремума, возврат за него и подтверждённый сдвиг закрытия. Не считай один прокол разворотом; иначе жди." },
    { id: "liquidity_sweep", title: "Liquidity Sweep", en: "Liquidity Sweep", kk: "Liquidity Sweep", prompt: "Ищи снятие ликвидности за прошлым swing high/low и возврат закрытием обратно за уровень. Не заявляй о видимой ликвидности без подтверждающих данных; иначе жди." },
    { id: "data", title: "DATA", en: "DATA", kk: "DATA", prompt: "Анализируй только предоставленные рыночные данные; отмечай неопределённость и выбирай ожидание, если данных недостаточно." },
    { id: "breakout", title: "Breakout", en: "Breakout", kk: "Breakout", prompt: "Ищи пробой уровня закрытием за пределами прошлого диапазона и удержание цены за уровнем. Не входи на одном касании или незакрытой свече; иначе жди." },
  ];
  const FRAMES = [["1m", "M1"], ["5m", "M5"], ["15m", "M15"], ["1h", "H1"], ["4h", "H4"], ["1d", "D1"]];
  const DEFAULTS = {
    strictness: 100,
    strategies: STRATEGIES.map((s) => ({ id: s.id, enabled: true, prompt: JSON.stringify({ objective: s.prompt, entry_rules: ["Use only closed-candle confirmation."], exit_rules: ["Exit when the setup is invalidated."], timeframes: ["1m", "5m", "15m", "1h", "4h", "1d"] }, null, 2), timeframes: FRAMES.map(([id]) => id), risk_pct: 0.5, max_positions: 1 })),
    risk_pct: 0.5,
    initial_equity: 10000,
    max_spread: 1,
    commission_per_oz: 0,
    slippage: 0,
    ai_enabled: false,
    mode: "paper",
    broker: "roboforex",
  };
  const LABELS = {
    ru: {
      ai_analysis: "Включить AI-анализ стратегий", execution_mode: "Режим исполнения сделок", paper_label: "Paper — симуляция", money_label: "Реальные деньги", ai_analysis_help: "AI включён — разрешён анализ стратегий, а не торговля реальными деньгами. Исполнение отдельно определяется режимом и разрешением сессии.", execution_help: "Paper симулирует сделки без ордеров брокеру. Реальные деньги — исполнение через брокера; выбор режима сам по себе не даёт разрешения.", authorization_help: "Разрешение сессии (money_armed) даётся отдельно подтверждением ниже. Оно не заменяет другие проверки безопасности.", strict_permission_help: "Строгость — соответствие инструкциям стратегии, не разрешение на исполнение сделок.",
      intro: "AI для RoboForex MT5 · XAUUSD: Paper по умолчанию и денежный режим только с подтверждением.", no_orders: "В Paper реальные ордера не отправляются. Денежный режим может отправлять сделки только после отдельного подтверждения на эту сессию.", stop_protect: "Остановка прекращает новые AI-анализы. Защита уже открытых позиций продолжает действовать.",
      strategies: "Стратегии", enabled: "Включена", prompt: "Инструкция стратегии для ИИ", frames: "Таймфреймы для анализа",
      strict: "Следование стратегиям", strict_help: "100 — строго по инструкциям стратегий; 0 — больше свободы модели. На малых значениях ИИ может анализировать и дополнительные таймфреймы.",
      strict_strategy: "Строго по стратегиям", free: "Свободнее", position_lock: "Есть открытая Paper-позиция; дождитесь её закрытия перед изменением настроек.", equity_locked: "Начальный капитал нельзя изменить после первого решения.", risk: "Риск на Paper-сделку, %", equity: "Начальный Paper-капитал, USD", spread: "Максимальный спред, USD/oz", commission: "Комиссия за одну сторону, USD/oz (round-trip ×2)", slippage: "Проскальзывание, USD/oz", costs: "Риск и издержки симуляции",
      save: "Сохранить настройки", start: "Запустить наблюдение", stop: "Остановить", refresh: "Обновить", defaults: "Восстановить исходные настройки", journal: "Скачать журнал JSONL", saved: "Настройки сохранены.", dirty: "Есть несохранённые изменения. Запуск сначала сохранит их.", loading: "Загрузка конфигурации…", running: "Наблюдение активно", stopped: "Остановлено", busy: "Выполняется операция…", error: "Ошибка", phaseCollecting: "Сбор таймфреймов RoboForex", phaseWaiting: "Ожидание свежих котировок MT5; новые входы приостановлены", phaseInference: "Ошибка анализа; новые входы приостановлены", phaseRecorded: "Решение записано; наблюдение продолжается", no_conn: "Для запуска подключите RoboForex MT5 и инструмент XAUUSD. Проверьте готовность локальной модели Qwen в настройках ИИ.", stopped_edit: "Чтобы менять конфигурацию, сначала остановите наблюдение.", quote: "Котировка XAUUSD", bid: "Bid", ask: "Ask", spread_now: "Спред", age: "Возраст котировки", book: "Стакан", no_book: "Стакан недоступен — используем имеющиеся котировки и данные.", book_yes: "Стакан доступен", volume: "Объём", ticks: "Тиковый объём CFD, не лента биржевых сделок", frames_live: "Сводки таймфреймов", decision: "Последнее решение ИИ", position: "Paper-позиция", none: "Нет открытой Paper-позиции", equity_now: "Paper-капитал", stats: "Счётчики", decisions: "Решения", outcomes: "Исходы", phase: "Фаза", candidate: "Решение", accepted: "Принято", rejected: "Отклонено", reason: "Причина", elapsed: "Время", entry: "Вход", units: "Количество, oz", pnl: "Нереализованный P/L", no_sample: "Ожидание рыночных данных…", no_decision: "Решений пока нет.", journal_empty: "Журнал пока пуст.", download_error: "Не удалось скачать журнал", preflight: "Перед запуском проверьте: подключён RoboForex MT5 с XAUUSD; локальная модель Qwen установлена и запущена. Backend также проверит все ограничения безопасности.", run_saved: "Конфигурация сохранена; запускаю наблюдение…", defaults_done: "Показаны исходные настройки. Сохраните их, чтобы применить.", unknown: "—", need_strategy: "Включите хотя бы одну стратегию.", prompt_invalid: "Инструкция каждой стратегии должна содержать от 1 до 512 символов.", timeframe_invalid: "Для каждой стратегии выберите хотя бы один таймфрейм.", config_invalid: "Проверьте риск (0–1%), капитал, спред, комиссию и проскальзывание.", phaseNames: { idle: "Ожидание", starting: "Запуск", observing: "Наблюдение", analyzing: "Анализ", error: "Ошибка" },
    },
    en: {
      ai_analysis: "Enable AI strategy analysis", execution_mode: "Trade execution mode", paper_label: "Paper — simulation", money_label: "Real money", ai_analysis_help: "AI enabled permits strategy analysis, not real-money trading. Execution is separately determined by mode and session authorization.", execution_help: "Paper simulates trades without broker orders. Real money executes through the broker; selecting the mode alone does not authorize it.", authorization_help: "Session authorization (money_armed) is granted separately by the confirmation below. It does not bypass other safety checks.", strict_permission_help: "Strictness means conformance to strategy instructions, not permission to execute trades.",
      intro: "AI for RoboForex MT5 · XAUUSD: Paper by default; Money mode requires explicit confirmation.", no_orders: "Paper sends no broker orders. Money mode may place trades only after a separate confirmation for this session.", phaseCollecting: "Collecting RoboForex timeframes", phaseWaiting: "Waiting for fresh MT5 quotes; new entries paused", phaseInference: "Analysis failed; new entries paused", phaseRecorded: "Decision recorded; observation continues", stop_protect: "Stopping ends new AI analysis. Protection for already-open positions remains active.", position_lock: "An open Paper position remains; wait for it to close before editing settings.", equity_locked: "Initial equity cannot change after the first decision.", strategies: "Strategies", enabled: "Enabled", prompt: "Strategy instructions for AI", frames: "Analysis timeframes", strict: "Strategy adherence", strict_help: "100 means strictly follow strategy instructions; 0 gives the model more freedom. At low values AI may also analyze additional timeframes.", strict_strategy: "Strict strategies", free: "More freedom", risk: "Paper trade risk, %", equity: "Initial Paper equity, USD", spread: "Maximum spread, USD/oz", commission: "Commission per side, USD/oz (round trip ×2)", slippage: "Slippage, USD/oz", costs: "Simulation risk and costs", save: "Save settings", start: "Start observation", stop: "Stop", refresh: "Refresh", defaults: "Restore defaults", journal: "Download JSONL journal", saved: "Settings saved.", dirty: "Unsaved changes. Start will save them first.", loading: "Loading configuration…", running: "Observation active", stopped: "Stopped", busy: "Operation in progress…", error: "Error", no_conn: "Connect RoboForex MT5 with XAUUSD to start. Check that the local Qwen model is ready in AI settings.", stopped_edit: "Stop observation before editing configuration.", quote: "XAUUSD quote", bid: "Bid", ask: "Ask", spread_now: "Spread", age: "Quote age", book: "Order book", no_book: "Order book unavailable — using available quotes and data.", book_yes: "Order book available", volume: "Volume", ticks: "CFD tick volume, not exchange time-and-sales", frames_live: "Timeframe summaries", decision: "Latest AI decision", position: "Paper position", none: "No open Paper position", equity_now: "Paper equity", stats: "Counters", decisions: "Decisions", outcomes: "Outcomes", phase: "Phase", candidate: "Decision", accepted: "Accepted", rejected: "Rejected", reason: "Reason", elapsed: "Time", entry: "Entry", units: "Quantity, oz", pnl: "Unrealized P/L", no_sample: "Waiting for market data…", no_decision: "No decisions yet.", journal_empty: "Journal is empty.", download_error: "Could not download journal", preflight: "Before starting, check that RoboForex MT5 with XAUUSD is connected and local Qwen is installed and running. The backend also enforces all safety limits.", run_saved: "Configuration saved; starting observation…", defaults_done: "Defaults loaded. Save them to apply.", unknown: "—", need_strategy: "Enable at least one strategy.", prompt_invalid: "Each strategy prompt must contain 1 to 512 characters.", timeframe_invalid: "Select at least one timeframe for every strategy.", config_invalid: "Check risk (0–1%), equity, spread, commission, and slippage.", phaseNames: { idle: "Idle", starting: "Starting", observing: "Observing", analyzing: "Analyzing", error: "Error" },
    },
    kk: {
      ai_analysis: "AI стратегия талдауын қосу", execution_mode: "Мәмілені орындау режимі", paper_label: "Paper — симуляция", money_label: "Нақты ақша", ai_analysis_help: "AI қосу стратегияны талдауға мүмкіндік береді, нақты ақшамен саудаға рұқсат бермейді. Орындау режим мен сессия рұқсатына бөлек тәуелді.", execution_help: "Paper брокерге ордер жібермей мәмілелерді симуляциялайды. Нақты ақша — брокер арқылы орындау; режимді таңдау өздігінен рұқсат бермейді.", authorization_help: "Сессия рұқсаты (money_armed) төмендегі растау арқылы бөлек беріледі. Ол басқа қауіпсіздік тексерулерін алмастырмайды.", strict_permission_help: "Қатаңдық — стратегия нұсқауларына сәйкестік, мәмілені орындауға рұқсат емес.",
      intro: "RoboForex MT5 · XAUUSD үшін ЖИ: әдепкіде Paper, ақша режимі бөлек растауды талап етеді.", no_orders: "Paper брокерге ордер жібермейді. Ақша режимі тек осы сеансқа бөлек растаудан кейін мәміле аша алады.", stop_protect: "Тоқтату жаңа ЖИ талдауын тоқтатады. Ашық позициялардың қорғанысы жалғасады.", position_lock: "Ашық Paper позициясы бар; баптауларды өзгерту үшін оның жабылуын күтіңіз.", equity_locked: "Бірінші шешімнен кейін бастапқы капиталды өзгертуге болмайды.", strategies: "Стратегиялар", enabled: "Қосулы", prompt: "ЖИ стратегиясының нұсқауы", frames: "Талдау таймфреймдері", strict: "Стратегияны сақтау", strict_help: "100 — стратегия нұсқауын қатаң орындау; 0 — модель еркіндігі жоғары. Төмен мәндерде ЖИ қосымша таймфреймдерді талдауы мүмкін.", strict_strategy: "Стратегияны қатаң сақтау", free: "Еркінірек", risk: "Paper мәміле тәуекелі, %", equity: "Бастапқы Paper капиталы, USD", spread: "Ең үлкен спред, USD/oz", commission: "Бір жаққа комиссия, USD/oz (round-trip ×2)", slippage: "Сырғу, USD/oz", costs: "Симуляция тәуекелі мен шығындары", save: "Баптауларды сақтау", start: "Бақылауды бастау", stop: "Тоқтату", refresh: "Жаңарту", defaults: "Әдепкі баптауларды қалпына келтіру", journal: "JSONL журналын жүктеу", saved: "Баптаулар сақталды.", dirty: "Сақталмаған өзгерістер бар. Бастау алдында сақталады.", loading: "Конфигурация жүктелуде…", running: "Бақылау белсенді", stopped: "Тоқтатылды", busy: "Операция орындалуда…", error: "Қате", no_conn: "Бастау үшін XAUUSD бар RoboForex MT5 қосыңыз. ЖИ баптауларында жергілікті Qwen дайын екенін тексеріңіз.", stopped_edit: "Конфигурацияны өзгерту үшін алдымен бақылауды тоқтатыңыз.", quote: "XAUUSD бағасы", bid: "Bid", ask: "Ask", spread_now: "Спред", age: "Баға жасы", book: "Өтімділік кітабы", no_book: "Кітап қолжетімсіз — бар баға мен деректі қолданамыз.", book_yes: "Кітап қолжетімді", volume: "Көлем", ticks: "CFD тик көлемі, биржалық мәмілелер таспасы емес", frames_live: "Таймфрейм қорытындылары", decision: "ЖИ соңғы шешімі", position: "Paper позициясы", none: "Ашық Paper позициясы жоқ", equity_now: "Paper капиталы", stats: "Санағыштар", decisions: "Шешімдер", outcomes: "Нәтижелер", phase: "Кезең", candidate: "Шешім", accepted: "Қабылданды", rejected: "Қабылданбады", reason: "Себеп", elapsed: "Уақыт", entry: "Кіру", units: "Саны, oz", pnl: "Іске асырылмаған P/L", no_sample: "Нарық дерегі күтілуде…", no_decision: "Шешім әзірге жоқ.", journal_empty: "Журнал бос.", download_error: "Журналды жүктеу мүмкін болмады", preflight: "Бастамас бұрын XAUUSD бар RoboForex MT5 қосылғанын және жергілікті Qwen орнатылып, іске қосылғанын тексеріңіз. Backend қауіпсіздік шектеулерін де тексереді.", run_saved: "Конфигурация сақталды; бақылау басталуда…", defaults_done: "Әдепкі баптаулар көрсетілді. Қолдану үшін сақтаңыз.", unknown: "—", need_strategy: "Кемінде бір стратегияны қосыңыз.", prompt_invalid: "Әр стратегия нұсқауы 1–512 таңбадан тұруы керек.", timeframe_invalid: "Әр стратегия үшін кемінде бір таймфрейм таңдаңыз.", config_invalid: "Тәуекелді (0–1%), капиталды, спредті, комиссияны және сырғуды тексеріңіз.", phaseNames: { idle: "Күту", starting: "Іске қосу", observing: "Бақылау", analyzing: "Талдау", error: "Қате" },
    },
  };
  const STATES = new WeakMap();
  const PRICE_LINES = new Map();
  let closingPosition = false;

  function makeState() {
    return { config: null, saved: null, status: null, loaded: false, busy: false, notice: "", error: "", bound: false, timer: null, loadToken: 0 };
  }
  function lang() {
    const value = window.I18N?.lang || "ru";
    return LABELS[value] ? value : "ru";
  }
  function text(key) {
    const ru = {
      "AI settings": "AI", "AI control": "Режим торговли", "Enable AI strategies": "AI-торговля",
      "Trading mode": "Как торгуем", "Broker": "Брокер", "Risk per strategy, %": "Риск стратегии, %",
      "Maximum positions": "Максимум позиций (денежный режим)", "Daily loss limit, %": "Дневной лимит убытка, %",
      "Edit strategy SPA": "SPA стратегии", "SPA — System Prompt Algorithm": "SPA — Системный промпт алгоритма",
      "Edit SPA": "Редактировать SPA", "Save SPA": "Сохранить SPA", "SPA saved.": "SPA сохранён.",
      "Balance": "Баланс", "Equity": "Средства", "Free margin": "Свободная маржа", "Profit": "Прибыль",
      "RoboForex account": "Счёт RoboForex", "Close position": "Закрыть по рынку", "Close all AI positions": "Закрыть все позиции AEGIS по рынку",
      "Diagnostics": "Проверка цикла и задержек", "Run readiness checks": "Проверить цикл", "Test provider": "Проверить API",
      "OpenRouter provider": "Anthropic через OpenRouter", "API key": "Ключ API", "Save provider": "Сохранить ключ",
      "Model": "Модель", "Leave empty to keep the stored key": "Пустое поле сохраняет текущий ключ",
      "Click again to confirm.": "Нажмите ещё раз для подтверждения.",
      "No AI-owned XAUUSD positions.": "Нет открытых позиций AEGIS на XAUUSD.",
      "Observation has been stopped. Existing positions remain protected. Click Edit SPA again to continue.": "Новые входы остановлены. Защита открытых позиций остаётся активной. Нажмите «Редактировать SPA» ещё раз.",
      "This will send a billable OpenRouter API request. Continue?": "Будет отправлен оплачиваемый запрос OpenRouter. Продолжить?",
      "Full readiness checks send a billable AI provider request but never place an order. Continue?": "Проверка отправляет оплачиваемый API-запрос; ордера не выставляются. Продолжить?",
    };
    if (lang() === "ru" && ru[key]) return ru[key];
    return LABELS[lang()][key] || LABELS.ru[key] || window.I18N?.t(key) || key;
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
  function confirmAction(root, button, message) {
    if (button.dataset.confirmed === "yes") {
      delete button.dataset.confirmed;
      root.querySelector('[data-confirm-note]')?.remove();
      return true;
    }
    button.dataset.confirmed = "yes";
    root.querySelector('[data-confirm-note]')?.remove();
    const note = document.createElement("p");
    note.dataset.confirmNote = "";
    note.className = "obs-safety";
    note.textContent = `${message} ${text("Click again to confirm.")}`;
    button.parentElement.append(note);
    return false;
  }
  function cfgFrom(source) {
    const input = source && typeof source === "object" ? source : {};
    const strategies = STRATEGIES.map((def) => {
      const item = (input.strategies || []).find((s) => s.id === def.id) || {};
      const defaults = DEFAULTS.strategies.find((s) => s.id === def.id);
      return { id: def.id, enabled: item.enabled == null ? true : !!item.enabled, prompt: typeof item.prompt === "string" ? item.prompt : defaults.prompt, timeframes: FRAMES.map(([id]) => id).filter(id => (Array.isArray(item.timeframes) ? item.timeframes : defaults.timeframes).includes(id)), risk_pct: Number.isFinite(Number(item.risk_pct)) ? Number(item.risk_pct) : defaults.risk_pct, max_positions: Number.isFinite(Number(item.max_positions)) ? Number(item.max_positions) : defaults.max_positions, max_daily_loss_pct: Number.isFinite(Number(item.max_daily_loss_pct)) ? Number(item.max_daily_loss_pct) : 3 };
    });
    const number = (key) => Number.isFinite(Number(input[key])) ? Number(input[key]) : DEFAULTS[key];
    return { ...input, strictness: Math.min(100, Math.max(0, Math.round(number("strictness")))), strategies, risk_pct: number("risk_pct"), initial_equity: number("initial_equity"), max_spread: number("max_spread"), commission_per_oz: number("commission_per_oz"), slippage: number("slippage"), ai_enabled: !!input.ai_enabled, mode: input.mode === "money" ? "money" : "paper", broker: "roboforex" };
  }
  function configMarkup(config) {
    const lock = STATES.get(document.querySelector(".obs-root"));
    const disabled = lock?.status?.running || lock?.busy ? "disabled" : "";
    return `<form class="obs-form" data-testid="observer-config">
      <section class="obs-card"><h3>${esc(text("AI control"))}</h3>
        <label class="obs-toggle obs-ai-toggle"><input type="checkbox" data-config="ai_enabled" ${config.ai_enabled ? "checked" : ""} ${disabled}><span>${esc(text("ai_analysis"))}</span></label>
        <p class="obs-help">${esc(text("ai_analysis_help"))}</p>
        <div class="obs-numbers"><label class="obs-label">${esc(text("execution_mode"))}<select data-config="mode" ${disabled}><option value="paper" ${config.mode === "paper" ? "selected" : ""}>${esc(text("paper_label"))}</option><option value="money" ${config.mode === "money" ? "selected" : ""}>${esc(text("money_label"))}</option></select></label><label class="obs-label">${esc(text("Broker"))}<select disabled><option>RoboForex</option></select></label></div>
        <p class="obs-help">${esc(text("execution_help"))}</p>
        <p class="obs-help">${esc(text("authorization_help"))}</p>
        <label class="obs-toggle obs-money-confirm"><input type="checkbox" data-money-confirm><span>${esc(text("I understand real-money trading can lose money and is not guaranteed."))}</span></label>
        <p class="obs-safety">${esc(text("Money mode warning. Live execution requires an explicit confirmation for this session. Verify the Windows demo first."))}</p>
      </section>
      <section class="obs-card"><h3>${esc(text("strategies"))}</h3><div class="obs-strategy-list">${config.strategies.map((strategy) => {
        const def = STRATEGIES.find(s => s.id === strategy.id);
        return `<fieldset class="obs-strategy" data-strategy="${esc(strategy.id)}" ${disabled}><legend>${esc(def[lang()] || def.en)}</legend>
          <label class="obs-toggle"><input type="checkbox" data-config="enabled" ${strategy.enabled ? "checked" : ""} ${disabled}><span>${esc(text("enabled"))}</span></label>
          <div class="obs-numbers"><label class="obs-label">${esc(text("Risk per strategy, %"))}<input type="range" min="0.01" max="1" step="0.01" value="${strategy.risk_pct}" data-config="risk_pct" ${disabled}><output>${strategy.risk_pct.toFixed(2)}%</output></label><label class="obs-label">${esc(text("Maximum positions"))}<input type="range" min="1" max="5" step="1" value="${strategy.max_positions}" data-config="max_positions" ${disabled}><output>${strategy.max_positions}</output></label></div>
          <span class="obs-subtitle">${esc(text("frames"))}</span><div class="obs-frames">${FRAMES.map(([id, title]) => `<label><input type="checkbox" data-frame="${id}" ${strategy.timeframes.includes(id) ? "checked" : ""} ${disabled}>${title}</label>`).join("")}</div>
          <label class="obs-label">${esc(text("Daily loss limit, %"))}<input type="range" min="0.1" max="10" step="0.1" value="${strategy.max_daily_loss_pct}" data-config="max_daily_loss_pct" ${disabled}><output>${strategy.max_daily_loss_pct}%</output></label>
          <button type="button" class="ghost sm" data-action="edit-strategy" data-strategy-id="${esc(strategy.id)}">${esc(text("Edit strategy SPA"))}</button>
        </fieldset>`;
      }).join("")}</div></section>
      <section class="obs-card"><div class="obs-section-head"><h3>${esc(text("strict"))}</h3><output class="obs-range-value" data-strict-value>${config.strictness}</output></div>
        <div class="obs-range-labels"><span>${esc(text("free"))}</span><span>${esc(text("strict_strategy"))}</span></div><input class="obs-range" type="range" min="0" max="100" step="1" value="${config.strictness}" data-config="strictness" aria-label="${esc(text("strict"))}" ${disabled}>
        <p class="obs-help">${esc(text("strict_help"))}</p>
        <p class="obs-help">${esc(text("strict_permission_help"))}</p>
      </section>
      <section class="obs-card"><h3>${esc(text("costs"))}</h3><div class="obs-numbers">${[["initial_equity", "equity", .01, 100000000, "any"], ["max_spread", "spread", .01, 100, .01], ["commission_per_oz", "commission", 0, 100, .01], ["slippage", "slippage", 0, 100, .01]].map(([key, label, min, max, step]) => `<label class="obs-label">${esc(text(label))}${key === "initial_equity" ? `<small class="obs-help" data-equity-lock ${lock?.status?.decisions > 0 ? "" : "hidden"}>${esc(text("equity_locked"))}</small>` : ""}<input type="number" data-config="${key}" min="${min}" max="${max}" step="${step}" value="${esc(config[key])}" required ${disabled} ${key === "initial_equity" && lock?.status?.decisions > 0 ? "disabled" : ""}></label>`).join("")}</div></section>
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
  function positionMarkup(position, mode) {
    if (!position) return `<p class="obs-muted">${esc(text("none"))}</p>`;
    return `<div class="obs-metrics">${field(text("candidate"), position.side ?? position.direction ?? position.action ?? text("unknown"))}${field(text("entry"), `${number(position.entry_price ?? position.entry ?? position.price, 3)} USD/oz`)}${field(text("units"), number(position.quantity_oz ?? position.quantity ?? position.units, 4))}${position.unrealized_pnl == null && position.pnl == null ? "" : field(text("pnl"), `${number(position.unrealized_pnl ?? position.pnl, 2)} USD`)}</div><button type="button" class="ghost sm" data-action="close-position" data-ticket="${esc(position.id ?? position.ticket)}">${esc(text(mode === "money" ? "Close live position at market" : "Close position"))}</button>`;
  }
  function accountMarkup(account, positions) {
    if (!account) return `<p class="obs-muted">${esc(text("Connect RoboForex to view account details."))}</p>`;
    const trading = account.account || account;
    const metrics = `<div class="obs-metrics">${field(text("Balance"), `${number(trading.balance, 2)} USD`)}${field(text("Equity"), `${number(trading.equity, 2)} USD`)}${field(text("Free margin"), `${number(trading.margin_free ?? trading.free_margin, 2)} USD`)}${field(text("Profit"), `${number(trading.profit, 2)} USD`)}</div>`;
    const own = (Array.isArray(positions) ? positions : account.positions || []).filter((position) => Number(position.magic) === 26070552 && String(position.symbol || "XAUUSD") === "XAUUSD");
    const list = own.length ? `<div class="obs-owned-positions">${own.map((position) => `<div class="obs-position-row"><span>#${esc(position.ticket)} · ${esc(position.side || position.type || "position")} · ${esc(number(position.quantity_oz ?? position.volume ?? position.quantity, 2))} oz</span><button type="button" class="ghost sm" data-action="close-position" data-ticket="${esc(position.ticket)}">${esc(text("Close position"))}</button></div>`).join("")}<button type="button" class="ghost sm" data-action="close-all">${esc(text("Close all AI positions"))}</button></div>` : `<p class="obs-muted">${esc(text("No AI-owned XAUUSD positions."))}</p>`;
    return `${metrics}${list}`;
  }
  function syncChartStatus(status) {
    syncQuickPositions(status);
    const chart = window.AEGIS?.S?.chart;
    const series = window.AEGIS?.candles;
    if (!chart || !series) return;
    const connectedToRoboForex = chart.broker === "roboforex";
    const positions = connectedToRoboForex ? (status?.live_positions || status?.account?.positions || []).filter((p) => Number(p.magic) === 26070552 && String(p.symbol || "XAUUSD") === "XAUUSD") : [];
    if (connectedToRoboForex && status?.position) positions.push({ ...status.position, ticket: `paper-${status.position.id}` });
    const wanted = new Set();
    positions.forEach((position) => {
      const ticket = String(position.ticket);
      const entry = Number(position.price_open ?? position.entry_price ?? position.entry);
      if (Number.isFinite(entry)) {
        const key = `${ticket}:entry`; wanted.add(key);
        const line = PRICE_LINES.get(key);
        if (line) { if (line.price !== entry) line.handle.applyOptions({ price: entry }); line.price = entry; }
        else PRICE_LINES.set(key, { price: entry, handle: series.createPriceLine({ price: entry, color: "#8b9cff", lineWidth: 1, lineStyle: 2, axisLabelVisible: true, title: `#${ticket} entry` }) });
      }
      for (const [role, value, color] of [["sl", position.sl ?? position.stop_loss ?? position.stop, "#ff6464"], ["tp", position.tp ?? position.take_profit ?? position.target, "#55d68b"]]) {
        const price = Number(value);
        if (!Number.isFinite(price) || price <= 0) continue;
        const key = `${ticket}:${role}`; wanted.add(key);
        const line = PRICE_LINES.get(key);
        if (line) { if (line.price !== price) line.handle.applyOptions({ price }); line.price = price; }
        else PRICE_LINES.set(key, { price, handle: series.createPriceLine({ price, color, lineWidth: 1, lineStyle: 2, axisLabelVisible: true, title: `#${ticket} ${role.toUpperCase()}` }) });
      }
    });
    for (const [key, line] of PRICE_LINES) {
      if (!wanted.has(key)) { series.removePriceLine(line.handle); PRICE_LINES.delete(key); }
    }
  }
  function syncQuickPositions(status) {
    let box=document.getElementById("positionActions");
    if (!box) {
      box=document.createElement("aside");box.id="positionActions";box.className="position-actions glass";
      box.addEventListener("click",async event=>{
        const button=event.target.closest("button");if(!button || button.disabled || closingPosition)return;
        closingPosition=true;
        box.querySelectorAll("button").forEach(b=>b.disabled=true);
        try {
          const result=button.dataset.closeAll ? await invoke("observer_close_all") : await invoke("observer_close",{ticket:Number(button.dataset.ticket)});
          const outcomes=Array.isArray(result)?result:[result];
          const failures=outcomes.filter(r=>r.status && r.status!=="filled");
          if(failures.length)throw new Error(failures.map(r=>`${r.status}: ${r.message}`).join("; "));
          window.AEGIS.log(text("Position closure confirmed by broker."),"ok");
          syncChartStatus(await invoke("observer_status"));
        } catch(error) {window.AEGIS.log(String(error),"bad");}
        finally {closingPosition=false;box.querySelectorAll("button").forEach(b=>b.disabled=false);}
      });
      document.body.append(box);
    }
    const own=(status?.account?.positions||[]).filter(p=>p.magic===26070552&&p.symbol==="XAUUSD");
    if(status?.position)own.push({...status.position,ticket:status.position.id});
    box.hidden=!own.length;
    if(!own.length)return;
    box.innerHTML=`<strong>RoboForex · XAUUSD</strong>${own.map(p=>`<button type="button" class="ghost sm" data-ticket="${esc(p.ticket)}">#${esc(p.ticket)} · ${esc(text("Close position"))}</button>`).join("")}<button type="button" class="ghost sm" data-close-all="yes">${esc(text("Close all AI positions"))}</button>`;
    box.querySelectorAll("button").forEach(button=>{button.disabled=closingPosition;});
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
    if (!root.isConnected) return;
    const status = state.status || {};
    const running = !!status.running;
    const backendBusy = !!status.busy || ["starting", "saving", "stopping", "busy"].includes(status.phase);
    const busy = state.busy || backendBusy;
    const phase = phaseLabel(status.phase, running);
    const providerState = root.querySelector("[data-provider-state]");
    if (providerState && status.provider_ready) providerState.textContent = text("Provider ready");
    const moneyState = root.querySelector("[data-money-status]");
    if (moneyState) moneyState.textContent = state.config?.mode === "money" ? (status.money_armed ? text("Real-money execution is armed for this session.") : text("Real-money mode is disarmed. Confirm it again before starting.")) : text("Paper simulation mode.");
    const stateNode = root.querySelector("[data-status-line]");
    const statusError = status.error || state.error;
    if (stateNode) stateNode.innerHTML = `<span class="obs-status-dot ${running ? "is-running" : ""}"></span><strong>${esc(phase)}</strong>${statusError ? `<span class="obs-status-error">${esc(statusError)}</span>` : ""}`;
    const action = root.querySelector('[data-action="run"]');
    if (action) {
      action.textContent = running ? text("stop") : text(state.config?.mode === "money" ? "Start Money trading" : "Start Paper trading");
      action.dataset.run = running ? "stop" : "start";
      action.classList.toggle("is-stop", running);
      action.disabled = !running && ((busy && !running) || state.statusUnavailable || !state.loaded || !state.config?.ai_enabled || (state.config?.mode === "money" && !status.money_armed));
    }
    const hasPosition = !!status.position;
    const lock = running || busy || hasPosition;
    root.querySelectorAll(".obs-form input, .obs-form textarea, .obs-form select, .obs-form button").forEach(node => {
      if (node.type === "range" || node.dataset.config || node.dataset.frame || node.type === "checkbox" || node.dataset.action === "save" || node.dataset.action === "defaults") node.disabled = lock;
    });
    const equity = root.querySelector('[data-config="initial_equity"]');
    if (equity) equity.disabled = lock || status.decisions > 0;
    root.querySelectorAll('[data-config="risk_pct"]').forEach(node => {
      if (!node.closest('[data-strategy]')) node.closest('label').hidden = true;
    });
    root.querySelectorAll('[data-config="max_positions"]').forEach(node => {
      node.disabled = lock || state.config?.mode === "paper";
      node.title = state.config?.mode === "paper" ? "Paper: one position total" : "";
    });
    const equityLock = root.querySelector("[data-equity-lock]");
    if (equityLock) equityLock.hidden = !(status.decisions > 0);
    const notice = root.querySelector("[data-edit-notice]");
    if (notice) notice.textContent = running ? text("stopped_edit") : hasPosition ? text("position_lock") : busy ? text("busy") : state.config && !jsonEqual(state.config, state.saved) ? text("dirty") : state.notice;
    const save = root.querySelector('[data-action="save"]');
    if (save) save.disabled = lock || !state.loaded;
    const start = root.querySelector('[data-action="run"]');
    if (start) start.disabled = !running && ((busy && !running) || state.statusUnavailable || !state.loaded || !state.config?.ai_enabled || (state.config?.mode === "money" && !status.money_armed));
    root.querySelector("[data-quote]").innerHTML = `<h3>${esc(text("quote"))}</h3>${sampleMarkup(status.sample)}`;
    root.querySelector("[data-frames]").innerHTML = `<h3>${esc(text("frames_live"))}</h3>${framesMarkup(status.frames)}`;
    root.querySelector("[data-decision]").innerHTML = `<h3>${esc(text("decision"))}</h3>${decisionMarkup(status.last_decision)}`;
    root.querySelector("[data-position]").innerHTML = `<h3>${esc(text("position"))}</h3>${positionMarkup(status.position, state.config?.mode)}`;
    root.querySelector("[data-stats]").innerHTML = `<h3>${esc(text("stats"))}</h3><div class="obs-metrics">${field(text("equity_now"), `${number(status.equity, 2)} USD`)}${field(text("decisions"), number(status.decisions, 0))}${field(text("outcomes"), number(status.outcomes, 0))}</div>`;
    const account = root.querySelector("[data-account]");
    const currentAccount = window.AEGIS?.S?.account || status.account;
    if (account) account.innerHTML = `<h3>${esc(text("RoboForex account"))}</h3>${accountMarkup(currentAccount, status.live_positions)}`;
    syncChartStatus({ ...status, account: currentAccount });
    root.dataset.running = running ? "true" : "false";
  }
  function readForm(root, state) {
    const config = clone(state.config);
    root.querySelectorAll("[data-config]").forEach(node => {
      const key = node.dataset.config;
      if (["enabled", "prompt", "risk_pct", "max_positions", "max_daily_loss_pct"].includes(key)) {
        const item = config.strategies.find(s => s.id === node.closest("[data-strategy]")?.dataset.strategy);
        if (item) item[key] = node.type === "checkbox" ? node.checked : Number(node.value);
      } else if (key === "strictness") config.strictness = Number(node.value);
      else if (key === "ai_enabled") config.ai_enabled = node.checked;
      else if (key === "mode") config.mode = node.value;
      else if (node.value !== "") config[key] = Number(node.value);
    });
    root.querySelectorAll("[data-frame]").forEach(node => {
      const item = config.strategies.find(s => s.id === node.closest("[data-strategy]")?.dataset.strategy);
      if (!item) return;
      const set = new Set(item.timeframes);
      if (node.checked) set.add(node.dataset.frame); else set.delete(node.dataset.frame);
      item.timeframes = FRAMES.map(([id]) => id).filter(id => set.has(id));
      try {
        const spa=JSON.parse(item.prompt);
        spa.timeframes=[...item.timeframes];
        item.prompt=JSON.stringify(spa);
      } catch { /* Invalid SPA is rejected before saving. */ }
    });
    state.config = config;
      const out = root.querySelector("[data-strict-value]");
    if (out) out.value = String(config.strictness);
    root.querySelectorAll('[data-strategy] input[type="range"] + output').forEach((output) => {
      const input = output.previousElementSibling;
      output.value = input.dataset.config === "risk_pct" ? `${Number(input.value).toFixed(2)}%` : input.value;
      output.textContent = output.value;
    });
    const notice = root.querySelector("[data-edit-notice]");
    if (notice) notice.textContent = !jsonEqual(state.config, state.saved) ? text("dirty") : "";
  }
  function validateConfig(config) {
    if (config.ai_enabled && !config.strategies.some(strategy => strategy.enabled)) return text("need_strategy");
    for (const strategy of config.strategies) {
      if (!strategy.prompt.trim() || strategy.prompt.length > 10000) return text("prompt_invalid");
      if (!strategy.timeframes.length) return text("timeframe_invalid");
      if (!(strategy.risk_pct > 0 && strategy.risk_pct <= 1) || strategy.max_positions < 1 || strategy.max_positions > 5 || !(strategy.max_daily_loss_pct >= 0.1 && strategy.max_daily_loss_pct <= 10)) return text("config_invalid");
    }
    if (!(config.risk_pct > 0 && config.risk_pct <= 1) || !(config.initial_equity > 0) || !(config.max_spread > 0) || config.commission_per_oz < 0 || config.slippage < 0) return text("config_invalid");
    return "";
  }
  async function saveConfig(root, state) {
    const formError = root.querySelector("[data-form-error]");
    formError.textContent = "";
    const invalid = validateConfig(state.config);
    if (invalid) { formError.textContent = invalid; throw new Error(invalid); }
    if (state.config.mode === "money" && !root.querySelector("[data-money-confirm]")?.checked) {
      const message = text("money_consent_required");
      formError.textContent = message;
      throw new Error(message);
    }
    const form = root.querySelector(".obs-form");
    if (form && !form.reportValidity()) throw new Error("Invalid configuration fields");
    const wasBusy = state.busy;
    state.busy = true;
    updateStatus(root, state);
    try {
      const config = await invoke("observer_save", { config: state.config });
      state.config = cfgFrom(config);
      window.AEGIS.S.aiEnabled=state.config.ai_enabled;
      window.AEGIS.refreshStrategies?.();
      state.saved = clone(state.config);
      if (state.config.mode === "money") state.status = await invoke("observer_arm_money", { confirmed: true });
      else state.status = await invoke("observer_arm_money", { confirmed: false }).catch(() => state.status);
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
      state.statusUnavailable = false;
      state.error = "";
    } catch (error) {
      state.statusUnavailable = true;
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
      if (action === "edit-strategy") return window.AEGIS?.openPanel?.("observer_strategy", button.dataset.strategyId);
      if (action === "provider-save") {
        const key = root.querySelector("[data-provider-key]").value.trim();
        const model = root.querySelector("[data-provider-model]").value;
        const message = root.querySelector("[data-provider-message]");
        button.disabled = true;
        try {
          const result = await invoke("observer_provider_save", { key, model });
          root.querySelector("[data-provider-key]").value = "";
          message.textContent = result?.provider_key_stored ? text("Provider key and model saved.") : text("Provider settings updated.");
          root.querySelector("[data-provider-state]").textContent = result?.provider_key_stored ? text("Provider key stored") : text("Provider not configured");
        } catch (error) { message.textContent = String(error?.message || error); }
        finally { button.disabled = false; }
        return;
      }
      if (action === "diagnostics") {
        const output = root.querySelector("[data-diagnostics]");
        button.disabled = true;
        output.textContent = text("loading");
        try {
          if (!confirmAction(root,button,text("Full readiness checks send a billable AI provider request but never place an order. Continue?"))) return;
          const rows = await invoke("observer_check_cycle");
          output.innerHTML = Array.isArray(rows) ? rows.map((row) => `<div class="obs-diagnostic"><strong>${esc(row.name || text("unknown"))}</strong><span>${row.ok ? "✓" : "!"} ${esc(row.detail || "")}${row.latency_ms == null ? "" : ` · ${esc(row.latency_ms)} ms`}</span></div>`).join("") : esc(JSON.stringify(rows));
        } catch (error) { output.textContent = String(error?.message || error); }
        finally { button.disabled = false; }
        return;
      }
      if (action === "provider-test") {
        const output = root.querySelector("[data-provider-message]");
        button.disabled = true;
        output.textContent = text("loading");
        try {
          if (!confirmAction(root,button,text("This will send a billable OpenRouter API request. Continue?"))) return;
          const result = await invoke("observer_provider_test");
          output.textContent = `${result.ok ? "✓" : "!"} ${result.detail || ""}${result.latency_ms == null ? "" : ` · ${result.latency_ms} ms`}`;
        } catch (error) { output.textContent = String(error?.message || error); }
        finally { button.disabled = false; }
        return;
      }
      if (action === "close-position" || action === "close-all") {
        if (closingPosition) return;
        closingPosition=true;
        button.disabled = true;
        try {
          const result = action === "close-position" ? await invoke("observer_close", { ticket: Number(button.dataset.ticket) }) : await invoke("observer_close_all");
          const results = Array.isArray(result) ? result : [result];
          const failures = results.filter(row => row.status && row.status !== "filled");
          if (failures.length) throw new Error(failures.map(row => `${row.status}: ${row.message}`).join("; "));
          await refreshStatus(root, state);
        } catch (error) {
          const node = root.querySelector("[data-action-error]");
          if (node) node.textContent = String(error?.message || error);
        } finally { closingPosition=false;button.disabled = false; }
        return;
      }
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
      window.AEGIS.S.aiEnabled=config.ai_enabled;
      window.AEGIS.refreshStrategies?.();
      state.saved = clone(config);
      state.status = info.status || null;
      state.loaded = true;
      root.querySelector("[data-config-form]").innerHTML = configMarkup(config);
      root.querySelector("[data-preflight]").textContent = text("preflight");
      const provider = root.querySelector("[data-provider-state]");
      if (provider) provider.textContent = info.provider_key_stored ? text("Provider key stored") : text("Provider not configured");
      updateStatus(root, state);
      await refreshStatus(root, state);
    } catch (error) {
      if (!root.isConnected || token !== state.loadToken) return;
      state.error = String(error?.message || error);
      root.querySelector("[data-status-line]").innerHTML = `<strong>${esc(text("error"))}</strong><span class="obs-status-error">${esc(state.error)}</span>`;
      root.querySelector("[data-config-form]").innerHTML = `<p class="obs-status-error" role="alert">${esc(state.error)}</p>`;
    }
  }
  function renderSettings(container, focus) {
    if (!container) return;
    let root = container.querySelector(":scope > .obs-root");
    let state = root && STATES.get(root);
    if (!root) {
      root = document.createElement("div");
      root.className = "obs-root";
      root.dataset.testid = "observer-settings";
      container.replaceChildren(root);
      state = makeState();
      STATES.set(root, state);
      root.innerHTML = `<div class="obs-heading"><div><span class="obs-kicker">AI SETTINGS</span><h2>${esc(text("AI settings"))}</h2><p>${esc(text("intro"))}</p></div><button class="ghost sm" type="button" data-action="refresh">${esc(text("refresh"))}</button></div>
        <div class="obs-safety" role="note">${esc(text("no_orders"))}</div><p class="obs-help obs-stop-note">${esc(text("stop_protect"))}</p><p class="obs-preflight" data-preflight>${esc(text("preflight"))}</p><p class="obs-help" data-money-status>${esc(text("Paper simulation mode."))}</p>
        <section class="obs-card obs-status-card"><div class="obs-status-line" data-status-line role="status" aria-live="polite"><span class="obs-status-dot"></span><strong>${esc(text("loading"))}</strong></div><div class="obs-actions obs-run-actions"><button class="cta" type="button" data-action="run" data-run="start" disabled>${esc(text("start"))}</button><button class="ghost" type="button" data-action="journal">${esc(text("journal"))}</button><span class="obs-inline-error" data-action-error role="alert"></span></div></section>
        <div data-config-form><p class="obs-muted">${esc(text("loading"))}</p></div>
        <section class="obs-card obs-provider"><h3>${esc(text("OpenRouter provider"))}</h3><p class="obs-help">${esc(text("No spend limits are configured here. Provider charges depend on your OpenRouter account."))}</p><label class="obs-label">${esc(text("API key"))}<input type="password" autocomplete="new-password" data-provider-key placeholder="${esc(text("Leave empty to keep the stored key"))}"></label><label class="obs-label">${esc(text("Model"))}<select data-provider-model><option value="anthropic/claude-fable-5">Fable5</option></select></label><div class="obs-actions"><button type="button" class="ghost" data-action="provider-save">${esc(text("Save provider"))}</button><button type="button" class="ghost" data-action="provider-test">${esc(text("Test provider"))}</button><span data-provider-state>${esc(text("Provider status will appear here."))}</span></div><div class="obs-inline-error" data-provider-message role="status"></div></section>
        <section class="obs-card"><div class="obs-section-head"><h3>${esc(text("Diagnostics"))}</h3><button type="button" class="ghost sm" data-action="diagnostics">${esc(text("Run readiness checks"))}</button></div><p class="obs-help">${esc(text("Checks MT5 and model readiness without placing an order. Includes a billable provider API ping; confirmation is required."))}</p><div data-diagnostics></div></section>
        <section class="obs-card" data-account><h3>${esc(text("RoboForex account"))}</h3></section>
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
        if (!root.isConnected || (window.AEGIS?.S?.panel && window.AEGIS.S.panel !== "observer" && !(window.AEGIS.S.panel === "theme" && window.AEGIS.S.settingsTab === "ai"))) {
          clearInterval(state.timer); state.timer = null; return;
        }
        refreshStatus(root, state);
      }, 1000);
    }
  }
  function renderStrategy(container, id) {
    if (!container) return;
    const strategyId = id === "bounce" ? "density_bounce" : id;
    const def = STRATEGIES.find((item) => item.id === strategyId);
    if (!def) return;
    container.innerHTML = `<div class="obs-root obs-strategy-editor" data-testid="strategy-spa"><div class="obs-heading"><div><span class="obs-kicker">SPA</span><h2>${esc(def[lang()] || def.en)}</h2><p>${esc(text("SPA — System Prompt Algorithm"))}</p></div></div><div class="obs-safety">${esc(text("Editing first cancels pending analysis and pauses new entries. Existing positions keep their protection."))}</div><label class="obs-label">${esc(text("SPA — System Prompt Algorithm"))}<textarea rows="14" data-spa-input disabled spellcheck="false"></textarea></label><div class="obs-actions"><button type="button" class="ghost" data-spa-edit>${esc(text("Edit SPA"))}</button><button type="button" class="cta" data-spa-save disabled>${esc(text("Save SPA"))}</button><span data-spa-status role="status"></span></div></div>`;
    const editor = container.querySelector("[data-spa-input]");
    const status = container.querySelector("[data-spa-status]");
    const buttons = container.querySelectorAll("[data-spa-edit], [data-spa-save]");
    let config;
    let current;
    const setError = (value) => { status.textContent = value; };
    invoke("observer_info").then(async (info) => {
      config = cfgFrom(info.config);
      current = config.strategies.find((item) => item.id === strategyId);
      editor.value = current.prompt;
      const edit = container.querySelector("[data-spa-edit]");
      const save = container.querySelector("[data-spa-save]");
      edit.onclick = async () => {
        try {
          const running = info.status?.running || (await invoke("observer_status")).running;
          if (running) {
            await invoke("observer_stop");
            info.status.running=false;
            setError(text("Observation has been stopped. Existing positions remain protected. Click Edit SPA again to continue."));
            return;
          }
          editor.disabled = false;
          save.disabled = false;
          edit.disabled = true;
          editor.focus();
          setError("");
        } catch (error) { setError(String(error?.message || error)); }
      };
      save.onclick = async () => {
        try {
          const spa = JSON.parse(editor.value);
          const keys = Object.keys(spa).sort().join(",");
          if (keys !== "entry_rules,exit_rules,objective,timeframes" || typeof spa.objective !== "string" || !Array.isArray(spa.entry_rules) || !Array.isArray(spa.exit_rules) || !Array.isArray(spa.timeframes)) throw new Error(text("SPA must be a JSON object with objective, entry_rules, exit_rules and timeframes."));
          current.prompt = JSON.stringify(spa);
          if (spa.timeframes.length) current.timeframes = [...spa.timeframes];
          config.strategies = config.strategies.map((item) => item.id === strategyId ? current : item);
          await invoke("observer_save", { config });
          editor.disabled = true;
          save.disabled = true;
          edit.disabled = false;
          info = await invoke("observer_info");
          setError(config.mode === "money" ? text("SPA saved. Money mode is disarmed; confirm it again in Settings → AI.") : text("SPA saved."));
        } catch (error) { setError(String(error?.message || error)); }
      };
    }).catch((error) => {
      editor.value = "";
      setError(String(error?.message || error));
      buttons.forEach((button) => { button.disabled = true; });
    });
  }
  function render(container, focus) { return renderSettings(container, focus); }
  window.AEGIS = window.AEGIS || {};
  window.AEGIS.observer = { render, renderSettings, renderStrategy, syncChartStatus, isEnabled: async () => { try { return !!(await invoke("observer_info")).config?.ai_enabled; } catch { return false; } } };
})();
