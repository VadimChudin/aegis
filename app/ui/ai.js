"use strict";
(() => {
  const A = window.AEGIS;
  if (!A) return;
  const { invoke, esc, S } = A;
  const fields = [
    { id: "local_url", label: "Адрес локальной модели", type: "url", placeholder: "http://127.0.0.1:11434" },
    { id: "local_model", label: "Локальная модель", placeholder: "qwen3:8b" },
    { id: "cloud_model", label: "Облачная модель", placeholder: "Название модели провайдера" },
    { id: "strategy", label: "Стратегия", placeholder: "Название стратегии" },
    { id: "initial_equity", label: "Фиксированный начальный капитал Paper, USD", type: "number", min: "10000", max: "10000", step: "1", fallback: 10000, readonly: true },
    { id: "max_risk_pct", label: "Совокупный риск Paper, %", type: "number", min: "0", max: "100", step: "any" },
    { id: "max_leverage", label: "Максимальное кредитное плечо", type: "number", min: "1", step: "any", fallback: 1 },
    { id: "max_positions", label: "Максимум открытых позиций", type: "number", min: "1", step: "1" },
    { id: "daily_loss_limit", label: "Дневной лимит убытка, USD", type: "number", min: "0", step: "any" },
    { id: "interval_seconds", label: "Интервал проверки, секунд", type: "number", min: "1", step: "1" },
    { id: "api_daily_budget_usd", label: "Дневной лимит API, USD", type: "number", min: "0", step: "any" },
    { id: "api_monthly_budget_usd", label: "Месячный лимит API, USD", type: "number", min: "0", step: "any" },
    { id: "max_cloud_requests_per_day", label: "Максимум облачных запросов в день", type: "number", min: "0", step: "1" },
  ];
  const B = { status: null, loaded: false, loading: false, busy: false, notice: "", error: "" };
  let pollTimer = null;

  const $ = (root, id) => root.querySelector(`#${id}`);
  const val = (v, digits = 2) => Number.isFinite(Number(v)) ? Number(v).toLocaleString("ru-RU", { maximumFractionDigits: digits }) : "—";
  const stringify = (v) => {
    if (typeof v === "string") return v;
    try { return JSON.stringify(v, null, 2); } catch { return String(v); }
  };
  const connectedBrokers = () => Object.keys(S.sessions || {});
  function preferredBroker() {
    const connected = connectedBrokers();
    if (connected.includes(S.chart?.broker)) return S.chart.broker;
    return connected[0] || "";
  }
  function errorText(e) {
    if (e && typeof e === "object" && "message" in e) return String(e.message);
    return String(e ?? "Неизвестная ошибка");
  }
  function setupLabel(status) {
    return ({
      checking_runtime: "Проверка Ollama",
      downloading_runtime: "Загрузка Ollama",
      starting_runtime: "Запуск Ollama",
      preparing_model: "Подготовка qwen3:8b",
      downloading_model: "Загрузка qwen3:8b",
      ready: "Готово",
      error: "Ошибка установки",
    })[status] || status || "не запущена";
  }
  function fieldMarkup(f, settings) {
    const id = `ai-${f.id}`;
    const type = f.type || "text";
    const value = f.id === "initial_equity" ? "10000" : settings?.[f.id] == null ? (f.fallback == null ? "" : String(f.fallback)) : String(settings[f.id]);
    const attrs = type === "number" ? `min="${f.min}" ${f.max ? `max="${f.max}"` : ""} step="${f.step}" required ${f.readonly ? "readonly" : ""}` : `${f.type === "url" ? "inputmode=\"url\"" : ""}`;
    return `<label class="ai-field" for="${id}"><span>${esc(f.label)}</span><input id="${id}" name="${f.id}" type="${type}" value="${esc(value)}" placeholder="${esc(f.placeholder || "")}" ${attrs} autocomplete="off" data-testid="ai-setting-${f.id}"></label>`;
  }
  function settingsMarkup(settings) {
    return `<form id="ai-settings-form" class="ai-settings-form">
      <div class="ai-form-grid">${fields.slice(0, 4).map(f => fieldMarkup(f, settings)).join("")}</div>
      <label class="ai-field ai-field-wide" for="ai-preprompt"><span>Дополнительная инструкция (необязательно)</span><textarea id="ai-preprompt" name="preprompt" rows="3" placeholder="Свои ограничения или контекст для модели">${esc(settings?.preprompt ?? "")}</textarea></label>
      <div class="ai-form-grid">${fields.slice(4).map(f => fieldMarkup(f, settings)).join("")}</div>
      <div class="ai-key-row"><label class="ai-field" for="ai-api-key"><span>Ключ облачного API</span><input id="ai-api-key" name="apiKey" type="password" autocomplete="new-password" placeholder="${B.status?.key_stored ? "Ключ сохранён · пустое поле оставит его без изменений" : "Необязательно; хранится приложением"}"></label>
        <button class="ghost sm" id="ai-forget-key" type="button" ${B.status?.key_stored ? "" : "disabled"}>Удалить сохранённый ключ</button></div>
      <div class="ai-form-actions"><button class="cta" type="submit" id="ai-save">Сохранить настройки</button><span class="ai-inline-error" id="ai-save-error" role="alert"></span></div>
    </form>`;
  }
  function brokerSelect() {
    const ids = connectedBrokers();
    const selected = preferredBroker();
    const options = ids.map(id => {
      const session = S.sessions[id];
      const label = session?.account ? `${id} · ${session.account}` : id;
      return `<option value="${esc(id)}" ${id === selected ? "selected" : ""}>${esc(label)}</option>`;
    }).join("");
    return `<label class="ai-broker"><span>Источник котировок / Paper-брокер</span><select id="ai-broker" ${ids.length ? "" : "disabled"}>${ids.length ? options : `<option value="">Нет подключённых брокеров</option>`}</select></label>`;
  }
  function scaffold(root) {
    root.innerHTML = `<div class="ai-panel" data-testid="ai-panel">
      <section class="ai-card ai-live-card" aria-labelledby="ai-live-title"><div class="ai-section-head"><div><span class="ai-kicker">AI · PAPER</span><h3 id="ai-live-title">Состояние и результаты</h3></div><button type="button" class="ghost sm" id="ai-refresh">Обновить</button></div>
        <div id="ai-status" class="ai-status" role="status" aria-live="polite"><p class="hint">Загрузка состояния…</p></div>
        <div id="ai-operation-result" class="ai-operation-result" role="status" hidden></div>
      </section>
      <section class="ai-card"><div class="ai-section-head"><div><span class="ai-kicker">УПРАВЛЕНИЕ</span><h3>Бумажная торговля</h3></div></div>
        ${brokerSelect()}<div class="ai-actions"><button type="button" class="cta" id="ai-start" disabled>Запустить Paper</button><button type="button" class="ghost" id="ai-stop" disabled>Остановить</button><button type="button" class="ghost" id="ai-step" disabled>Один шаг</button></div>
        <p class="ai-disclaimer">Только симуляция: реальные ордера не отправляются. На Linux MetaTrader 5 (MT5) не поддерживается; подключите доступный здесь брокер. Результаты Paper не равны исполнению или доходности реального счёта.</p>
      </section>
      <section class="ai-card"><div class="ai-section-head"><div><span class="ai-kicker">ПОДКЛЮЧЕНИЕ МОДЕЛИ</span><h3>Локальная модель</h3></div></div>
        <p class="ai-copy">Установка управляемого Ollama и загрузка qwen3:8b начинаются только после явного нажатия. Модель занимает около 5 ГБ; вместе с Ollama потребуется примерно 6,5 ГБ. Перед установкой освободите более 10 ГБ на диске и убедитесь, что хватает памяти для запуска.</p>
        <div class="ai-setup-row"><button type="button" class="ghost" id="ai-setup">Установить / загрузить qwen3:8b</button><div id="ai-setup-progress" class="ai-setup-progress" role="status">Статус установки появится здесь.</div></div>
      </section>
      <section class="ai-card"><div class="ai-section-head"><div><span class="ai-kicker">КОНФИГУРАЦИЯ</span><h3>Настройки и лимиты</h3></div></div>
        <p class="ai-copy">Лимиты API — защитные настройки. Сумма «Учтено/зарезервировано» включает подтверждённую стоимость и консервативный резерв по тарифам; если провайдер не сообщает стоимость, резерв не пересчитывается автоматически. Проверка ключа OpenRouter использует GET и не запускает платный запрос модели.</p>
        <div id="ai-settings">${B.loaded ? settingsMarkup(B.status?.settings || {}) : `<p class="hint">Загрузка настроек…</p>`}</div>
      </section>
    </div>`;
    if (root.dataset.aiBound !== "1") {
      root.dataset.aiBound = "1";
      root.addEventListener("click", (event) => handleClick(root, event));
      root.addEventListener("submit", (event) => { if (event.target.id === "ai-settings-form") { event.preventDefault(); saveSettings(root); } });
    }
  }
  function setError(root, message) {
    B.error = message;
    const el = $(root, "ai-operation-result");
    if (!el) return;
    el.hidden = !message;
    el.className = "ai-operation-result is-error";
    el.textContent = message;
  }
  function metric(label, value) {
    return `<div class="ai-metric"><span>${esc(label)}</span><strong>${esc(String(value))}</strong></div>`;
  }
  function journalMarkup(rows) {
    if (!Array.isArray(rows) || rows.length === 0) return `<p class="ai-empty">Записей пока нет.</p>`;
    const recent = rows.slice(-100).reverse();
    return `<div class="ai-table-wrap"><table class="ai-table"><thead><tr><th>Время</th><th>Событие</th><th>Результат / причина</th></tr></thead><tbody>${recent.map(row => {
      const item = row && typeof row === "object" ? row : { message: row };
      const time = item.time ?? item.timestamp ?? item.created_at ?? "—";
      const event = item.action ?? item.event ?? item.status ?? item.side ?? "Запись журнала";
      const detail = item.reason ?? item.message ?? item.result ?? item.realized ?? item.pnl ?? item.profit ?? stringify(item);
      return `<tr><td>${esc(time)}</td><td>${esc(event)}</td><td class="ai-reason">${esc(typeof detail === "object" ? stringify(detail) : detail)}</td></tr>`;
    }).join("")}</tbody></table></div>`;
  }
  function renderStatus(root) {
    const statusEl = $(root, "ai-status");
    if (!statusEl || !B.status) return;
    const s = B.status;
    const p = s.paper || {};
    const running = !!s.running;
    const busy = !!s.busy || B.busy;
    const setup = s.setup || {};
    const progress = Math.max(0, Math.min(100, Number(setup.progress) || 0));
    const statusLabel = s.message || (running ? "Paper запущен" : "Paper остановлен");
    const positions = Array.isArray(p.positions) ? p.positions : [];
    const selectedBroker = $(root, "ai-broker")?.value || "";
    const positionsHtml = positions.length ? `<div class="ai-table-wrap"><table class="ai-table"><thead><tr><th>ID</th><th>Сторона</th><th>Количество</th><th>Вход</th><th>Текущая цена</th><th>Стоп</th><th></th></tr></thead><tbody>${positions.map(x => {
      const positionBroker = String(x.broker || "").toLowerCase();
      const closeEnabled = !!selectedBroker && positionBroker === selectedBroker.toLowerCase();
      return `<tr><td>${esc(x.id ?? "—")}</td><td>${esc(x.side ?? "—")}</td><td>${esc(val(x.quantity, 8))}</td><td>${esc(val(x.entry_price, 8))}</td><td>${esc(val(x.mark_price, 8))}</td><td>${esc(val(x.stop, 8))}</td><td><button type="button" class="ghost sm ai-close-position" data-ai-close data-position-id="${esc(x.id)}" ${closeEnabled ? "" : "disabled"} aria-label="Закрыть Paper-позицию ${esc(x.id)}" data-testid="ai-close-position-${esc(x.id)}">Закрыть</button></td></tr>`;
    }).join("")}</tbody></table></div>` : `<p class="ai-empty">Открытых позиций нет.</p>`;
    statusEl.innerHTML = `<div class="ai-state-line"><span class="ai-state-dot ${running ? "is-running" : ""}"></span><strong>${esc(running ? "Paper запущен" : "Paper остановлен")}</strong><span class="ai-state-message">${esc(statusLabel)}</span></div>
      <div class="ai-metrics">${metric("Капитал Paper, USD", p.equity == null ? "—" : val(p.equity))}${metric("Реализованный результат, USD", p.realized == null ? "—" : val(p.realized))}${metric("Открытые позиции", positions.length)}${metric("Ключ облака", s.key_stored ? "Сохранён" : "Не задан")}${metric("Учтено/зарезервировано за день, USD", s.costs?.day_spent == null ? "—" : val(s.costs.day_spent, 4))}${metric("Учтено/зарезервировано за месяц, USD", s.costs?.month_spent == null ? "—" : val(s.costs.month_spent, 4))}${metric("Запросы сегодня", s.costs?.requests_today == null ? "—" : val(s.costs.requests_today, 0))}</div>
      <div class="ai-subsection"><h4>Открытые позиции</h4>${positionsHtml}</div><div class="ai-subsection"><h4>Журнал Paper</h4>${journalMarkup(p.journal)}</div>
      <div class="ai-setup-status"><span>Установка модели · ${esc(setupLabel(setup.status))}</span><progress max="100" value="${progress}"></progress><span>${progress}%</span></div>`;
    const broker = $(root, "ai-broker")?.value;
    const hasBroker = !!broker && connectedBrokers().includes(broker);
    const start = $(root, "ai-start");
    const stop = $(root, "ai-stop");
    const step = $(root, "ai-step");
    const statusHasError = /error|ошиб|поврежд|некорректн|заблокирован/i.test(s.message || "");
    if (start) start.disabled = running || busy || !hasBroker || statusHasError || !!B.error;
    if (stop) stop.disabled = !running && !busy;
    if (step) step.disabled = running || busy || !hasBroker;
    const setupButton = $(root, "ai-setup");
    const setupBusy = ["checking_runtime", "downloading_runtime", "starting_runtime", "preparing_model", "downloading_model"].includes(setup.status);
    if (setupButton) setupButton.disabled = busy || setupBusy;
    const keyInput = $(root, "ai-api-key");
    if (keyInput && !keyInput.placeholder.includes("·")) keyInput.placeholder = s.key_stored ? "Ключ сохранён · пустое поле оставит его без изменений" : "Необязательно; хранится приложением";
    const forget = $(root, "ai-forget-key");
    if (forget) forget.disabled = busy || !s.key_stored;
    const setupEl = $(root, "ai-setup-progress");
    if (setupEl) setupEl.textContent = `${setupLabel(setup.status)}${setup.progress == null ? "" : ` · ${progress}%`}`;
  }
  async function refresh(root) {
    if (B.loading || S.panel !== "ai" || !root.isConnected) return;
    B.loading = true;
    try {
      const status = await invoke("ai_status");
      if (S.panel !== "ai" || !root.isConnected) return;
      B.status = status || {};
      B.error = "";
      if (!B.loaded) {
        B.loaded = true;
        const settingsEl = $(root, "ai-settings");
        if (settingsEl) settingsEl.innerHTML = settingsMarkup(B.status.settings || {});
        ensureTestButtons(root);
      }
      renderStatus(root);
    } catch (e) {
      if (S.panel === "ai" && root.isConnected) {
        const statusEl = $(root, "ai-status");
        if (statusEl) statusEl.innerHTML = `<p class="ai-inline-error" role="alert">Не удалось получить состояние AI: ${esc(errorText(e))}</p>`;
        const settingsEl = $(root, "ai-settings");
        if (!B.loaded && settingsEl) settingsEl.innerHTML = `<p class="ai-inline-error" role="alert">Настройки пока недоступны. Нажмите «Обновить» после устранения ошибки.</p>`;
      }
    } finally {
      B.loading = false;
    }
  }
  function showResult(root, message, error = false) {
    const el = $(root, "ai-operation-result");
    if (!el) return;
    el.hidden = !message;
    el.className = `ai-operation-result${error ? " is-error" : ""}`;
    el.textContent = message;
  }
  async function action(root, label, command, args) {
    if (B.busy) return;
    B.busy = true;
    B.notice = `${label}…`;
    showResult(root, B.notice);
    renderStatus(root);
    try {
      await invoke(command, args);
      showResult(root, `${label}: готово.`);
      await refresh(root);
    } catch (e) {
      const message = `${label}: ${errorText(e)}`;
      showResult(root, message, true);
      setError(root, message);
    } finally {
      B.busy = false;
      renderStatus(root);
    }
  }
  async function saveSettings(root) {
    const form = $(root, "ai-settings-form");
    if (!form || !form.reportValidity()) return;
    const settings = {};
    for (const f of fields) {
      const input = form.elements.namedItem(f.id);
      settings[f.id] = f.type === "number" ? Number(input.value) : input.value.trim();
    }
    settings.preprompt = form.elements.namedItem("preprompt").value.trim();
    const keyInput = $(root, "ai-api-key");
    const apiKey = keyInput.value.trim() || null;
    const errorEl = $(root, "ai-save-error");
    const save = $(root, "ai-save");
    errorEl.textContent = "";
    save.disabled = true;
    try {
      await invoke("ai_save", { settings, apiKey });
      keyInput.value = "";
      B.loaded = false;
      showResult(root, "Настройки сохранены.");
      await refresh(root);
    } catch (e) {
      errorEl.textContent = `Не удалось сохранить настройки: ${errorText(e)}`;
    } finally {
      save.disabled = false;
    }
  }
  async function forgetKey(root) {
    if (B.busy) return;
    const button = $(root, "ai-forget-key");
    button.disabled = true;
    try {
      await invoke("ai_forget_key");
      showResult(root, "Сохранённый облачный ключ удалён.");
      await refresh(root);
    } catch (e) {
      showResult(root, `Не удалось удалить ключ: ${errorText(e)}`, true);
      button.disabled = false;
    }
  }
  function handleClick(root, event) {
    const closeButton = event.target.closest("[data-ai-close]");
    if (closeButton) {
      const broker = $(root, "ai-broker")?.value;
      const positionId = Number(closeButton.dataset.positionId);
      if (!broker || !connectedBrokers().includes(broker) || !Number.isSafeInteger(positionId) || positionId < 0) {
        showResult(root, "Выберите подключённый брокер этой Paper-позиции для закрытия.", true);
        return;
      }
      closeButton.disabled = true;
      showResult(root, `Закрытие Paper-позиции ${positionId}…`);
      invoke("ai_close", { broker, positionId }).then(() => {
        showResult(root, `Paper-позиция ${positionId} закрыта вручную; выполнявшееся решение отменено.`);
        refresh(root);
      }).catch(e => {
        showResult(root, `Не удалось закрыть Paper-позицию ${positionId}: ${errorText(e)}`, true);
        refresh(root);
      });
      return;
    }
    const id = event.target.closest("button")?.id;
    if (!id) return;
    if (id === "ai-refresh") { refresh(root); return; }
    if (id === "ai-forget-key") { forgetKey(root); return; }
    if (id === "ai-setup") { action(root, "Установка модели", "ai_setup_model"); return; }
    if (id === "ai-stop") {
      showResult(root, "Остановка Paper…");
      invoke("ai_stop").then(() => {
        showResult(root, "Paper остановлен. Текущий AI-запрос отменён; защитные Paper-стопы продолжают действовать, пока приложение открыто.");
        refresh(root);
      }).catch(e => showResult(root, `Не удалось остановить Paper: ${errorText(e)}`, true));
      return;
    }
    if (id === "ai-start" || id === "ai-step") {
      const broker = $(root, "ai-broker")?.value;
      if (!broker || !connectedBrokers().includes(broker)) {
        showResult(root, "Сначала подключите брокер с котировками и выберите его для Paper.", true);
        return;
      }
      action(root, id === "ai-start" ? "Запуск Paper" : "Шаг Paper", id === "ai-start" ? "ai_start" : "ai_step", { broker });
      return;
    }
    if (id === "ai-test-local" || id === "ai-test-cloud") {
      const cloud = id === "ai-test-cloud";
      action(root, cloud ? "Проверка облачной модели" : "Проверка локальной модели", "ai_test", { cloud });
    }
  }
  function ensureTestButtons(root) {
    const actions = $(root, "ai-settings")?.querySelector(".ai-form-actions");
    if (actions && !$(root, "ai-test-local")) {
      actions.insertAdjacentHTML("beforebegin", `<div class="ai-test-actions"><button type="button" class="ghost" id="ai-test-local">Проверить локальную модель</button><button type="button" class="ghost" id="ai-test-cloud">Проверить облачную модель</button></div>`);
    }
  }
  function render(container) {
    const root = container || document.getElementById("panelBody");
    if (!root) return;
    if (!root.querySelector("[data-testid='ai-panel']")) scaffold(root);
    ensureTestButtons(root);
    refresh(root);
    if (!pollTimer) pollTimer = setInterval(() => {
      if (S.panel === "ai" && root.isConnected) refresh(root);
    }, 2000);
  }
  window.AEGIS.ai = { render };
})();
