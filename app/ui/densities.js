"use strict";

(() => {
  const $ = (id) => document.getElementById(id);
  const tauri = window.__TAURI__;
  const invoke = (cmd, args) => tauri.core.invoke(cmd, args);
  const words = {
    en: {
      title: "Densities", waiting: "Waiting for source", noBroker: "No broker connected", live: "Live", unavailable: "Unavailable",
      error: "Source error", recording: "Recording", recordingOff: "Recording inactive", updated: "Updated {time}",
      list: "Live density list", dock: "Keep beside terminal", sort: "Sort by", distance: "Distance", strengthSort: "Strength",
      distanceUnavailable: "Market price unavailable", noData: "No densities yet", noDataHelp: "Levels from the selected broker will appear here when available.",
      noBrokerHelp: "Connect a broker in the terminal to monitor its gold feed.", unavailableHelp: "The density feed is unavailable in this view.",
      errorHelp: "Density data could not be loaded.", probability: "Probability: not calibrated", scoreNote: "Score is observational, not a probability.",
      storage: "History is saved in app data · densities", bid: "BID", ask: "ASK", age: "Age", price: "Price", quantity: "Quantity",
      notional: "Notional", strength: "Strength", touches: "Touches", reactions: "Reactions", score: "Score", probabilityShort: "Probability",
      notCalibrated: "Not calibrated", hide: "Hide screener", noSource: "No broker", waitingStatus: "Waiting", sourceUnavailable: "Unavailable",
      since: "Observed {age}", reference: "from book mid", seen: "Last seen", unknownStatus: "Status: {status}",
    },
    ru: {
      title: "Плотности", waiting: "Ожидание источника", noBroker: "Нет подключённого брокера", live: "Поток активен", unavailable: "Недоступно",
      error: "Ошибка источника", recording: "Запись идёт", recordingOff: "Запись неактивна", updated: "Обновлено {time}",
      list: "Список плотностей", dock: "Закрепить у терминала", sort: "Сортировка", distance: "Расстояние", strengthSort: "Сила",
      distanceUnavailable: "Рыночная цена недоступна", noData: "Плотностей пока нет", noDataHelp: "Здесь появятся уровни выбранного брокера, когда они будут доступны.",
      noBrokerHelp: "Подключите брокера в терминале, чтобы отслеживать его поток золота.", unavailableHelp: "Поток плотностей недоступен в этом окне.",
      errorHelp: "Не удалось загрузить данные плотностей.", probability: "Вероятность: не калибрована", scoreNote: "Оценка наблюдательная, это не вероятность.",
      storage: "История сохраняется в данных приложения · densities", bid: "ПОКУПКА", ask: "ПРОДАЖА", age: "Возраст", price: "Цена", quantity: "Объём",
      notional: "Номинал", strength: "Сила", touches: "Касания", reactions: "Реакции", score: "Оценка", probabilityShort: "Вероятность",
      notCalibrated: "Не калибрована", hide: "Скрыть скринер", noSource: "Нет брокера", waitingStatus: "Ожидание", sourceUnavailable: "Недоступно",
      since: "Стоит {age}", reference: "от mid стакана", seen: "Обновлён", unknownStatus: "Статус: {status}",
    },
    kk: {
      title: "Тығыздықтар", waiting: "Дереккөз күтілуде", noBroker: "Брокер қосылмаған", live: "Ағын белсенді", unavailable: "Қолжетімсіз",
      error: "Дереккөз қатесі", recording: "Жазу жүріп жатыр", recordingOff: "Жазу белсенді емес", updated: "Жаңартылды {time}",
      list: "Тығыздықтар тізімі", dock: "Терминал жанында бекіту", sort: "Сұрыптау", distance: "Қашықтық", strengthSort: "Күші",
      distanceUnavailable: "Нарық бағасы қолжетімсіз", noData: "Әзірше тығыздықтар жоқ", noDataHelp: "Таңдалған брокердің деңгейлері қолжетімді болғанда осында көрсетіледі.",
      noBrokerHelp: "Алтын ағынын бақылау үшін терминалда брокерді қосыңыз.", unavailableHelp: "Бұл терезеде тығыздық ағыны қолжетімсіз.",
      errorHelp: "Тығыздық деректерін жүктеу мүмкін болмады.", probability: "Ықтималдық: калибрленбеген", scoreNote: "Баға — бақылау көрсеткіші, ықтималдық емес.",
      storage: "Тарих қолданба деректерінде сақталады · densities", bid: "САТЫП АЛУ", ask: "САТУ", age: "Жасы", price: "Баға", quantity: "Көлем",
      notional: "Номинал", strength: "Күші", touches: "Тию саны", reactions: "Реакциялар", score: "Баға", probabilityShort: "Ықтималдық",
      notCalibrated: "Калибрленбеген", hide: "Скринерді жасыру", noSource: "Брокер жоқ", waitingStatus: "Күтілуде", sourceUnavailable: "Қолжетімсіз",
      since: "Басталуы: {age}", reference: "нарықтан", seen: "Жаңартылды", unknownStatus: "Күйі: {status}",
    },
  };

  const S = { lang: "en", sort: "strength", snapshot: null, eventCount: 0, revision: -1, docked: true, error: null };
  const t = (key, vars = {}) => (words[S.lang] || words.en)[key].replace(/\{(\w+)\}/g, (_, k) => vars[k] ?? "");
  const finite = (n) => n !== null && n !== undefined && n !== "" && Number.isFinite(Number(n));
  const number = (n, digits = 2) => finite(n) ? Number(n).toLocaleString(undefined, { maximumFractionDigits: digits }) : "—";
  const money = (n) => finite(n) ? `$${Number(n).toLocaleString(undefined, { maximumFractionDigits: 0 })}` : "—";
  const age = (stamp) => {
    const seconds = Math.max(0, Math.floor((Date.now() - Number(stamp)) / 1000));
    if (seconds < 60) return `${seconds}s`;
    if (seconds < 3600) return `${Math.floor(seconds / 60)}m`;
    if (seconds < 86400) return `${Math.floor(seconds / 3600)}h ${Math.floor((seconds % 3600) / 60)}m`;
    return `${Math.floor(seconds / 86400)}d ${Math.floor((seconds % 86400) / 3600)}h`;
  };
  const brokerName = (broker) => broker ? String(broker).replaceAll("_", " ").replace(/\b\w/g, (c) => c.toUpperCase()) : "—";
  const statusKind = (payload) => {
    if (S.error) return "unavailable";
    if (!payload) return "waiting";
    if (payload.error) return "error";
    if (payload.status === "live" && payload.broker) {
      return finite(payload.updated_at) && Date.now() - Number(payload.updated_at) <= 5000 ? "live" : "waiting";
    }
    if (["unavailable", "error"].includes(payload.status)) return payload.status;
    if (!payload.broker || payload.status === "no_broker") return "no_broker";
    if (["waiting", "starting", "connecting", "idle"].includes(payload.status)) return "waiting";
    return payload.status ? "unknown" : "waiting";
  };

  function setLanguage(lang) {
    S.lang = ["ru", "kk", "en"].includes(lang) ? lang : "en";
    document.documentElement.lang = S.lang;
    $("title").textContent = t("title");
    $("close").title = t("hide");
    $("close").setAttribute("aria-label", t("hide"));
    $("listTitle").textContent = t("list");
    $("dockLabel").textContent = t("dock");
    $("sortLabel").textContent = t("sort");
    $("sortDistance").textContent = t("distance");
    $("sortStrength").textContent = t("strengthSort");
    $("probabilityNote").textContent = t("probability");
    $("scoreNote").textContent = t("scoreNote");
    $("storage").textContent = t("storage");
    render();
  }

  function setPreferences(prefs) {
    if (prefs?.theme && ["glass-dark", "glass-light", "glass-blue"].includes(prefs.theme)) {
      document.documentElement.dataset.theme = prefs.theme;
    }
    if (prefs?.lang) setLanguage(prefs.lang);
  }

  function applySnapshot(snapshot) {
    if (finite(snapshot?.revision) && Number(snapshot.revision) < S.revision) return;
    if (finite(snapshot?.revision)) S.revision = Number(snapshot.revision);
    S.snapshot = snapshot || null;
    if (typeof snapshot?.docked === "boolean") {
      S.docked = snapshot.docked;
      $("docked").checked = snapshot.docked;
    }
    const kind = statusKind(S.snapshot);
    if (S.snapshot && ["error", "unavailable", "no_broker"].includes(kind)) S.snapshot.rows = [];
    render();
  }

  function renderStatus() {
    const data = S.snapshot;
    const kind = statusKind(data);
    const statusText = {
      live: t("live"), waiting: t("waiting"), no_broker: t("noBroker"), unavailable: t("unavailable"), error: t("error"),
    }[kind] || t("unknownStatus", { status: data?.status || "—" });
    $("status").textContent = statusText;
    $("statusDot").dataset.status = kind;
    const symbol = data?.symbol ? ` · ${data.symbol}` : "";
    $("source").textContent = `${brokerName(data?.broker)}${symbol}`;
    const recording = Boolean(data?.recording);
    $("recording").textContent = recording ? t("recording") : t("recordingOff");
    $("recordIndicator").dataset.recording = recording ? "1" : "0";
    const updated = finite(data?.updated_at) ? new Date(Number(data.updated_at)).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" }) : "—";
    $("updated").textContent = t("updated", { time: updated });
    const errorText = S.error || data?.error || data?.recording_error || "";
    $("error").hidden = !errorText;
    $("error").textContent = errorText;
    const referencePrice = Number(data?.mid_price);
    const hasReference = finite(referencePrice) && referencePrice > 0;
    if (!hasReference && S.sort === "distance") S.sort = "strength";
    $("sortDistance").disabled = !hasReference;
    $("sortDistance").title = hasReference ? t("distance") : t("distanceUnavailable");
    $("distanceNote").hidden = hasReference;
    $("distanceNote").textContent = t("distanceUnavailable");
  }

  function emptyState() {
    const kind = statusKind(S.snapshot);
    const title = kind === "no_broker" ? t("noBroker") : kind === "unavailable" ? t("unavailable") : kind === "error" ? t("error") : t("noData");
    const help = kind === "no_broker" ? t("noBrokerHelp") : kind === "unavailable" ? t("unavailableHelp") : kind === "error" ? t("errorHelp") : t("noDataHelp");
    const box = document.createElement("div");
    box.className = "empty-state";
    const heading = document.createElement("strong");
    heading.textContent = title;
    const description = document.createElement("span");
    description.textContent = help;
    box.append(heading, description);
    return box;
  }

  function metric(label, value) {
    const node = document.createElement("div");
    node.className = "metric";
    const title = document.createElement("span");
    title.className = "metric-label";
    title.textContent = label;
    const content = document.createElement("span");
    content.className = "metric-value";
    content.textContent = value;
    node.append(title, content);
    return node;
  }

  function renderCard(row, referencePrice) {
    const card = document.createElement("article");
    card.className = "density-card";
    const head = document.createElement("div");
    head.className = "density-card-head";
    const side = document.createElement("span");
    side.className = "side-tag";
    side.dataset.side = row.side === "ask" ? "ask" : "bid";
    side.textContent = row.side === "ask" ? t("ask") : t("bid");
    const lifetime = document.createElement("span");
    lifetime.className = "density-age";
    lifetime.textContent = t("since", { age: age(row.first_seen) });
    head.append(side, lifetime);

    const priceRow = document.createElement("div");
    priceRow.className = "density-price-row";
    const price = document.createElement("strong");
    price.className = "density-price";
    price.textContent = number(row.price, 2);
    const distance = document.createElement("span");
    distance.className = "density-distance";
    distance.textContent = referencePrice > 0 && finite(row.price) ? `${number(Math.abs(Number(row.price) - referencePrice), 2)} ${t("reference")}` : "";
    priceRow.append(price, distance);

    const metrics = document.createElement("div");
    metrics.className = "density-metrics";
    metrics.append(
      metric(t("quantity"), number(row.quantity)),
      metric(t("notional"), S.snapshot?.broker === "roboforex" ? number(row.notional, 0) : money(row.notional)),
      metric(t("strength"), number(row.strength, 3)),
      metric(t("touches"), number(row.touches, 0)),
      metric(t("reactions"), number(row.reactions, 0)),
      metric(t("probabilityShort"), t("notCalibrated")),
    );
    const footer = document.createElement("div");
    footer.className = "density-meta";
    const lastSeen = document.createElement("span");
    lastSeen.textContent = `${t("seen")}: ${age(row.last_seen)}`;
    const score = document.createElement("span");
    score.textContent = `${t("score")}: ${number(row.score, 1)} / 100`;
    footer.append(lastSeen, score);
    card.append(head, priceRow, metrics, footer);
    return card;
  }

  function renderRows() {
    const data = S.snapshot;
    const rows = Array.isArray(data?.rows) ? data.rows.slice() : [];
    const referencePrice = Number(data?.mid_price);
    if (S.sort === "distance" && finite(referencePrice)) {
      rows.sort((a, b) => Math.abs(Number(a.price) - referencePrice) - Math.abs(Number(b.price) - referencePrice));
    } else {
      rows.sort((a, b) => Number(b.strength || 0) - Number(a.strength || 0));
    }
    if (S.error || !rows.length || statusKind(data) !== "live") {
      $("rows").replaceChildren(emptyState());
      return;
    }
    $("rows").replaceChildren(...rows.map((row) => renderCard(row, referencePrice)));
  }

  function render() {
    renderStatus();
    $("sortDistance").classList.toggle("on", S.sort === "distance");
    $("sortStrength").classList.toggle("on", S.sort === "strength");
    renderRows();
  }

  async function boot() {
    $("close").onclick = () => {
      if (tauri) invoke("density_hide").catch(() => window.close());
      else window.close();
    };
    $("docked").checked = S.docked;
    $("docked").onchange = async () => {
      const value = $("docked").checked;
      try {
        await invoke("density_set_docked", { docked: value });
        S.docked = value;
      } catch (err) {
        $("docked").checked = S.docked;
        S.error = String(err);
        render();
      }
    };
    $("sortDistance").onclick = () => { if (!$("sortDistance").disabled) { S.sort = "distance"; render(); } };
    $("sortStrength").onclick = () => { S.sort = "strength"; render(); };

    if (!tauri) {
      S.error = "Desktop density backend is unavailable in browser preview.";
      $("docked").checked = false;
      $("docked").disabled = true;
      render();
      return;
    }
    try {
      const bootstrap = await invoke("bootstrap");
      setPreferences({ theme: bootstrap.settings?.theme, lang: bootstrap.settings?.lang || "en" });
      await Promise.all([
        tauri.event.listen("density_update", ({ payload }) => {
          S.eventCount += 1;
          S.error = null;
          applySnapshot(payload);
        }),
        tauri.event.listen("density_preferences", ({ payload }) => setPreferences(payload)),
      ]);
      const beforeSnapshot = S.eventCount;
      const snapshot = await invoke("density_snapshot");
      if (beforeSnapshot === S.eventCount) {
        S.error = null;
        applySnapshot(snapshot);
      }
    } catch (err) {
      S.error = String(err);
      S.snapshot = { status: "unavailable", broker: null, rows: [], recording: false };
      render();
    }
  }

  setLanguage("en");
  window.setInterval(() => {
    if (S.snapshot) render();
  }, 1000);
  boot();
})();
