"use strict";

// History panel: Dukascopy bid / ask 1-minute candles downloaded once and kept on disk for
// strategy backtests. A date range (optionally a rolling window that Refresh moves forward,
// deleting the oldest days), the timeframes to build, and whether to fetch ask prices.
(() => {
  const A = window.AEGIS;
  if (!A) return;
  const { invoke, log, esc, S } = A;
  const t = (s, v) => window.I18N.t(s, v);
  const $ = (id) => document.getElementById(id);
  // The datafeed usually takes 15-35 s per file; used only for the estimate before a download.
  const SECONDS_PER_FILE = 20;
  const QUICK = [
    ["1y", 1],
    ["3y", 3],
    ["5y", 5],
    ["10y", 10],
  ];

  const H = { info: null, config: null, plan: null, planRefresh: null, planError: "", running: null, progress: null, rate: null };

  const clone = (x) => JSON.parse(JSON.stringify(x));
  const fmtDay = (ts) => new Date(ts * 1000).toISOString().slice(0, 10);
  const fmtTime = (ts) => new Date(ts * 1000).toISOString().slice(0, 16).replace("T", " ");
  const int = (n) => Number(n || 0).toLocaleString("en-US").replace(/,/g, " ");

  function fmtDuration(sec) {
    if (!Number.isFinite(sec) || sec < 0) return "—";
    if (sec < 90) return t("{n} s", { n: Math.round(sec) });
    const m = Math.round(sec / 60);
    if (m < 90) return t("{n} min", { n: m });
    const h = Math.floor(m / 60);
    return h < 48 ? t("{h} h {m} min", { h, m: m % 60 }) : t("{d} days", { d: (h / 24).toFixed(1) });
  }

  function fmtBytes(b) {
    if (!b) return "0";
    const u = ["B", "KB", "MB", "GB"];
    const k = Math.min(u.length - 1, Math.floor(Math.log(b) / Math.log(1024)));
    return `${(b / 1024 ** k).toFixed(k ? 1 : 0)} ${u[k]}`;
  }

  function shiftYears(day, years) {
    const d = new Date(`${day}T00:00:00Z`);
    d.setUTCFullYear(d.getUTCFullYear() - years);
    d.setUTCDate(d.getUTCDate() + 1);
    return d.toISOString().slice(0, 10);
  }

  async function load() {
    H.info = await invoke("history_info");
    if (!H.config) H.config = clone(H.info.config);
    if (H.info.running && !H.running) H.running = "download";
    await plan();
  }

  let planTimer = null;
  function schedulePlan() {
    clearTimeout(planTimer);
    planTimer = setTimeout(() => plan().then(renderEstimate), 250);
  }

  async function plan() {
    try {
      H.plan = await invoke("history_plan", { config: H.config, refresh: false });
      H.planRefresh = H.config.rolling ? await invoke("history_plan", { config: H.config, refresh: true }) : null;
      H.planError = "";
    } catch (e) {
      H.plan = null;
      H.planRefresh = null;
      H.planError = String(e);
    }
  }

  // ---- settings column ------------------------------------------------------------------

  function toggleRow(id, label, on, help) {
    return `<div class="bt-row" title="${esc(help)}"><span class="bt-label">${esc(label)}</span>
      <button type="button" class="toggle${on ? " on" : ""}" data-hs="${id}" ${H.running ? "disabled" : ""}><span class="knob"></span></button></div>
      <p class="hint">${esc(help)}</p>`;
  }

  function settingsHtml() {
    const c = H.config;
    const i = H.info;
    const dis = H.running ? "disabled" : "";
    const quick = QUICK.map(([k, y]) => `<button type="button" class="ghost sm" data-quick="${y}" ${dis}>${esc(t(k))}</button>`).join("");
    const tfs = i.timeframes
      .map((tf) => `<button type="button" class="ghost sm${c.timeframes.includes(tf) ? " on" : ""}" data-tf="${tf}" ${dis}>${tf}</button>`)
      .join("");
    return `
      <details class="bt-group" open><summary>${esc(t("Source"))}</summary>
        <div class="bt-row"><span class="bt-label">${esc(t("Instrument"))}</span><output>XAUUSD</output></div>
        <p class="hint">${esc(t("Dukascopy bid and ask 1-minute candles (UTC), the only free source with years of gold minutes and spreads. The server is slow: about one file per day and price side, often 15-35 s each. A download can be stopped and resumed; files already on disk are never fetched again."))}</p>
      </details>
      <details class="bt-group" open><summary>${esc(t("Date range"))}</summary>
        <div class="hs-dates">
          <label>${esc(t("From"))}<input type="date" class="bt-input" id="hsFrom" value="${esc(c.from)}" min="${esc(i.earliest)}" max="${esc(i.yesterday)}" ${dis}></label>
          <label>${esc(t("To"))}<input type="date" class="bt-input" id="hsTo" value="${esc(c.to)}" min="${esc(i.earliest)}" max="${esc(i.yesterday)}" ${dis}></label>
        </div>
        <div class="bt-presets">${quick}<button type="button" class="ghost sm" data-quick="all" ${dis}>${esc(t("All (from 2003)"))}</button></div>
        ${toggleRow(
          "rolling",
          t("Rolling window"),
          c.rolling,
          t("Refresh keeps the same number of days and moves the window to end yesterday: new days are added and the oldest ones are deleted, so the disk always holds exactly this range."),
        )}
      </details>
      <details class="bt-group" open><summary>${esc(t("Timeframes"))}</summary>
        <div class="bt-presets">${tfs}</div>
        <p class="hint">${esc(t("Every timeframe is built from the same 1-minute candles, so several cost nothing extra to download. Strategies need 1m for exact entry times."))}</p>
      </details>
      <details class="bt-group" open><summary>${esc(t("Prices"))}</summary>
        ${toggleRow("ask", t("Ask prices (spread)"), c.ask, t("Also download ask candles, so backtests can use the real spread at each entry and exit. Doubles the number of files."))}
      </details>
      <div id="hsEstimate">${estimateHtml()}</div>
      <div class="bt-run">
        <button type="button" class="cta" id="hsDownload" ${dis}>${esc(t(H.running === "download" ? "Downloading…" : "Download"))}</button>
        <button type="button" class="ghost" id="hsRefresh" ${dis || !i.manifest ? "disabled" : ""}>${esc(t(H.running === "refresh" ? "Refreshing…" : "Refresh"))}</button>
        ${H.running ? `<button type="button" class="ghost" id="hsCancel">${esc(t("Stop"))}</button>` : ""}
      </div>`;
  }

  function estimateHtml() {
    if (H.planError) return `<p class="hint warn">${esc(t(H.planError))}</p>`;
    const p = H.plan;
    if (!p) return "";
    const lines = [
      t("{d} trading days in the range; {f} files still to download (about {time}).", {
        d: int(p.days),
        f: int(p.files),
        time: fmtDuration(p.files * SECONDS_PER_FILE),
      }),
    ];
    const r = H.planRefresh;
    if (r) lines.push(t("Refresh would keep {from} … {to} and download {f} files.", { from: r.from, to: r.to, f: int(r.files) }));
    return lines.map((l) => `<p class="hint">${esc(l)}</p>`).join("");
  }

  function renderEstimate() {
    const el = $("hsEstimate");
    if (el) el.innerHTML = estimateHtml();
  }

  // ---- status column ----------------------------------------------------------------------

  function progressHtml() {
    const p = H.progress;
    if (!H.running) return "";
    if (!p) return `<div class="bt-empty"><div class="spinner"></div><p>${esc(t("Starting…"))}</p></div>`;
    const frac = p.total ? p.done / p.total : 0;
    let eta = "";
    if (p.stage === "Downloading" && H.rate && p.done > H.rate.done0) {
      const perFile = (performance.now() - H.rate.t0) / 1000 / (p.done - H.rate.done0);
      eta = t("{s} s per file · about {time} left", { s: perFile.toFixed(1), time: fmtDuration(perFile * (p.total - p.done)) });
    }
    return `<div class="bt-sub">${esc(t(p.stage))}: ${esc(p.day)} · ${int(p.done)} / ${int(p.total)}</div>
      <div class="bt-progress"><div class="bar" style="width:${Math.round(frac * 100)}%"></div></div>
      <p class="hint">${esc(eta)}</p>`;
  }

  function statusHtml() {
    const m = H.info.manifest;
    if (!m) {
      return `<div class="bt-empty"><p>${esc(t("No history on disk yet. Choose a range and timeframes, then Download."))}</p></div>`;
    }
    const c = m.config;
    const bars = Object.entries(m.bars)
      .map(([tf, n]) => `<tr><td>${esc(tf)}</td><td>${int(n)}</td></tr>`)
      .join("");
    const cards = [
      [t("Range"), `${c.from} … ${c.to}`, c.rolling ? t("rolling window") : t("fixed range")],
      [t("Data"), m.first ? `${fmtDay(m.first)} … ${fmtDay(m.last)}` : "—", t("{n} days with data", { n: int(m.days) })],
      [t("On disk"), fmtBytes(m.bytes), c.ask ? t("bid + ask") : t("bid only")],
      [t("Updated"), m.updated ? fmtTime(m.updated) : "—", "UTC"],
    ];
    const notes = [];
    if (m.error) notes.push(`<p class="hint warn">${esc(t(m.error))}</p>`);
    if (m.cancelled) notes.push(`<p class="hint warn">${esc(t("The last download was stopped; Refresh or Download continues it."))}</p>`);
    if (m.missing.length) {
      const list = m.missing.slice(0, 40).join(", ") + (m.missing.length > 40 ? " …" : "");
      notes.push(`<p class="hint warn">${esc(t("{n} days failed to download: {list}. Refresh retries them.", { n: m.missing.length, list }))}</p>`);
    }
    if (m.empty_days) notes.push(`<p class="hint">${esc(t("{n} weekdays without data (holidays).", { n: m.empty_days }))}</p>`);
    return `<div class="bt-cards">${cards
      .map(([k, v, sub]) => `<div class="bt-card"><span>${esc(k)}</span><strong>${esc(v)}</strong><em>${esc(sub)}</em></div>`)
      .join("")}</div>
      ${notes.join("")}
      <div class="bt-table"><div class="bt-sub">${esc(t("Bars per timeframe"))}</div><table><tr><th>${esc(t("Timeframe"))}</th><th>${esc(t("Bars"))}</th></tr>${bars}</table></div>
      <p class="hint">${esc(t("Folder"))}: <code>${esc(H.info.folder)}</code></p>`;
  }

  // ---- render and actions ---------------------------------------------------------------

  function render(body) {
    body = body || $("panelBody");
    if (!H.info) {
      body.innerHTML = `<p class="hint">${esc(t("Loading…"))}</p>`;
      load()
        .then(() => S.panel === "history" && render())
        .catch((e) => (body.innerHTML = `<p class="hint warn">${esc(String(e))}</p>`));
      return;
    }
    body.innerHTML = `<div class="bt-cols">
      <div class="bt-col" id="hsSettings">${settingsHtml()}</div>
      <div class="bt-col" id="hsStatus"><div id="hsProgress">${progressHtml()}</div>${statusHtml()}</div></div>`;
    bind(body);
  }

  function setConfig(patch) {
    Object.assign(H.config, patch);
    schedulePlan();
  }

  function bind(body) {
    body.querySelectorAll("[data-hs]").forEach((b) => {
      b.onclick = () => {
        b.classList.toggle("on");
        setConfig({ [b.dataset.hs]: b.classList.contains("on") });
      };
    });
    body.querySelectorAll("[data-tf]").forEach((b) => {
      b.onclick = () => {
        const tf = b.dataset.tf;
        const on = !H.config.timeframes.includes(tf);
        const order = H.info.timeframes;
        const set = on ? [...H.config.timeframes, tf] : H.config.timeframes.filter((x) => x !== tf);
        b.classList.toggle("on", on);
        setConfig({ timeframes: order.filter((x) => set.includes(x)) });
      };
    });
    body.querySelectorAll("[data-quick]").forEach((b) => {
      b.onclick = () => {
        const to = H.info.yesterday;
        const from = b.dataset.quick === "all" ? H.info.earliest : shiftYears(to, Number(b.dataset.quick));
        setConfig({ from, to });
        $("hsFrom").value = from;
        $("hsTo").value = to;
      };
    });
    const from = $("hsFrom");
    const to = $("hsTo");
    if (from) from.onchange = () => setConfig({ from: from.value });
    if (to) to.onchange = () => setConfig({ to: to.value });
    const dl = $("hsDownload");
    if (dl) dl.onclick = () => sync(false);
    const rf = $("hsRefresh");
    if (rf) rf.onclick = () => sync(true);
    const cancel = $("hsCancel");
    if (cancel)
      cancel.onclick = () => {
        invoke("history_cancel");
        cancel.disabled = true;
      };
  }

  async function sync(refresh) {
    if (H.running) return;
    H.running = refresh ? "refresh" : "download";
    H.progress = null;
    H.rate = null;
    if (S.panel === "history") render();
    try {
      const m = await invoke("history_sync", { config: H.config, refresh });
      H.info.manifest = m;
      H.config = clone(m.config);
      const bars = Object.entries(m.bars)
        .map(([tf, n]) => `${tf} ${int(n)}`)
        .join(", ");
      log(t("History: {days} days with data, bars {bars}", { days: m.days, bars }), m.missing.length || m.error ? "warn" : "ok");
      if (m.error) log(`${t("History")}: ${t(m.error)}`, "bad");
      document.dispatchEvent(new CustomEvent("aegis-history"));
    } catch (e) {
      log(`${t("History")}: ${t(String(e))}`, "bad");
    }
    H.running = null;
    H.progress = null;
    await load().catch(() => {});
    if (S.panel === "history") render();
  }

  if (window.__TAURI__) {
    window.__TAURI__.event.listen("history_progress", ({ payload }) => {
      if (payload.stage === "Downloading" && (!H.rate || H.progress?.stage !== "Downloading")) {
        H.rate = { t0: performance.now(), done0: payload.done };
      }
      H.progress = payload;
      const el = $("hsProgress");
      if (el && S.panel === "history") el.innerHTML = progressHtml();
    });
  }

  A.panels.history = { title: "History", render };
})();
