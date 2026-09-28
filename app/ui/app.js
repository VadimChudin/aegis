"use strict";

(() => {
  const $ = (id) => document.getElementById(id);
  const tauri = window.__TAURI__;
  const invoke = (cmd, args) => tauri.core.invoke(cmd, args);
  const t = (str, vars) => window.I18N.t(str, vars);
  const THEMES = [
    ["glass-dark", "Glass dark"],
    ["glass-light", "Glass light"],
    ["glass-blue", "Glass blue"],
  ];

  const S = {
    infos: {}, // broker id → BrokerInfo
    order: [],
    timeframes: [],
    settings: null, // PublicSettings
    sessions: {}, // broker id → AccountSummary
    reports: {}, // broker id → last ConnectReport of this run
    bars: [],
    busy: {},
    openCard: null,
    panel: null,
    chart: { broker: null, tf: "15m", generation: 0, lastTime: 0, wanted: null, request: 0, error: null },
  };

  const esc = (s) =>
    String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);

  // ---- log ------------------------------------------------------------------

  function log(text, kind = "") {
    const row = document.createElement("div");
    row.className = `log-row ${kind}`;
    const time = new Date().toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });
    row.innerHTML = `<span class="log-time">${esc(time)}</span><span class="log-body">${esc(text)}</span>`;
    $("log").prepend(row);
    $("log").scrollTop = 0;
    while ($("log").childElementCount > 200) $("log").lastChild.remove();
  }

  // ---- theme ------------------------------------------------------------------

  function css(name) {
    return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  }

  function applyTheme(theme) {
    document.documentElement.dataset.theme = theme;
    chart.applyOptions({
      layout: { textColor: css("--chart-text") },
      grid: { vertLines: { color: css("--chart-grid") }, horzLines: { color: css("--chart-grid") } },
      rightPriceScale: { borderColor: css("--hair") },
      timeScale: { borderColor: css("--hair") },
    });
    const up = css("--candle-up");
    const down = css("--candle-down");
    candles.applyOptions({ upColor: up, downColor: down, wickUpColor: up, wickDownColor: down });
    volumes.setData(S.bars.map(volBar));
  }

  // ---- chart ------------------------------------------------------------------

  // The chart formats with navigator.language; a POSIX locale such as "C" makes Intl throw.
  function safeLocale() {
    try {
      return Intl.getCanonicalLocales(navigator.language)[0] || "en-US";
    } catch {
      return "en-US";
    }
  }

  const chart = LightweightCharts.createChart($("chart"), {
    autoSize: true,
    localization: { locale: safeLocale() },
    layout: {
      background: { type: "solid", color: "transparent" },
      fontFamily: '-apple-system, BlinkMacSystemFont, "SF Pro Text", "Segoe UI", sans-serif',
    },
    timeScale: { timeVisible: true, secondsVisible: false, rightOffset: 6 },
    crosshair: { mode: LightweightCharts.CrosshairMode.Normal },
  });
  const candles = chart.addCandlestickSeries({ borderVisible: false, priceFormat: { type: "price", precision: 2, minMove: 0.01 } });
  const volumes = chart.addHistogramSeries({
    priceScaleId: "volume",
    priceFormat: { type: "volume" },
    lastValueVisible: false,
    priceLineVisible: false,
  });
  chart.priceScale("volume").applyOptions({ scaleMargins: { top: 0.84, bottom: 0 } });

  // Plain rgba: canvas fill styles do not reliably accept color-mix().
  function withAlpha(color, a) {
    const hex = color.replace("#", "");
    if (!/^[0-9a-f]{6}$/i.test(hex)) return color;
    const n = parseInt(hex, 16);
    return `rgba(${n >> 16}, ${(n >> 8) & 255}, ${n & 255}, ${a})`;
  }
  const volBar = (c) => ({
    time: c.time,
    value: c.volume,
    color: withAlpha(css(c.close >= c.open ? "--candle-up" : "--candle-down"), 0.34),
  });

  function clearChart() {
    candles.setData([]);
    volumes.setData([]);
    S.bars = [];
    S.chart.lastTime = 0;
    $("legendPx").textContent = "";
    $("statLast").textContent = "—";
  }

  function showLast(c) {
    if (!c) return;
    const px = c.close.toFixed(2);
    $("legendPx").textContent = px;
    $("legendPx").className = `px ${c.close >= c.open ? "up" : "down"}`;
    $("statLast").textContent = px;
  }

  function setLamp(kind, text) {
    const lamp = $("feedLamp");
    lamp.dataset.on = kind === "ok" ? "1" : "0";
    lamp.dataset.status = kind === "busy" ? "warming_up" : kind;
    lamp.title = text;
  }

  async function loadChart(broker) {
    if (!broker || !S.sessions[broker]) return;
    if (S.btView) window.AEGIS.bounce?.leaveBacktestView(true);
    const request = ++S.chart.request;
    S.chart.broker = broker;
    const tf = S.chart.tf;
    S.chart.error = null;
    S.chart.generation = -1;
    clearChart();
    renderChartChrome();
    setLamp("busy", t("Loading…"));
    try {
      const data = await invoke("load_chart", { broker, timeframe: tf });
      if (request !== S.chart.request || broker !== S.chart.broker || tf !== S.chart.tf) return;
      S.chart.generation = data.generation;
      S.bars = data.candles.slice();
      candles.setData(data.candles.map(({ volume, ...bar }) => bar));
      volumes.setData(data.candles.map(volBar));
      const last = data.candles[data.candles.length - 1];
      S.chart.lastTime = last ? last.time : 0;
      chart.timeScale().setVisibleLogicalRange({ from: data.candles.length - 160, to: data.candles.length + 6 });
      showLast(last);
      setLamp("ok", `${t("Live")} · ${S.infos[broker].name}`);
      window.AEGIS.bounce?.onLiveChart();
    } catch (err) {
      if (request !== S.chart.request) return;
      if (String(err) === "superseded") return;
      S.chart.error = String(err);
      setLamp("halt", String(err));
      log(`${S.infos[broker].name} chart: ${err}`, "bad");
      renderChartChrome();
    }
  }

  async function stopChart() {
    ++S.chart.request;
    S.chart.broker = null;
    S.chart.error = null;
    S.chart.generation = -1;
    await invoke("stop_chart");
    clearChart();
    setLamp("off", t("Offline"));
    renderChartChrome();
  }

  /** Keeps the chart on a connected broker: the preferred one, else any. */
  async function ensureChart() {
    const connected = S.order.filter((id) => S.sessions[id]);
    if (S.chart.broker && S.sessions[S.chart.broker]) return renderChartChrome();
    const next = [S.chart.wanted, ...connected].find((id) => id && S.sessions[id]);
    if (next) await loadChart(next);
    else await stopChart();
  }

  // ---- custom select (same markup as Vespera) ---------------------------------

  function closeMenus() {
    document.querySelectorAll(".v-menu.open").forEach((m) => m.classList.remove("open"));
  }

  function fillOpt(node, opt) {
    node.textContent = "";
    if (opt.img) {
      const img = document.createElement("img");
      img.className = "opt-ico";
      img.src = opt.img;
      img.alt = "";
      node.appendChild(img);
    }
    const label = document.createElement("span");
    label.textContent = opt.label;
    node.appendChild(label);
  }

  function buildSelect(id, options, value, onChange, { disabled = false, placeholder } = {}) {
    const wrap = document.createElement("div");
    wrap.className = "v-select";
    wrap.id = id;
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "v-select-btn";
    btn.disabled = disabled;
    const face = document.createElement("span");
    face.className = "v-face";
    const current = options.find((o) => o.value === value) || options[0];
    fillOpt(face, current || { label: placeholder || "—" });
    const chev = document.createElement("span");
    chev.className = "chev";
    chev.textContent = "▾";
    btn.append(face, chev);
    const menu = document.createElement("div");
    menu.className = "v-menu";
    for (const opt of options) {
      const item = document.createElement("button");
      item.type = "button";
      item.className = `v-opt${opt.img ? " has-ico" : ""}${opt.value === value ? " on" : ""}`;
      fillOpt(item, opt);
      item.onclick = (e) => {
        e.stopPropagation();
        closeMenus();
        if (opt.value !== value) onChange(opt.value);
      };
      menu.appendChild(item);
    }
    btn.onclick = (e) => {
      e.stopPropagation();
      const open = menu.classList.contains("open");
      closeMenus();
      if (!open && !disabled) menu.classList.add("open");
    };
    wrap.append(btn, menu);
    return wrap;
  }

  // ---- header, dock, rail -------------------------------------------------------

  function brokerState(id) {
    if (S.busy[id]) return ["busy", t("Connecting…")];
    const r = S.reports[id];
    if (S.sessions[id]) return r && !r.ready ? ["warn", t("Connected · check")] : ["on", t("Connected")];
    if (r && !r.connected) return ["err", t("Failed")];
    const stored = S.settings?.brokers?.[id];
    if (stored && (stored.stored.length || Object.keys(stored.values).length)) return ["off", t("Saved")];
    return ["off", t("Not connected")];
  }

  function renderChartChrome() {
    const connected = S.order.filter((id) => S.sessions[id]);
    const opts = connected.map((id) => ({ value: id, label: S.infos[id].name, img: S.infos[id].icon }));
    $("hdrBroker").replaceChildren(
      buildSelect("brokerSel", opts, S.chart.broker, (v) => {
        S.chart.wanted = v;
        loadChart(v);
      }, { disabled: !opts.length, placeholder: t("No broker") }),
    );
    const tfs = S.timeframes.map((tf) => ({ value: tf, label: tf }));
    $("hdrTf").replaceChildren(
      buildSelect("tfSel", tfs, S.chart.tf, (v) => {
        S.chart.tf = v;
        renderChartChrome();
        if (S.chart.broker) loadChart(S.chart.broker);
      }),
    );

    $("brokerDots").replaceChildren(
      ...S.order.map((id) => {
        const [state, label] = brokerState(id);
        const b = document.createElement("button");
        b.type = "button";
        b.className = "broker-dot";
        b.dataset.state = state;
        b.title = `${S.infos[id].name}: ${label}`;
        b.innerHTML = `<img src="${esc(S.infos[id].icon)}" alt="">`;
        b.onclick = () => openPanel("brokers", id);
        return b;
      }),
    );

    const b = S.chart.broker;
    const info = b && S.infos[b];
    const session = b && S.sessions[b];
    $("legendSym").textContent = session ? session.symbol : "XAU";
    $("legendMeta").textContent = info ? `${info.name} · ${S.chart.tf}` : t("No broker");
    $("statBrokers").textContent = `${connected.length} / ${S.order.length}`;
    $("statChart").textContent = info ? `${info.name} · ${session.symbol}` : "—";
    $("btnBrokers").hidden = connected.length > 0;

    const empty = (!b || S.chart.error) && !S.btView;
    $("empty").hidden = !empty;
    if (empty) {
      const busy = S.order.some((id) => S.busy[id]);
      $("emptyTitle").textContent = t(S.chart.error ? "Chart unavailable" : busy ? "Connecting…" : "No broker connected");
      $("emptyText").textContent = S.chart.error
        ? t("The chart could not load. Check your connection and try again.")
        : busy
          ? t("Checking the saved brokers.")
          : t("AEGIS shows gold only from a broker you are connected to: Binance, Bybit or RoboForex.");
      $("emptyConnect").textContent = t(S.chart.error ? "Retry chart" : "Connect a broker");
      $("emptyConnect").hidden = !S.chart.error && busy;
      $("emptyIcons").innerHTML = S.order.map((id) => `<img src="${esc(S.infos[id].icon)}" alt="">`).join("");
    }
  }

  function renderStrategies(list) {
    $("strategyList").replaceChildren(
      ...list.map((s) => {
        const chip = document.createElement("button");
        chip.type = "button";
        const ready = s.status !== "stub";
        chip.className = `strategy-chip${ready ? "" : " soon"}`;
        chip.dataset.strategy = s.id;
        chip.title = ready ? t(s.summary) : `${t(s.summary)} ${t("Coming in a later version.")}`;
        chip.innerHTML = `<span></span><span class="v-opt-tag">${esc(t(ready ? "backtest" : "soon"))}</span>`;
        chip.firstChild.textContent = t(s.name);
        if (ready) chip.onclick = () => openPanel(s.id);
        return chip;
      }),
    );
  }

  // ---- connect flow -------------------------------------------------------------

  async function refreshSessions() {
    const list = await invoke("sessions");
    S.sessions = Object.fromEntries(list.map((s) => [s.broker, s]));
  }

  function logReport(id, r) {
    const name = S.infos[id].name;
    if (r.connected) {
      const warns = r.checks.filter((c) => c.status === "warn").length;
      log(`${name} connected · ${r.account || ""}${warns ? ` · ${warns} warning${warns > 1 ? "s" : ""}` : ""}`, warns ? "warn" : "ok");
    } else {
      const bad = r.checks.find((c) => c.status === "fail");
      log(`${name}: ${bad ? bad.detail || `${bad.label} failed` : "not connected"}`, "bad");
    }
  }

  async function afterConnect(id, report) {
    S.busy[id] = false;
    if (report) {
      S.reports[id] = report;
      logReport(id, report);
    }
    await refreshSessions();
    S.settings = await invoke("settings_get");
    renderChartChrome();
    if (S.panel === "brokers") renderBrokersPanel();
    await ensureChart();
  }

  async function connectSaved(id) {
    S.busy[id] = true;
    renderChartChrome();
    let report = null;
    try {
      report = await invoke("broker_connect_saved", { broker: id });
    } catch (err) {
      log(`${S.infos[id].name}: ${err}`, "bad");
    }
    await afterConnect(id, report);
  }

  async function connectForm(id) {
    const card = document.querySelector(`.broker-card[data-broker="${id}"]`);
    const form = {};
    card.querySelectorAll("input[data-key]").forEach((i) => (form[i.dataset.key] = i.value.trim()));
    const autoConnect = card.querySelector(".toggle").classList.contains("on");
    S.busy[id] = true;
    renderChartChrome();
    renderBrokersPanel(id);
    let report = null;
    try {
      report = await invoke("broker_connect", { broker: id, form, autoConnect });
    } catch (err) {
      log(`${S.infos[id].name}: ${err}`, "bad");
    }
    await afterConnect(id, report);
  }

  async function disconnect(id, forget) {
    S.settings = await invoke("broker_disconnect", { broker: id, forget });
    if (forget) delete S.reports[id];
    log(`${S.infos[id].name} ${forget ? "disconnected, credentials removed" : "disconnected"}`);
    await refreshSessions();
    if (S.chart.broker === id) S.chart.broker = null;
    renderChartChrome();
    if (S.panel === "brokers") renderBrokersPanel();
    await ensureChart();
  }

  // ---- panels -------------------------------------------------------------------

  function openSheet(show) {
    $("sheet").hidden = !show;
    $("backdrop").hidden = !show && $("panel").hidden;
  }

  function closePanel() {
    $("panel").hidden = true;
    $("panel").classList.remove("wide", "strategy");
    $("backdrop").hidden = true;
    $("panelBody").replaceChildren();
    document.querySelectorAll(".strategy-chip").forEach((c) => c.classList.remove("active"));
    S.panel = null;
    document.body.classList.remove("panel-open");
    document.querySelectorAll(".rail-item").forEach((b) => b.classList.toggle("on", b.dataset.panel === "chart"));
  }

  function openPanel(kind, focus) {
    openSheet(false);
    closeMenus();
    S.panel = kind;
    document.body.classList.add("panel-open");
    $("panel").hidden = false;
    $("backdrop").hidden = false;
    document.querySelectorAll(".rail-item").forEach((b) => b.classList.toggle("on", b.dataset.panel === kind));
    if (kind === "brokers") {
      $("panelTitle").textContent = t("Brokers");
      $("panel").classList.add("wide");
      const firstOff = S.order.find((id) => !S.sessions[id]) || S.order[0];
      S.openCard = focus || S.openCard || firstOff;
      renderBrokersPanel();
    } else if (kind === "bounce" && window.AEGIS.bounce) {
      $("panelTitle").textContent = t("Bounce");
      $("panel").classList.add("wide", "strategy");
      document.querySelectorAll(".strategy-chip").forEach((c) => c.classList.toggle("active", c.dataset.strategy === kind));
      window.AEGIS.bounce.render($("panelBody"));
    } else if (kind === "theme") {
      $("panelTitle").textContent = t("Settings");
      $("panel").classList.remove("wide");
      renderThemePanel();
    }
  }

  const ICON = { ok: "\u2713", warn: "!", fail: "\u2715", skip: "\u2013" };

  function checksHtml(r, connected) {
    if (!r) return "";
    const warns = r.checks.filter((c) => c.status === "warn").length;
    const note = warns ? ` · ${t(warns > 1 ? "{n} warnings" : "{n} warning", { n: warns })}` : "";
    const head = r.ready ? `${t("Ready")}${note}` : r.connected ? t("Connected, see the checks") : t(connected ? "Reconnect failed · previous connection still active" : "Not connected");
    const when = r.at ? new Date(r.at).toLocaleString() : "";
    const rows = r.checks
      .map(
        (c) => `<div class="chk chk-${esc(c.status)}"><span class="chk-ico">${ICON[c.status] || ""}</span><div>
          <div>${esc(t(c.label))}</div>${c.detail ? `<div class="hint">${esc(t(c.detail))}</div>` : ""}</div></div>`,
      )
      .join("");
    return `<div class="chk-head" data-ready="${r.ready ? "1" : "0"}">${esc(head)}<span class="hint">${esc(when)}</span></div>${rows}`;
  }

  function renderBrokersPanel(submittedId) {
    const body = $("panelBody");
    const scroll = body.scrollTop;
    const drafts = new Map();
    let focused = null;
    body.querySelectorAll(".broker-card").forEach((card) => {
      if (card.dataset.broker === submittedId) return;
      drafts.set(card.dataset.broker, {
        values: [...card.querySelectorAll("input[data-key]")].map((input) => input.value),
        auto: card.querySelector(".toggle").classList.contains("on"),
      });
      if (card.contains(document.activeElement) && document.activeElement.matches("input[data-key]")) {
        focused = [card.dataset.broker, document.activeElement.dataset.key];
      }
    });
    const intro = `<p class="hint">${esc(t("Connect one or more brokers; they stay connected together. The chart shows gold from the broker picked in the header. Credentials are stored encrypted on this computer, and secrets are never shown again."))}</p>`;
    body.innerHTML = intro + S.order.map(cardHtml).join("");
    body.querySelectorAll(".broker-card").forEach((card) => {
      const draft = drafts.get(card.dataset.broker);
      if (draft) {
        card.querySelectorAll("input[data-key]").forEach((input, i) => (input.value = draft.values[i]));
        card.querySelector(".toggle").classList.toggle("on", draft.auto);
      }
      bindCard(card);
    });
    body.scrollTop = scroll;
    if (focused) {
      const input = [...body.querySelectorAll(".broker-card input[data-key]")].find(
        (node) => node.closest(".broker-card").dataset.broker === focused[0] && node.dataset.key === focused[1],
      );
      input?.focus({ preventScroll: true });
    }
  }

  function cardHtml(id) {
    const info = S.infos[id];
    const saved = S.settings.brokers[id];
    const [state, label] = brokerState(id);
    const open = S.openCard === id;
    const report = S.reports[id] || saved.last_report;
    const fields = info.fields
      .map((f, i) => {
        const stored = f.secret && saved.stored.includes(f.key);
        const value = f.secret ? "" : saved.values[f.key] || "";
        const ph = stored ? t("Stored · leave empty to keep") : f.placeholder || "";
        const input = `<input data-key="${esc(f.key)}" type="${f.secret ? "password" : "text"}" value="${esc(value)}" placeholder="${esc(ph)}" autocomplete="off" spellcheck="false" />`;
        const row = i === 0 ? `<div class="api-row"><img class="api-mark" src="${esc(info.icon)}" alt="">${input}</div>` : input;
        return `<div class="field"><label>${esc(t(f.label))}${f.optional ? ` (${esc(t("optional"))})` : ""}</label>${row}<p class="hint">${esc(t(f.hint))}</p></div>`;
      })
      .join("");
    const connected = !!S.sessions[id];
    const hasSaved = saved.stored.length || Object.keys(saved.values).length;
    const auto = saved.auto_connect || !hasSaved;
    return `
      <section class="broker-card${open ? " open" : ""}" data-broker="${id}">
        <button class="broker-head" type="button">
          <img class="broker-ico" src="${esc(info.icon)}" alt="">
          <div class="broker-title"><strong>${esc(info.name)}</strong><span>${esc(info.venue)} · ${esc(info.symbol)}${
            S.sessions[id] ? ` · ${esc(S.sessions[id].account)}` : ""
          }</span></div>
          <span class="bstate" data-state="${state}">${esc(label)}</span>
          <span class="chev">▾</span>
        </button>
        <div class="broker-body">
          <div class="req-title">${esc(t("Requirements"))}</div>
          <ul class="req-list">${info.requirements.map((r) => `<li>${esc(t(r))}</li>`).join("")}</ul>
          <div class="keys-url">${esc(info.keys_url)}</div>
          ${saved.unreadable ? `<p class="hint warn">${esc(t("Saved credentials could not be decrypted on this computer. Enter them again."))}</p>` : ""}
          ${fields}
          <div class="field toggle-row"><span class="field-label">${esc(t("Connect on start"))}</span>
            <button type="button" class="toggle${auto ? " on" : ""}" aria-label="${esc(t("Connect on start"))}"><span class="knob"></span></button></div>
          <div class="btn-row">
            <button type="button" class="ok act-connect" ${S.busy[id] ? "disabled" : ""}>${esc(t(S.busy[id] ? "Connecting…" : connected ? "Reconnect" : "Connect"))}</button>
            ${connected ? `<button type="button" class="ghost act-disconnect">${esc(t("Disconnect"))}</button>` : ""}
            ${hasSaved ? `<button type="button" class="ghost danger act-forget">${esc(t("Forget"))}</button>` : ""}
          </div>
          <div class="checks">${checksHtml(report, connected)}</div>
        </div>
      </section>`;
  }

  function bindCard(card) {
    const id = card.dataset.broker;
    card.querySelector(".broker-head").onclick = () => {
      S.openCard = S.openCard === id ? null : id;
      document.querySelectorAll(".broker-card").forEach((c) => c.classList.toggle("open", c.dataset.broker === S.openCard));
    };
    const toggle = card.querySelector(".toggle");
    toggle.onclick = async () => {
      toggle.classList.toggle("on");
      if (S.settings.brokers[id].stored.length) await invoke("set_auto_connect", { broker: id, on: toggle.classList.contains("on") });
    };
    card.querySelector(".act-connect").onclick = () => connectForm(id);
    card.querySelectorAll("input").forEach((i) =>
      i.addEventListener("keydown", (e) => {
        if (e.key === "Enter" && !S.busy[id]) connectForm(id);
      }),
    );
    const d = card.querySelector(".act-disconnect");
    if (d) d.onclick = () => disconnect(id, false);
    // Two clicks instead of confirm(): native dialogs are not available in every webview.
    const f = card.querySelector(".act-forget");
    if (f) {
      f.onclick = () => {
        if (f.dataset.armed) return disconnect(id, true);
        f.dataset.armed = "1";
        f.textContent = t("Click again to forget");
        setTimeout(() => {
          delete f.dataset.armed;
          f.textContent = t("Forget");
        }, 3000);
      };
    }
  }

  function renderThemePanel() {
    const current = S.settings.theme;
    const lang = window.I18N.lang;
    $("panelBody").innerHTML = `<div class="field"><label>${esc(t("Language"))}</label></div><div class="lang-row" id="langRow">${window.I18N.LANGS.map(
      ([id, name]) => `<button type="button" class="ghost sm${id === lang ? " on" : ""}" data-lang="${id}">${esc(name)}</button>`,
    ).join("")}</div>
      <div class="field"><label>${esc(t("Theme"))}</label></div><div class="theme-grid" id="themeGrid">${THEMES.map(
      ([id, name]) => `<button type="button" class="theme-card${id === current ? " on" : ""}" data-theme="${id}">
        <span class="theme-swatch" data-swatch="${id}"><i></i><i></i><i></i></span><span class="theme-name">${esc(t(name))}</span></button>`,
    ).join("")}</div><p class="hint">AEGIS v${esc(S.version)} · ${esc(t("settings are stored on this computer."))}</p>`;
    $("themeGrid").querySelectorAll(".theme-card").forEach((b) => {
      b.onclick = async () => {
        S.settings.theme = b.dataset.theme;
        applyTheme(b.dataset.theme);
        $("themeGrid").querySelectorAll(".theme-card").forEach((x) => x.classList.toggle("on", x === b));
        await invoke("set_theme", { theme: b.dataset.theme });
      };
    });
    $("langRow").querySelectorAll("[data-lang]").forEach((b) => {
      b.onclick = async () => {
        await setLang(b.dataset.lang, true);
        renderThemePanel();
      };
    });
  }

  /** Switches the interface language and redraws everything that holds text. */
  async function setLang(lang, persist) {
    window.I18N.set(lang);
    S.settings.lang = window.I18N.lang;
    if (persist) await invoke("set_lang", { lang: window.I18N.lang });
    renderStrategies(S.strategies || []);
    renderChartChrome();
    if (S.panel === "brokers") renderBrokersPanel();
    if (S.panel === "bounce" && window.AEGIS.bounce) window.AEGIS.bounce.render($("panelBody"));
    const titles = { brokers: "Brokers", bounce: "Bounce", theme: "Settings" };
    if (S.panel && titles[S.panel]) $("panelTitle").textContent = t(titles[S.panel]);
  }

  // ---- live feed ----------------------------------------------------------------

  function listenFeed() {
    tauri.event.listen("candle", ({ payload }) => {
      if (payload.generation !== S.chart.generation) return;
      const c = payload.candle;
      // The feed resends the previous bar; the chart only accepts the newest one.
      if (c.time < S.chart.lastTime) return;
      const { volume, ...bar } = c;
      candles.update(bar);
      volumes.update(volBar(c));
      if (S.bars.length && S.bars[S.bars.length - 1].time === c.time) S.bars[S.bars.length - 1] = c;
      else S.bars.push(c);
      S.chart.lastTime = c.time;
      showLast(c);
    });
    tauri.event.listen("feed_status", ({ payload }) => {
      if (payload.generation !== S.chart.generation) return;
      setLamp(payload.ok ? "ok" : "halt", payload.message);
      if (!payload.ok) log(`Feed: ${payload.message}`, "bad");
    });
  }

  // Shared with strategy panels (bounce.js).
  window.AEGIS = {
    invoke,
    log,
    esc,
    S,
    chart,
    candles,
    volumes,
    clearChart,
    setLamp,
    closePanel,
    renderChartChrome,
    reloadLive: () => (S.chart.broker ? loadChart(S.chart.broker) : (clearChart(), setLamp("off", t("Offline")), renderChartChrome())),
  };

  // ---- boot ---------------------------------------------------------------------

  async function boot() {
    if (!tauri) {
      setLamp("halt", "Open AEGIS through the desktop app");
      return;
    }
    window.addEventListener("error", (e) => log(`UI error: ${e.message}`, "bad"));
    window.addEventListener("unhandledrejection", (e) => log(`UI error: ${e.reason}`, "bad"));

    const b = await invoke("bootstrap");
    S.version = b.version;
    S.order = b.brokers.map((x) => x.id);
    S.infos = Object.fromEntries(b.brokers.map((x) => [x.id, x]));
    S.timeframes = b.timeframes;
    S.settings = b.settings;
    S.sessions = Object.fromEntries(b.sessions.map((s) => [s.broker, s]));
    S.chart.tf = b.settings.timeframe || "15m";
    S.chart.wanted = b.settings.chart_broker;
    applyTheme(b.settings.theme);
    S.strategies = b.strategies;
    window.I18N.set(b.settings.lang || window.I18N.detect());
    renderStrategies(b.strategies);
    listenFeed();

    document.addEventListener("click", closeMenus);
    document.addEventListener("keydown", (e) => {
      if (e.key === "Escape") {
        closeMenus();
        closePanel();
        openSheet(false);
      }
    });
    $("btnMenu").onclick = () => openSheet($("sheet").hidden);
    $("sheetClose").onclick = () => openSheet(false);
    $("backdrop").onclick = () => {
      openSheet(false);
      closePanel();
    };
    $("panelClose").onclick = closePanel;
    $("btnBrokers").onclick = () => openPanel("brokers");
    $("emptyConnect").onclick = () => (S.chart.error ? loadChart(S.chart.broker) : openPanel("brokers"));
    document.querySelectorAll("[data-open]").forEach((b) => (b.onclick = () => openPanel(b.dataset.open)));
    document.querySelectorAll(".rail-item").forEach((b) => {
      b.onclick = () => (b.dataset.panel === "chart" ? closePanel() : openPanel(b.dataset.panel));
    });

    const saved = S.order.filter((id) => {
      const s = S.settings.brokers[id];
      return s.auto_connect && (s.stored.length || Object.keys(s.values).length) && !S.sessions[id];
    });
    saved.forEach((id) => (S.busy[id] = true));
    renderChartChrome();
    log(`AEGIS ${b.version}${saved.length ? ` · ${t("connecting")} ${saved.map((id) => S.infos[id].name).join(", ")}` : ""}`);
    await ensureChart();
    await Promise.all(saved.map(connectSaved));
  }

  boot().catch((err) => log(`Start failed: ${err}`, "bad"));
})();
