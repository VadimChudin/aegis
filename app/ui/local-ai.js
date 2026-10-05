"use strict";

(() => {
  const A = window.AEGIS;
  if (!A) return;
  const { invoke, esc, log, S } = A;

  const DICT = {
    ru: {
      "Local AI": "Локальный ИИ",
      "Private, on-device model tools. Live trading is not part of this step.": "Приватные инструменты модели на вашем устройстве. Живая торговля не входит в этот этап.",
      "Model status": "Статус модели",
      "Checking local model…": "Проверяем локальную модель…",
      "Retry status": "Повторить проверку",
      "Model": "Модель",
      "Endpoint": "Адрес сервера",
      "Version": "Версия",
      "Server": "Сервер",
      "Model files": "Файлы модели",
      "Stored experiences": "Сохранено уроков",
      "Data directory": "Папка данных",
      "Loaded on": "Вычисления",
      "Not installed": "Не установлена",
      "Installed": "Установлена",
      "Downloaded": "Загружена",
      "Not downloaded": "Не загружена",
      "Ready": "Готова",
      "Stopped": "Остановлен",
      "Starting": "Запускается",
      "GPU": "GPU",
      "CPU": "CPU",
      "Not loaded": "Не загружена в память",
      "Install locally": "Установить локально",
      "Start server": "Запустить сервер",
      "Stop server": "Остановить сервер",
      "Ping model": "Проверить модель",
      "Before installation": "Перед установкой",
      "The model weights are about 5.2 GB, plus about 1.5 GB for the runtime. Several GB may be needed after extraction. The server listens on loopback only. No drivers are installed automatically; required drivers must already be present.": "Веса модели занимают около 5,2 ГБ, а среда выполнения — около 1,5 ГБ. После распаковки может потребоваться еще несколько гигабайт. Сервер доступен только через loopback. Драйверы автоматически не устанавливаются — необходимые драйверы должны быть уже установлены.",
      "I understand and approve downloading and installing the local model and runtime on this computer.": "Я понимаю и разрешаю загрузку и установку локальной модели и среды выполнения на этом компьютере.",
      "Ask a question": "Задать вопрос",
      "Ask a normal question to the local model. This is a model test only: no trading action and no external API request.": "Задайте обычный вопрос локальной модели. Это только проверка модели: никаких торговых действий и внешних API-запросов.",
      "Your question": "Ваш вопрос",
      "Ask model": "Спросить модель",
      "Enter a question first.": "Сначала введите вопрос.",
      "Last answer": "Последний ответ",
      "Memories used": "Использовано воспоминаний",
      "Experience memory": "Память об опыте",
      "This stores explicit lessons as local experience records; it does not fine-tune model weights. Lessons are entered by you, not generated or saved from assistant answers.": "Здесь хранятся явно внесенные заметки об опыте; веса модели не дообучаются. Уроки вводите вы — они не генерируются и не сохраняются из ответов ассистента.",
      "Strategy": "Стратегия",
      "Observation": "Наблюдение",
      "Lesson": "Урок",
      "Outcome (R, optional)": "Результат (R, необязательно)",
      "Add lesson": "Добавить урок",
      "Saved lessons": "Сохраненные уроки",
      "No lessons saved yet.": "Пока нет сохраненных уроков.",
      "Delete": "Удалить",
      "Export JSONL": "Экспортировать JSONL",
      "Export downloads a local file; nothing is transmitted.": "Экспорт загружает файл на это устройство; данные никуда не отправляются.",
      "Enter both an observation and a lesson.": "Введите и наблюдение, и урок.",
      "Enter a valid R value or leave it blank.": "Введите корректное значение R или оставьте поле пустым.",
      "The local model is not ready yet.": "Локальная модель еще не готова.",
      "Progress": "Ход выполнения",
      "Unknown size": "Размер неизвестен",
      "Elapsed": "Время",
      "Installing": "Установка",
      "Starting server": "Запуск сервера",
      "Stopping server": "Остановка сервера",
      "Pinging model": "Проверка модели",
      "Asking model": "Запрос модели",
      "Saving lesson": "Сохранение урока",
      "Deleting lesson": "Удаление урока",
      "Exporting": "Экспорт",
      "Yes": "Да",
      "No": "Нет",
      "Loopback only": "Только loopback",
      "Owned by this app": "Запущен этим приложением",
      "Server is ready": "Сервер готов",
      "No local server": "Локальный сервер не запущен",
    },
    kk: {
      "Local AI": "Жергілікті ЖИ",
      "Private, on-device model tools. Live trading is not part of this step.": "Құрылғыдағы жеке модель құралдары. Нақты сауда бұл кезеңге кірмейді.",
      "Model status": "Модель күйі",
      "Checking local model…": "Жергілікті модель тексерілуде…",
      "Retry status": "Күйді қайта тексеру",
      "Model": "Модель",
      "Endpoint": "Сервер мекенжайы",
      "Version": "Нұсқа",
      "Server": "Сервер",
      "Model files": "Модель файлдары",
      "Stored experiences": "Сақталған сабақтар",
      "Data directory": "Деректер бумасы",
      "Loaded on": "Есептеу құрылғысы",
      "Not installed": "Орнатылмаған",
      "Installed": "Орнатылған",
      "Downloaded": "Жүктелген",
      "Not downloaded": "Жүктелмеген",
      "Ready": "Дайын",
      "Stopped": "Тоқтатылған",
      "Starting": "Іске қосылуда",
      "GPU": "GPU",
      "CPU": "CPU",
      "Not loaded": "Жадқа жүктелмеген",
      "Install locally": "Жергілікті орнату",
      "Start server": "Серверді іске қосу",
      "Stop server": "Серверді тоқтату",
      "Ping model": "Модельді тексеру",
      "Before installation": "Орнату алдында",
      "The model weights are about 5.2 GB, plus about 1.5 GB for the runtime. Several GB may be needed after extraction. The server listens on loopback only. No drivers are installed automatically; required drivers must already be present.": "Модель салмақтары шамамен 5,2 ГБ, орындалу ортасы тағы шамамен 1,5 ГБ. Архивті ашқаннан кейін бірнеше ГБ орын қажет болуы мүмкін. Сервер тек loopback арқылы қолжетімді. Драйверлер автоматты орнатылмайды, қажетті драйверлер алдын ала орнатылған болуы керек.",
      "I understand and approve downloading and installing the local model and runtime on this computer.": "Осы компьютерге жергілікті модель мен орындалу ортасын жүктеп, орнатуға түсінемін және келісемін.",
      "Ask a question": "Сұрақ қою",
      "Ask a normal question to the local model. This is a model test only: no trading action and no external API request.": "Жергілікті модельге қарапайым сұрақ қойыңыз. Бұл тек модель сынағы: сауда әрекеті де, сыртқы API сұрауы да жоқ.",
      "Your question": "Сұрағыңыз",
      "Ask model": "Модельден сұрау",
      "Enter a question first.": "Алдымен сұрақ енгізіңіз.",
      "Last answer": "Соңғы жауап",
      "Memories used": "Қолданылған естеліктер",
      "Experience memory": "Тәжірибе жады",
      "This stores explicit lessons as local experience records; it does not fine-tune model weights. Lessons are entered by you, not generated or saved from assistant answers.": "Мұнда нақты сабақтар жергілікті тәжірибе жазбалары ретінде сақталады; модель салмақтары оқытылмайды. Сабақтарды сіз енгізесіз, ассистент жауаптарынан жасалмайды және сақталмайды.",
      "Strategy": "Стратегия",
      "Observation": "Бақылау",
      "Lesson": "Сабақ",
      "Outcome (R, optional)": "Нәтиже (R, міндетті емес)",
      "Add lesson": "Сабақ қосу",
      "Saved lessons": "Сақталған сабақтар",
      "No lessons saved yet.": "Әзірге сақталған сабақ жоқ.",
      "Delete": "Жою",
      "Export JSONL": "JSONL экспорттау",
      "Export downloads a local file; nothing is transmitted.": "Экспорт файлды осы құрылғыға жүктейді; ештеңе жіберілмейді.",
      "Enter both an observation and a lesson.": "Бақылау мен сабақты енгізіңіз.",
      "Enter a valid R value or leave it blank.": "R мәнін дұрыс енгізіңіз немесе өрісті бос қалдырыңыз.",
      "The local model is not ready yet.": "Жергілікті модель әлі дайын емес.",
      "Progress": "Орындалу барысы",
      "Unknown size": "Өлшемі белгісіз",
      "Elapsed": "Уақыты",
      "Installing": "Орнатылуда",
      "Starting server": "Сервер іске қосылуда",
      "Stopping server": "Сервер тоқтатылуда",
      "Pinging model": "Модель тексерілуде",
      "Asking model": "Модельге сұрау",
      "Saving lesson": "Сабақ сақталуда",
      "Deleting lesson": "Сабақ жойылуда",
      "Exporting": "Экспортталуда",
      "Yes": "Иә",
      "No": "Жоқ",
      "Loopback only": "Тек loopback",
      "Owned by this app": "Осы қолданба іске қосқан",
      "Server is ready": "Сервер дайын",
      "No local server": "Жергілікті сервер іске қосылмаған",
    },
  };

  const state = { status: null, statusError: "", operationError: "", action: "", answer: "", memoriesUsed: 0, elapsedMs: null, memories: [], timer: null, polling: false };
  const t = (text) => (DICT[window.I18N?.lang] && DICT[window.I18N.lang][text]) || text;
  const $ = (root, selector) => root.querySelector(selector);
  const asError = (error) => String(error?.message || error).replace(/^Error:\s*/, "");
  const isPanelReady = (container) => container.isConnected && S.panel === "local_ai" && Boolean($(container, "[data-lai-status]"));

  function statusMarkup() {
    return `<section class="lai-card" aria-labelledby="lai-status-title">
      <div class="lai-section-head"><h3 id="lai-status-title">${t("Model status")}</h3><button type="button" class="ghost sm" data-lai="refresh">${t("Retry status")}</button></div>
      <div class="lai-status" data-lai-status aria-live="polite">${t("Checking local model…")}</div>
      <div class="lai-progress" data-lai-progress hidden></div>
      <div class="lai-error" data-lai-status-error role="alert" hidden></div>
      <div class="btn-row lai-actions">
        <button type="button" class="ok" data-lai="install" disabled>${t("Install locally")}</button>
        <button type="button" class="ghost" data-lai="start" disabled>${t("Start server")}</button>
        <button type="button" class="ghost" data-lai="stop" disabled>${t("Stop server")}</button>
        <button type="button" class="ghost" data-lai="ping" disabled>${t("Ping model")}</button>
      </div>
      <details class="lai-consent">
        <summary>${t("Before installation")}</summary>
        <p>${t("The model weights are about 5.2 GB, plus about 1.5 GB for the runtime. Several GB may be needed after extraction. The server listens on loopback only. No drivers are installed automatically; required drivers must already be present.")}</p>
        <label class="lai-check"><input type="checkbox" data-lai-consent><span>${t("I understand and approve downloading and installing the local model and runtime on this computer.")}</span></label>
      </details>
      <p class="lai-note" data-lai-note></p>
    </section>`;
  }

  function renderMarkup() {
    return `<div class="lai">
      <p class="lai-intro">${t("Private, on-device model tools. Live trading is not part of this step.")}</p>
      ${statusMarkup()}
      <section class="lai-card" aria-labelledby="lai-ask-title">
        <h3 id="lai-ask-title">${t("Ask a question")}</h3>
        <p class="lai-note">${t("Ask a normal question to the local model. This is a model test only: no trading action and no external API request.")}</p>
        <form data-lai-form="ask">
          <div class="field"><label for="lai-prompt">${t("Your question")}</label><textarea id="lai-prompt" data-lai-prompt rows="4" maxlength="500" required></textarea></div>
          <button type="submit" class="ok" data-lai-ask disabled>${t("Ask model")}</button>
        </form>
        <div class="lai-error" data-lai-ask-error role="alert" hidden></div>
        <div class="lai-answer" data-lai-answer hidden><h4>${t("Last answer")}</h4><p data-lai-answer-text></p><div class="hint">${t("Memories used")}: <strong data-lai-used>0</strong><span data-lai-elapsed></span></div></div>
      </section>
      <section class="lai-card" aria-labelledby="lai-memory-title">
        <h3 id="lai-memory-title">${t("Experience memory")}</h3>
        <p class="lai-note">${t("This stores explicit lessons as local experience records; it does not fine-tune model weights. Lessons are entered by you, not generated or saved from assistant answers.")}</p>
        <form data-lai-form="memory">
          <div class="field"><label for="lai-strategy">${t("Strategy")}</label><input id="lai-strategy" data-lai-strategy value="bounce" required></div>
          <div class="field"><label for="lai-observation">${t("Observation")}</label><textarea id="lai-observation" data-lai-observation rows="3" required></textarea></div>
          <div class="field"><label for="lai-lesson">${t("Lesson")}</label><textarea id="lai-lesson" data-lai-lesson rows="3" required></textarea></div>
          <div class="field"><label for="lai-outcome">${t("Outcome (R, optional)")}</label><input id="lai-outcome" data-lai-outcome type="number" step="any"></div>
          <button type="submit" class="ok" data-lai-add>${t("Add lesson")}</button>
        </form>
        <div class="lai-error" data-lai-memory-error role="alert" hidden></div>
        <div class="lai-section-head lai-memory-head"><h4>${t("Saved lessons")} <span data-lai-count>0</span></h4><button type="button" class="ghost sm" data-lai="export" disabled>${t("Export JSONL")}</button></div>
        <p class="lai-note">${t("Export downloads a local file; nothing is transmitted.")}</p>
        <div class="lai-memory-list" data-lai-memories></div>
      </section>
    </div>`;
  }

  function formatBytes(value) {
    const bytes = Number(value);
    if (!Number.isFinite(bytes) || bytes < 0) return t("Unknown size");
    if (bytes === 0) return "0 B";
    const units = ["B", "KB", "MB", "GB", "TB"];
    const index = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), units.length - 1);
    return `${(bytes / 1024 ** index).toFixed(index > 1 ? 1 : 0)} ${units[index]}`;
  }

  function updateStatus(container) {
    if (!isPanelReady(container)) return;
    const target = $(container, "[data-lai-status]");
    const progress = $(container, "[data-lai-progress]");
    const error = $(container, "[data-lai-status-error]");
    const install = $(container, '[data-lai="install"]');
    const start = $(container, '[data-lai="start"]');
    const stop = $(container, '[data-lai="stop"]');
    const ping = $(container, '[data-lai="ping"]');
    const ask = $(container, "[data-lai-ask]");
    const consent = $(container, "[data-lai-consent]");
    const status = state.status;

    const statusError = state.operationError || state.statusError;
    error.hidden = !statusError;
    error.textContent = statusError;
    if (!status) {
      target.textContent = t("Checking local model…");
    } else {
      const server = status.server_ready ? t("Server is ready") : status.owned_server ? t("Starting") : t("No local server");
      const loadedOn = !status.model_loaded ? t("Not loaded") : Number(status.size_vram) > 0 ? t("GPU") : t("CPU");
      target.innerHTML = `<dl class="lai-status-grid">
        <div><dt>${t("Model")}</dt><dd>${esc(status.model || "—")}</dd></div>
        <div><dt>${t("Endpoint")}</dt><dd>${esc(status.endpoint || "—")}</dd></div>
        <div><dt>${t("Version")}</dt><dd>${esc(status.version || "—")}</dd></div>
        <div><dt>${t("Server")}</dt><dd>${esc(server)}</dd></div>
        <div><dt>${t("Model files")}</dt><dd>${esc(status.model_downloaded ? t("Downloaded") : t("Not downloaded"))}${status.installed ? ` · ${esc(t("Installed"))}` : ` · ${esc(t("Not installed"))}`}</dd></div>
        <div><dt>${t("Loaded on")}</dt><dd>${esc(loadedOn)}${status.model_loaded && Number(status.size_vram) > 0 ? ` · ${esc(formatBytes(status.size_vram))} VRAM` : ""}</dd></div>
        <div><dt>${t("Stored experiences")}</dt><dd>${esc(status.memory_count ?? state.memories.length)}</dd></div>
        <div><dt>${t("Data directory")}</dt><dd>${esc(status.data_dir || "—")}</dd></div>
        <div><dt>${t("Loopback only")}</dt><dd>${t("Yes")}</dd></div>
        <div><dt>${t("Owned by this app")}</dt><dd>${status.owned_server ? t("Yes") : t("No")}</dd></div>
      </dl>`;
    }

    const p = status?.progress;
    if (p?.busy || p?.error) {
      const total = Number(p.total) || 0;
      const completed = Number(p.completed) || 0;
      const known = total > 0;
      const pct = known ? Math.min(100, Math.round((completed / total) * 100)) : 0;
      const summary = p.error ? esc(asError(p.error)) : `${esc(p.stage || t("Progress"))}${known ? ` · ${pct}% (${esc(formatBytes(completed))} / ${esc(formatBytes(total))})` : completed ? ` · ${esc(formatBytes(completed))}` : ""}`;
      progress.hidden = false;
      progress.innerHTML = `<div class="lai-progress-label">${summary}</div>${p.busy ? `<progress aria-label="${esc(t("Progress"))}"${known ? ` max="100" value="${pct}"` : ""}></progress>` : ""}`;
    } else {
      progress.hidden = true;
      progress.replaceChildren();
    }

    const busy = Boolean(state.action) || Boolean(status?.progress?.busy);
    install.disabled = busy || !consent.checked || Boolean(status?.installed && status?.model_downloaded);
    start.disabled = busy || !status?.installed || !status?.model_downloaded;
    stop.disabled = busy || !status?.owned_server;
    ping.disabled = busy || !status?.server_ready || !status?.model_downloaded;
    ask.disabled = busy || !status?.server_ready || !status?.model_downloaded;
    $(container, "[data-lai-add]").disabled = busy;
    $(container, "[data-lai-note]").textContent = state.action ? `${state.action}…` : "";
  }

  function renderMemories(container) {
    if (!isPanelReady(container)) return;
    const list = $(container, "[data-lai-memories]");
    const count = $(container, "[data-lai-count]");
    const exportButton = $(container, '[data-lai="export"]');
    count.textContent = String(state.memories.length);
    exportButton.disabled = state.memories.length === 0 || Boolean(state.action);
    if (!state.memories.length) {
      list.innerHTML = `<p class="lai-empty">${t("No lessons saved yet.")}</p>`;
      return;
    }
    list.innerHTML = state.memories.map((memory) => `<article class="lai-memory" data-memory-id="${esc(memory.id)}">
      <header><strong>${esc(memory.strategy)}</strong>${memory.outcome_r === null || memory.outcome_r === undefined ? "" : `<span>${esc(Number(memory.outcome_r).toFixed(2))} R</span>`}<button type="button" class="ghost sm" data-lai-delete="${esc(memory.id)}"${state.action ? " disabled" : ""}>${t("Delete")}</button></header>
      <p><b>${t("Observation")}:</b> ${esc(memory.observation)}</p><p><b>${t("Lesson")}:</b> ${esc(memory.lesson)}</p>
    </article>`).join("");
  }

  function setError(container, selector, message) {
    const node = $(container, selector);
    if (!node) return;
    node.textContent = message;
    node.hidden = !message;
  }

  function setAction(container, action) {
    state.action = action;
    updateStatus(container);
    renderMemories(container);
  }

  async function refreshStatus(container) {
    if (state.polling || !isPanelReady(container)) return;
    state.polling = true;
    try {
      state.status = await invoke("local_ai_status");
      state.statusError = "";
    } catch (error) {
      state.statusError = asError(error);
    } finally {
      state.polling = false;
    }
    if (isPanelReady(container)) updateStatus(container);
  }

  async function loadMemories(container) {
    try {
      state.memories = await invoke("local_ai_memory_list");
      if (isPanelReady(container)) renderMemories(container);
    } catch (error) {
      setError(container, "[data-lai-memory-error]", asError(error));
    }
  }

  async function perform(container, action, command, args) {
    if (state.action) return false;
    state.operationError = "";
    setError(container, "[data-lai-status-error]", "");
    setAction(container, t(action));
    try {
      const result = await invoke(command, args);
      if (command === "local_ai_status" || command === "local_ai_install" || command === "local_ai_start" || command === "local_ai_stop") {
        state.status = result;
      } else if (command === "local_ai_memory_add" || command === "local_ai_memory_delete") {
        state.memories = result;
      } else if (command === "local_ai_ping") {
        state.answer = result.response;
        state.memoriesUsed = 0;
        state.elapsedMs = result.elapsed_ms;
        if (isPanelReady(container)) showAnswer(container);
      } else if (command === "local_ai_ask") {
        state.answer = result.response;
        state.memoriesUsed = result.memories_used;
        state.elapsedMs = result.elapsed_ms;
        if (isPanelReady(container)) showAnswer(container);
      } else if (command === "local_ai_memory_export") {
        downloadExport(result);
      }
      log(`${t("Local AI")}: ${t(action)}`, "ok");
      state.statusError = "";
      state.operationError = "";
      setError(container, "[data-lai-ask-error]", "");
      setError(container, "[data-lai-memory-error]", "");
      return true;
    } catch (error) {
      const message = asError(error);
      if (command === "local_ai_ask" || command === "local_ai_ping") setError(container, "[data-lai-ask-error]", message);
      else if (command === "local_ai_memory_add" || command === "local_ai_memory_delete" || command === "local_ai_memory_export") setError(container, "[data-lai-memory-error]", message);
      else {
        state.operationError = message;
        setError(container, "[data-lai-status-error]", message);
      }
      return false;
    } finally {
      state.action = "";
      if (isPanelReady(container)) {
        updateStatus(container);
        renderMemories(container);
      }
    }
  }

  function showAnswer(container) {
    const box = $(container, "[data-lai-answer]");
    box.hidden = !state.answer;
    $(container, "[data-lai-answer-text]").textContent = state.answer;
    $(container, "[data-lai-used]").textContent = String(state.memoriesUsed);
    $(container, "[data-lai-elapsed]").textContent = state.elapsedMs == null ? "" : ` · ${t("Elapsed")}: ${state.elapsedMs} ms`;
  }

  function downloadExport(jsonl) {
    const blob = new Blob([String(jsonl)], { type: "application/x-ndjson;charset=utf-8" });
    const url = URL.createObjectURL(blob);
    const link = document.createElement("a");
    link.href = url;
    link.download = `aegis-local-ai-memory-${new Date().toISOString().slice(0, 10)}.jsonl`;
    link.click();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
  }

  function bind(container) {
    container.addEventListener("click", (event) => {
      const button = event.target.closest("button[data-lai],button[data-lai-delete]");
      if (!button || !container.contains(button)) return;
      if (button.dataset.lai === "refresh") {
        state.operationError = "";
        refreshStatus(container);
      }
      if (button.dataset.lai === "install") perform(container, "Installing", "local_ai_install");
      if (button.dataset.lai === "start") perform(container, "Starting server", "local_ai_start");
      if (button.dataset.lai === "stop") perform(container, "Stopping server", "local_ai_stop");
      if (button.dataset.lai === "ping") perform(container, "Pinging model", "local_ai_ping");
      if (button.dataset.lai === "export") perform(container, "Exporting", "local_ai_memory_export");
      if (button.dataset.laiDelete !== undefined) {
        const rawId = button.dataset.laiDelete;
        const id = /^\d+$/.test(rawId) ? Number(rawId) : rawId;
        perform(container, "Deleting lesson", "local_ai_memory_delete", { id });
      }
    });
    container.addEventListener("change", (event) => {
      if (event.target.matches("[data-lai-consent]")) updateStatus(container);
    });
    container.addEventListener("submit", (event) => {
      const form = event.target;
      if (!form.matches("[data-lai-form]")) return;
      event.preventDefault();
      if (form.dataset.laiForm === "ask") {
        const prompt = $(container, "[data-lai-prompt]").value.trim();
        if (!prompt) return setError(container, "[data-lai-ask-error]", t("Enter a question first."));
        perform(container, "Asking model", "local_ai_ask", { prompt, strategy: $(container, "[data-lai-strategy]").value.trim() || "bounce" });
      } else {
        const strategy = $(container, "[data-lai-strategy]").value.trim() || "bounce";
        const observation = $(container, "[data-lai-observation]").value.trim();
        const lesson = $(container, "[data-lai-lesson]").value.trim();
        const rawOutcome = $(container, "[data-lai-outcome]").value.trim();
        if (!observation || !lesson) return setError(container, "[data-lai-memory-error]", t("Enter both an observation and a lesson."));
        const outcomeR = rawOutcome === "" ? null : Number(rawOutcome);
        if (outcomeR !== null && !Number.isFinite(outcomeR)) return setError(container, "[data-lai-memory-error]", t("Enter a valid R value or leave it blank."));
        perform(container, "Saving lesson", "local_ai_memory_add", { strategy, observation, lesson, outcomeR }).then((saved) => {
          if (saved && isPanelReady(container)) {
            $(container, "[data-lai-observation]").value = "";
            $(container, "[data-lai-lesson]").value = "";
            $(container, "[data-lai-outcome]").value = "";
          }
        });
      }
    });
  }

  function render(container) {
    if (!container) return;
    container.classList.add("lai-panel-body");
    container.innerHTML = renderMarkup();
    container = $(container, ".lai");
    bind(container);
    updateStatus(container);
    showAnswer(container);
    renderMemories(container);
    refreshStatus(container);
    loadMemories(container);
    if (state.timer) clearInterval(state.timer);
    state.timer = setInterval(() => {
      if (S.panel !== "local_ai" || !container.isConnected) {
        clearInterval(state.timer);
        state.timer = null;
        return;
      }
      refreshStatus(container);
    }, 1000);
  }

  window.AEGIS.localAI = { render };
})();
