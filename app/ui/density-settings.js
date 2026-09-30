"use strict";

(() => {
  const defaults = {
    poll_seconds: 1,
    auto_threshold: true,
    strength_multiplier: 3,
    window_minutes: 60,
    percentile: 95,
    min_quantity: 0,
    max_distance: 10,
    min_age_seconds: 0,
    min_touches: 0,
    min_strength: 0,
    bid_color: "#30d158",
    ask_color: "#ff453a",
    highlight: true,
    sort: "strength",
    recording: true,
    retention_days: 0,
    max_history_mb: 0,
    auto_open: true,
    docked: true,
  };

  const groups = [
    ["Collection", [
      ["poll_seconds", "Poll interval (seconds)", "number", 0.5, 10, 0.1],
      ["recording", "Record density history", "checkbox"],
    ]],
    ["Threshold", [
      ["auto_threshold", "Automatic threshold", "checkbox"],
      ["strength_multiplier", "Strength multiplier", "number", 1.5, 20, 0.1],
      ["window_minutes", "Activity window (minutes)", "number", 5, 240, 1],
      ["percentile", "Activity percentile", "number", 80, 99.9, 0.1],
    ]],
    ["Filters", [
      ["min_quantity", "Minimum quantity", "number", 0, null, "any"],
      ["max_distance", "Maximum distance ($)", "number", 0.1, 1000, 0.1],
      ["min_age_seconds", "Minimum level age (seconds)", "number", 0, 86400, 1],
      ["min_touches", "Minimum touches", "number", 0, 1000, 1],
      ["min_strength", "Minimum strength", "number", 0, 100, 0.1],
    ]],
    ["Appearance", [
      ["bid_color", "Bid color", "color"],
      ["ask_color", "Ask color", "color"],
      ["highlight", "Highlight stronger levels", "checkbox"],
      ["sort", "Sort order", "select"],
    ]],
    ["History", [
      ["retention_days", "History retention (days, 0 = unlimited)", "number", 0, 365, 1],
      ["max_history_mb", "Maximum history size (MB, 0 = unlimited)", "number", 0, 10240, 1],
    ]],
    ["Window", [
      ["auto_open", "Open with terminal", "checkbox"],
      ["docked", "Keep beside terminal", "checkbox"],
    ]],
  ];

  const words = {
    en: {
      Collection: "Collection", Threshold: "Threshold", Filters: "Filters", Appearance: "Appearance", History: "History", Window: "Window",
      "Poll interval (seconds)": "Poll interval (seconds)", "Automatic threshold": "Automatic threshold",
      "Strength multiplier": "Strength multiplier", "Activity window (minutes)": "Activity window (minutes)",
      "Activity percentile": "Activity percentile", "Minimum quantity": "Minimum quantity",
      "Maximum distance ($)": "Maximum distance ($)", "Minimum level age (seconds)": "Minimum level age (seconds)",
      "Minimum touches": "Minimum touches", "Minimum strength": "Minimum strength", "Bid color": "Bid color",
      "Ask color": "Ask color", "Highlight stronger levels": "Highlight stronger levels", "Sort order": "Sort order",
      "Record density history": "Record density history", "History retention (days)": "History retention (days)",
      "Maximum history size (MB)": "Maximum history size (MB)", "Open with terminal": "Open with terminal",
      "History retention (days, 0 = unlimited)": "History retention (days, 0 = unlimited)",
      "Maximum history size (MB, 0 = unlimited)": "Maximum history size (MB, 0 = unlimited)",
      "Keep beside terminal": "Keep beside terminal", Strength: "Strength", Distance: "Distance",
      "Book activity, not traded volume.": "Thresholds use order-book activity, not traded volume.",
      "History cleanup warning: nonzero limits can delete old completed density history files. The active file is kept. Set both limits to 0 to disable automatic cleanup.": "History cleanup warning: nonzero limits can delete old completed density history files. The active file is kept. Set both limits to 0 to disable automatic cleanup.",
      "Maximum history size must be 0 or at least 16 MB.": "Maximum history size must be 0 or at least 16 MB.",
      "Save screener settings": "Save screener settings", "Settings saved.": "Settings saved.",
      "Preview only · settings are not saved without the desktop app.": "Preview only · settings are not saved without the desktop app.",
      "Loading screener settings…": "Loading screener settings…", "Could not load saved screener settings.": "Could not load saved screener settings.",
      "Desktop screener settings are unavailable.": "Desktop screener settings are unavailable.",
      "Enter values within the displayed limits.": "Enter values within the displayed limits.",
    },
    ru: {
      Collection: "Сбор данных", Threshold: "Порог", Filters: "Фильтры", Appearance: "Вид", History: "История", Window: "Окно",
      "Poll interval (seconds)": "Интервал опроса (сек.)", "Automatic threshold": "Автоматический порог",
      "Strength multiplier": "Множитель силы", "Activity window (minutes)": "Окно активности (мин.)",
      "Activity percentile": "Перцентиль активности", "Minimum quantity": "Минимальный объём",
      "Maximum distance ($)": "Максимальное расстояние ($)", "Minimum level age (seconds)": "Минимальный возраст уровня (сек.)",
      "Minimum touches": "Минимум касаний", "Minimum strength": "Минимальная сила", "Bid color": "Цвет покупки",
      "Ask color": "Цвет продажи", "Highlight stronger levels": "Выделять сильные уровни", "Sort order": "Сортировка",
      "Record density history": "Записывать историю плотностей", "History retention (days)": "Хранить историю (дней)",
      "Maximum history size (MB)": "Максимальный размер истории (МБ)", "Open with terminal": "Открывать вместе с терминалом",
      "History retention (days, 0 = unlimited)": "Хранение истории (дней, 0 = без ограничений)",
      "Maximum history size (MB, 0 = unlimited)": "Размер истории (МБ, 0 = без ограничений)",
      "Keep beside terminal": "Закрепить у терминала", Strength: "Сила", Distance: "Расстояние",
      "Book activity, not traded volume.": "Порог учитывает активность стакана, а не объём сделок.",
      "History cleanup warning: nonzero limits can delete old completed density history files. The active file is kept. Set both limits to 0 to disable automatic cleanup.": "Очистка истории: ненулевые ограничения могут удалить старые завершённые файлы истории плотностей. Активный файл сохраняется. Установите 0 для обоих ограничений, чтобы отключить автоматическую очистку.",
      "Maximum history size must be 0 or at least 16 MB.": "Максимальный размер должен быть 0 или не менее 16 МБ.",
      "Save screener settings": "Сохранить настройки скринера", "Settings saved.": "Настройки сохранены.",
      "Preview only · settings are not saved without the desktop app.": "Предпросмотр · без настольного приложения настройки не сохраняются.",
      "Loading screener settings…": "Загрузка настроек скринера…", "Could not load saved screener settings.": "Не удалось загрузить настройки скринера.",
      "Desktop screener settings are unavailable.": "Настройки скринера недоступны в настольном приложении.",
      "Enter values within the displayed limits.": "Введите значения в указанных пределах.",
    },
    kk: {
      Collection: "Жинау", Threshold: "Шек", Filters: "Сүзгілер", Appearance: "Көрініс", History: "Тарих", Window: "Терезе",
      "Poll interval (seconds)": "Сұрау аралығы (сек.)", "Automatic threshold": "Автоматты шек",
      "Strength multiplier": "Күш көбейткіші", "Activity window (minutes)": "Белсенділік терезесі (мин.)",
      "Activity percentile": "Белсенділік перцентилі", "Minimum quantity": "Ең аз көлем",
      "Maximum distance ($)": "Ең үлкен қашықтық ($)", "Minimum level age (seconds)": "Деңгейдің ең аз жасы (сек.)",
      "Minimum touches": "Ең аз жанасу саны", "Minimum strength": "Ең аз күш", "Bid color": "Сатып алу түсі",
      "Ask color": "Сату түсі", "Highlight stronger levels": "Күшті деңгейлерді ерекшелеу", "Sort order": "Сұрыптау",
      "Record density history": "Тығыздық тарихын жазу", "History retention (days)": "Тарихты сақтау (күн)",
      "Maximum history size (MB)": "Тарихтың ең үлкен өлшемі (МБ)", "Open with terminal": "Терминалмен бірге ашу",
      "History retention (days, 0 = unlimited)": "Тарихты сақтау (күн, 0 = шектеусіз)",
      "Maximum history size (MB, 0 = unlimited)": "Тарих көлемі (МБ, 0 = шектеусіз)",
      "Keep beside terminal": "Терминал жанында бекіту", Strength: "Күші", Distance: "Қашықтық",
      "Book activity, not traded volume.": "Шек стакан белсенділігін өлшейді, сауда көлемін емес.",
      "History cleanup warning: nonzero limits can delete old completed density history files. The active file is kept. Set both limits to 0 to disable automatic cleanup.": "Тарихты тазарту: нөлден өзге шектер ескі аяқталған тығыздық тарихы файлдарын жоюы мүмкін. Белсенді файл сақталады. Автоматты тазартуды өшіру үшін екі шекті де 0 етіңіз.",
      "Maximum history size must be 0 or at least 16 MB.": "Ең үлкен тарих көлемі 0 немесе кемінде 16 МБ болуы керек.",
      "Save screener settings": "Скринер баптауларын сақтау", "Settings saved.": "Баптаулар сақталды.",
      "Preview only · settings are not saved without the desktop app.": "Алдын ала көру · жұмыс үстелі қолданбасынсыз баптаулар сақталмайды.",
      "Loading screener settings…": "Скринер баптаулары жүктелуде…", "Could not load saved screener settings.": "Скринер баптауларын жүктеу мүмкін болмады.",
      "Desktop screener settings are unavailable.": "Жұмыс үстелі қолданбасында скринер баптаулары қолжетімсіз.",
      "Enter values within the displayed limits.": "Мәндерді көрсетілген аралықта енгізіңіз.",
    },
  };

  const t = (key, lang) => (words[lang] || words.en)[key] || key;
  const esc = (value) => String(value ?? "").replace(/[&<>"']/g, (ch) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[ch]);
  const merge = (settings) => ({ ...defaults, ...(settings || {}) });
  let draft = null;
  let saving = false;

  function render(container, options = {}) {
    const { settings, lang = "en", preview = false, loading = false, loadError = "", invoke, onSaved, onRetry } = options;
    let savedValues = merge(settings);
    let values = merge({ ...savedValues, ...draft });
    const label = (text) => esc(t(text, lang));
    const message = loading ? t("Loading screener settings…", lang)
      : preview ? t("Preview only · settings are not saved without the desktop app.", lang)
        : loadError ? `${t("Could not load saved screener settings.", lang)} ${loadError}` : "";

    const sections = groups.map(([title, fields]) => `<section class="density-settings-section"><h3>${label(title)}</h3><div class="density-settings-grid">${fields.map(([key, name, type, min, max, step]) => {
      const disabled = loading ? " disabled" : "";
      if (type === "checkbox") return `<label class="density-setting-toggle"><input data-key="${key}" type="checkbox" ${values[key] ? "checked" : ""}${disabled}><span>${label(name)}</span></label>`;
      if (type === "select") return `<label class="density-setting-field"><span>${label(name)}</span><select data-key="${key}"${disabled}><option value="strength">${label("Strength")}</option><option value="distance">${label("Distance")}</option></select></label>`;
      const limits = `${min !== null && min !== undefined ? ` min="${min}"` : ""}${max !== null && max !== undefined ? ` max="${max}"` : ""}${step ? ` step="${step}"` : ""}`;
      return `<label class="density-setting-field"><span>${label(name)}</span><input data-key="${key}" type="${type}" value="${esc(values[key])}"${type === "number" ? " required" : ""}${limits}${disabled}></label>`;
    }).join("")}</div></section>`).join("");

    container.innerHTML = `<div class="density-settings-form">
      <p class="density-settings-note">${label("Book activity, not traded volume.")}</p>
      <p class="density-settings-warning">${label("History cleanup warning: nonzero limits can delete old completed density history files. The active file is kept. Set both limits to 0 to disable automatic cleanup.")}</p>
      ${sections}
      <p class="density-settings-status" role="status" aria-live="polite">${esc(message)}</p>
      <div class="density-settings-actions">${loadError && onRetry ? `<button class="ghost sm density-settings-retry" type="button">${esc(lang === "ru" ? "Повторить загрузку" : lang === "kk" ? "Қайта жүктеу" : "Retry loading")}</button>` : ""}<button class="ok density-settings-save" type="button" ${preview || loading || loadError || !invoke ? "disabled" : ""}>${label("Save screener settings")}</button></div>
    </div>`;
    const status = container.querySelector(".density-settings-status");
    const save = container.querySelector(".density-settings-save");
    const retry = container.querySelector(".density-settings-retry");
    if (retry) retry.onclick = onRetry;
    const setStatus = (text, error = false) => {
      status.textContent = text;
      status.dataset.error = error ? "1" : "0";
    };
    container.querySelectorAll("[data-key]").forEach((input) => {
      if (input.dataset.key === "sort") input.value = values.sort;
      const update = () => {
        if (!draft) draft = {};
        draft[input.dataset.key] = input.type === "checkbox" ? input.checked
          : input.type === "number" && input.value !== "" ? Number(input.value) : input.value;
        setStatus("");
      };
      input.addEventListener("input", update);
      input.addEventListener("change", update);
    });
    save.onclick = async () => {
      const invalid = [...container.querySelectorAll('input[type="number"]')].some((input) => !input.checkValidity());
      const maxHistory = Number(container.querySelector('[data-key="max_history_mb"]').value);
      if (invalid) {
        setStatus(t("Enter values within the displayed limits.", lang), true);
        return;
      }
      if (maxHistory > 0 && maxHistory < 16) {
        setStatus(t("Maximum history size must be 0 or at least 16 MB.", lang), true);
        return;
      }
      if (!invoke) {
        setStatus(t("Desktop screener settings are unavailable.", lang), true);
        return;
      }
      save.disabled = true;
      saving = true;
      const inputs = [...container.querySelectorAll("input, select")];
      inputs.forEach((input) => { input.disabled = true; });
      try {
        const saved = merge(await invoke("density_settings_save", { settings: merge({ ...savedValues, ...draft }) }));
        savedValues = saved;
        values = saved;
        draft = null;
        onSaved?.(saved);
        setStatus(t("Settings saved.", lang));
      } catch (error) {
        setStatus(String(error), true);
      } finally {
        saving = false;
        inputs.forEach((input) => { input.disabled = false; });
        save.disabled = false;
      }
    };
    return { get settings() { return merge({ ...values, ...draft }); } };
  }

  window.DensitySettingsUI = { defaults, merge, render, get saving() { return saving; } };
})();
