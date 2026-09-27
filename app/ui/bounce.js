"use strict";

// Bounce strategy panel: sliders and toggles, backtest, and chart overlays
// (expected entries with probability on the live chart; signals and trades in backtest view).
(() => {
  const A = window.AEGIS;
  if (!A) return;
  const { invoke, log, esc, S } = A;
  const $ = (id) => document.getElementById(id);

  const PRESETS = {
    binance: { label: "70% · Binance costs", costs: { maker_bps: 0, taker_bps: 4, slippage: 0.05 } },
    roboforex: { label: "70% · RoboForex costs", costs: { maker_bps: 0.2, taker_bps: 0.2, slippage: 0.15 } },
  };
  const GROUP_ORDER = ["Probability", "Trade", "Costs", "Direction", "Levels", "Sessions", "Touch"];
  const KEY_METRICS = 10;
  const LIVE_EVERY_MS = 5 * 60 * 1000;

  const B = {
    info: null,
    params: null,
    result: null,
    running: false,
    progress: "",
    lines: [],
    liveTimer: null,
    live: null,
    selected: null,
    open: { Probability: true, Trade: true, "Key metrics": true },
  };

  const clone = (x) => JSON.parse(JSON.stringify(x));
  const pct = (x, d = 1) => (Number.isFinite(x) ? `${(x * 100).toFixed(d)}%` : "—");
  const num = (x, d = 2) => (Number.isFinite(x) ? x.toFixed(d) : "—");
  const fmtDate = (t) => new Date(t * 1000).toISOString().slice(0, 16).replace("T", " ");

  // ---- params by spec id ("long", "kinds.swing5", "scan.zone_atr", "filters.wick") ----

  function getParam(id) {
    const p = B.params;
    if (id === "entry") return p.entry === "close";
    const [head, key] = id.split(".");
    if (!key) return p[head];
    if (head === "filters") return p.filters[key];
    return p[head]?.[key];
  }

  function setParam(id, value) {
    const p = B.params;
    if (id === "entry") p.entry = value ? "close" : "limit";
    else {
      const [head, key] = id.split(".");
      if (!key) p[head] = value;
      else {
        p[head] = p[head] || {};
        p[head][key] = value;
      }
    }
    save();
  }

  let saveTimer = null;
  function save() {
    clearTimeout(saveTimer);
    saveTimer = setTimeout(() => invoke("bounce_save", { params: B.params }).catch((e) => log(`Bounce: ${e}`, "bad")), 400);
    scheduleLive(1500);
  }

  async function ensureInfo() {
    if (B.info) return;
    B.info = await invoke("bounce_info");
    B.params = clone(B.info.saved || B.info.defaults);
  }

  function applyPreset(key) {
    B.params = clone(B.info.defaults);
    if (key !== "reset") Object.assign(B.params, PRESETS[key].costs);
    save();
    render();
  }

  // ---- rendering ------------------------------------------------------------------

  function researchBars(featureId) {
    const r = B.info.research?.metrics?.[featureId];
    if (!r) return "";
    const base = B.info.research.base;
    const cells = r.bins
      .map((b) => {
        const diff = b.win - base;
        const cls = diff > 0.02 ? "up" : diff < -0.02 ? "down" : "";
        const h = Math.max(4, Math.min(100, 50 + diff * 250));
        return `<i class="${cls}" style="height:${h}%" title="${esc(`${num(b.lo, 3)} … ${num(b.hi, 3)}: win ${pct(b.win)} (n=${b.n})`)}"></i>`;
      })
      .join("");
    return `<div class="rq" title="Win rate by quintile in the research run (base ${pct(base)})">${cells}</div>`;
  }

  function stepDigits(step) {
    const s = String(step);
    return s.includes(".") ? s.split(".")[1].length : 0;
  }

  function toggleRow(spec) {
    const on = !!getParam(spec.id);
    return `<div class="bt-row" title="${esc(spec.help)}"><span class="bt-label">${esc(spec.label)}</span>
      <button type="button" class="toggle${on ? " on" : ""}" data-id="${esc(spec.id)}" data-kind="toggle"><span class="knob"></span></button></div>`;
  }

  function sliderRow(spec) {
    const v = Number(getParam(spec.id));
    const d = stepDigits(spec.step);
    return `<div class="bt-row slider" title="${esc(spec.help)}"><span class="bt-label">${esc(spec.label)}</span>
      <input type="range" min="${spec.lo}" max="${spec.hi}" step="${spec.step}" value="${v}" data-id="${esc(spec.id)}" data-kind="slider" data-digits="${d}">
      <output>${v.toFixed(d)}</output></div>`;
  }

  function filterRow(spec) {
    const id = spec.id.slice("filters.".length);
    const f = getParam(spec.id) || { on: false, min: spec.lo, max: spec.hi };
    const d = stepDigits(spec.step);
    const inModel = B.info.model_features.includes(id);
    return `<div class="bt-filter${f.on ? " on" : ""}" data-id="${esc(spec.id)}" title="${esc(spec.help)}">
      <div class="bt-row"><span class="bt-label">${esc(spec.label)}${inModel ? ` <span class="tag">model</span>` : ""}</span>
        <output>${f.on ? `${Number(f.min).toFixed(d)} … ${Number(f.max).toFixed(d)}` : "any"}</output>
        <button type="button" class="toggle${f.on ? " on" : ""}" data-kind="filter-on"><span class="knob"></span></button></div>
      <div class="bt-range">
        <input type="range" min="${spec.lo}" max="${spec.hi}" step="${spec.step}" value="${Math.max(spec.lo, f.min)}" data-kind="filter-min" data-digits="${d}">
        <input type="range" min="${spec.lo}" max="${spec.hi}" step="${spec.step}" value="${Math.min(spec.hi, f.max)}" data-kind="filter-max" data-digits="${d}">
      </div>
      ${researchBars(id)}
    </div>`;
  }

  function group(name, rows) {
    const open = B.open[name] ? " open" : "";
    return `<details class="bt-group"${open} data-group="${esc(name)}"><summary>${esc(name)}</summary>${rows}</details>`;
  }

  function settingsHtml() {
    const specs = B.info.specs;
    const html = [];
    html.push(
      `<div class="bt-presets">${Object.entries(PRESETS)
        .map(([k, p]) => `<button type="button" class="ghost sm" data-preset="${k}">${esc(p.label)}</button>`)
        .join("")}<button type="button" class="ghost sm" data-preset="reset">Reset</button></div>`,
    );
    for (const g of GROUP_ORDER) {
      const rows = specs
        .filter((s) => s.group === g && s.kind !== "filter")
        .map((s) => (s.kind === "toggle" ? toggleRow(s) : sliderRow(s)))
        .join("");
      if (rows) html.push(group(g, rows));
    }
    const filters = specs.filter((s) => s.kind === "filter");
    const spread = (s) => B.info.research?.metrics?.[s.id.slice(8)]?.spread ?? 0;
    const key = [...filters].sort((a, b) => spread(b) - spread(a)).slice(0, KEY_METRICS);
    const note = `<p class="hint">Bars: win rate by quintile in research (${esc(B.info.research?.label || "")}). ${
      B.params.entry === "close" ? "Metrics of the touch bar." : "Metrics as of the bar before the touch (what a resting limit order can know)."
    }</p>`;
    html.push(group("Key metrics", note + key.map(filterRow).join("")));
    const byGroup = {};
    for (const f of filters) if (!key.includes(f)) (byGroup[f.group] = byGroup[f.group] || []).push(f);
    for (const [g, list] of Object.entries(byGroup)) html.push(group(`Metrics · ${g}`, list.map(filterRow).join("")));
    return html.join("");
  }

  function statsHtml(s) {
    const cards = [
      ["Trades", s.trades, `${num(s.trades_per_week, 1)} / week`],
      ["Win rate", pct(s.win_rate), `95% low ${pct(s.win_rate_lo)}`],
      ["Avg R", num(s.avg_r, 3), `win ${num(s.avg_win_r)} · loss ${num(s.avg_loss_r)}`],
      ["Total R", num(s.total_r, 1), `PF ${num(s.profit_factor)}`],
      ["Max drawdown", `${num(s.max_dd_r, 1)} R`, `worst streak ${s.max_losing_streak}`],
      ["Touches", s.touches, "levels touched"],
    ];
    return `<div class="bt-cards">${cards
      .map(([k, v, sub]) => `<div class="bt-card"><span>${esc(k)}</span><strong>${esc(v)}</strong><em>${esc(sub)}</em></div>`)
      .join("")}</div>`;
  }

  function equityHtml(eq) {
    if (!eq.length) return "";
    const w = 560;
    const h = 120;
    const ys = eq.map((p) => p[1]);
    const lo = Math.min(0, ...ys);
    const hi = Math.max(0, ...ys);
    const x = (i) => (i / Math.max(1, eq.length - 1)) * w;
    const y = (v) => h - ((v - lo) / Math.max(1e-9, hi - lo)) * h;
    const d = eq.map((p, i) => `${i ? "L" : "M"}${x(i).toFixed(1)},${y(p[1]).toFixed(1)}`).join("");
    return `<svg class="bt-equity" viewBox="0 0 ${w} ${h}" preserveAspectRatio="none"><line x1="0" x2="${w}" y1="${y(0)}" y2="${y(0)}"/><path d="${d}"/></svg>`;
  }

  function tableHtml(title, map, labels = {}) {
    const rows = Object.entries(map)
      .map(
        ([k, s]) =>
          `<tr><td>${esc(labels[k] || k)}</td><td>${s.trades}</td><td class="${s.win_rate >= 0.7 ? "pos" : ""}">${pct(s.win_rate)}</td><td class="${
            s.total_r >= 0 ? "pos" : "neg"
          }">${num(s.total_r, 1)}</td><td>${num(s.avg_r, 3)}</td></tr>`,
      )
      .join("");
    return `<div class="bt-table"><div class="bt-sub">${esc(title)}</div><table><tr><th></th><th>Trades</th><th>Win</th><th>R</th><th>Avg R</th></tr>${rows}</table></div>`;
  }

  function tradesHtml(trades) {
    const last = trades.slice(-150).reverse();
    const rows = last
      .map((t) => {
        const i = trades.indexOf(t);
        return `<tr data-trade="${i}" class="${B.selected === i ? "sel" : ""}"><td>${esc(fmtDate(t.entry_time))}</td><td class="${t.dir > 0 ? "pos" : "neg"}">${
          t.dir > 0 ? "Long" : "Short"
        }</td><td>${num(t.entry)}</td><td>${pct(t.prob, 0)}</td><td>${esc(t.outcome)}</td><td class="${t.r > 0 ? "pos" : "neg"}">${num(t.r)}</td></tr>`;
      })
      .join("");
    return `<div class="bt-table"><div class="bt-sub">Trades (latest 150, click to show on chart)</div><table class="trades"><tr><th>Entry (UTC)</th><th>Side</th><th>Price</th><th>Prob.</th><th>Exit</th><th>R</th></tr>${rows}</table></div>`;
  }

  function resultsHtml() {
    if (B.running) return `<div class="bt-empty"><div class="spinner"></div><p>${esc(B.progress || "Running…")}</p></div>`;
    const r = B.result?.report;
    if (!r) {
      return `<div class="bt-empty"><p>Press <b>Run backtest</b>. AEGIS downloads Binance XAUUSDT 5m history since the listing (public archive, no key), finds every level touch, trains the probability model month by month on the months before, and simulates the trades with costs.</p></div>`;
    }
    const s = r.stats;
    const kinds = Object.fromEntries(B.info.kinds);
    const verdict =
      s.trades < 30
        ? `<p class="hint warn">Only ${s.trades} trades: too few to trust.</p>`
        : `<p class="hint">${esc(B.result.source)} · ${esc(fmtDate(r.from))} → ${esc(fmtDate(r.to))} UTC · ${r.bars} bars. Probabilities are out of sample (model trained on earlier months only); the first 2 months only train the model.</p>`;
    return `${statsHtml(s)}${verdict}${equityHtml(r.equity)}
      <div class="bt-btns"><button type="button" class="ghost sm" id="btShow">Show on chart</button></div>
      ${tableHtml("By month", r.by_month)}${tableHtml("By session", r.by_session)}${tableHtml("By level kind", r.by_kind, kinds)}${tradesHtml(r.trades)}`;
  }

  function render(body) {
    body = body || $("panelBody");
    if (!B.info) {
      body.innerHTML = `<p class="hint">Loading…</p>`;
      ensureInfo()
        .then(() => S.panel === "bounce" && render())
        .catch((e) => (body.innerHTML = `<p class="hint warn">${esc(e)}</p>`));
      return;
    }
    body.innerHTML = `<div class="bt-cols">
      <div class="bt-col" id="btSettings">${settingsHtml()}</div>
      <div class="bt-col">
        <div class="bt-run"><button type="button" class="cta" id="btRun" ${B.running ? "disabled" : ""}>${B.running ? "Running…" : "Run backtest"}</button>
          <span class="hint">No orders are sent: backtest and expected entries only.</span></div>
        <div id="btResults">${resultsHtml()}</div>
      </div></div>`;
    bind(body);
  }

  function renderResults() {
    const el = $("btResults");
    if (!el) return;
    el.innerHTML = resultsHtml();
    const run = $("btRun");
    if (run) {
      run.disabled = B.running;
      run.textContent = B.running ? "Running…" : "Run backtest";
    }
    bindResults();
  }

  function bind(body) {
    body.querySelectorAll("details.bt-group").forEach((d) => d.addEventListener("toggle", () => (B.open[d.dataset.group] = d.open)));
    body.querySelectorAll("[data-preset]").forEach((b) => (b.onclick = () => applyPreset(b.dataset.preset)));
    body.querySelectorAll('[data-kind="toggle"]').forEach((t) => {
      t.onclick = () => {
        t.classList.toggle("on");
        setParam(t.dataset.id, t.classList.contains("on"));
        if (t.dataset.id === "entry") render();
      };
    });
    body.querySelectorAll('[data-kind="slider"]').forEach((r) => {
      r.oninput = () => {
        const v = Number(r.value);
        r.nextElementSibling.textContent = v.toFixed(Number(r.dataset.digits));
        setParam(r.dataset.id, r.dataset.id === "max_bars" || r.dataset.id === "scan.swing_n" ? Math.round(v) : v);
      };
    });
    body.querySelectorAll(".bt-filter").forEach((row) => {
      const id = row.dataset.id;
      const [on, lo, hi] = ["filter-on", "filter-min", "filter-max"].map((k) => row.querySelector(`[data-kind="${k}"]`));
      const out = row.querySelector("output");
      const d = Number(lo.dataset.digits);
      const update = (enable) => {
        let a = Number(lo.value);
        let b = Number(hi.value);
        if (a > b) [a, b] = [b, a];
        const f = { on: enable ?? on.classList.contains("on"), min: a, max: b };
        on.classList.toggle("on", f.on);
        row.classList.toggle("on", f.on);
        out.textContent = f.on ? `${a.toFixed(d)} … ${b.toFixed(d)}` : "any";
        setParam(id, f);
      };
      on.onclick = () => update(!on.classList.contains("on"));
      lo.oninput = () => update(true);
      hi.oninput = () => update(true);
    });
    $("btRun").onclick = runBacktest;
    bindResults();
  }

  function bindResults() {
    const show = $("btShow");
    if (show) show.onclick = () => enterBacktestView();
    document.querySelectorAll("#btResults tr[data-trade]").forEach((tr) => {
      tr.onclick = () => {
        B.selected = Number(tr.dataset.trade);
        enterBacktestView(B.selected);
      };
    });
  }

  // ---- backtest -------------------------------------------------------------------

  async function runBacktest() {
    if (B.running) return;
    B.running = true;
    B.progress = "Loading history…";
    B.selected = null;
    renderResults();
    const t0 = performance.now();
    try {
      B.result = await invoke("bounce_backtest", { params: B.params });
      const s = B.result.report.stats;
      log(
        `Bounce backtest: ${s.trades} trades, win ${pct(s.win_rate)}, avg ${num(s.avg_r, 3)} R, total ${num(s.total_r, 1)} R, PF ${num(s.profit_factor)} · ${(
          (performance.now() - t0) /
          1000
        ).toFixed(1)} s`,
        s.avg_r > 0 ? "ok" : "warn",
      );
    } catch (e) {
      log(`Bounce backtest: ${e}`, "bad");
    }
    B.running = false;
    renderResults();
  }

  function clearLines() {
    for (const l of B.lines) A.candles.removePriceLine(l);
    B.lines = [];
  }

  function line(price, color, title, style = 0) {
    B.lines.push(A.candles.createPriceLine({ price, color, lineWidth: 1, lineStyle: style, axisLabelVisible: true, title }));
  }

  const css = (n) => getComputedStyle(document.documentElement).getPropertyValue(n).trim();

  function enterBacktestView(selected) {
    const res = B.result;
    if (!res) return;
    S.btView = true;
    S.chart.generation = -1; // drop live feed events while the backtest is shown
    A.closePanel();
    clearLines();
    const bars = res.bars.map(([time, open, high, low, close]) => ({ time, open, high, low, close }));
    A.candles.setData(bars);
    A.volumes.setData([]);
    const up = css("--mark-up");
    const down = css("--mark-down");
    const win = css("--mark-win");
    const loss = css("--mark-loss");
    const muted = css("--muted");
    const markers = [];
    for (const s of res.report.signals) {
      if (s.taken) continue;
      markers.push({ time: s.time, position: s.dir > 0 ? "belowBar" : "aboveBar", color: muted, shape: "circle", size: 0.4 });
    }
    for (const t of res.report.trades) {
      markers.push({
        time: t.entry_time,
        position: t.dir > 0 ? "belowBar" : "aboveBar",
        color: t.dir > 0 ? up : down,
        shape: t.dir > 0 ? "arrowUp" : "arrowDown",
        text: pct(t.prob, 0),
      });
      markers.push({
        time: t.exit_time - 300,
        position: t.dir > 0 ? "aboveBar" : "belowBar",
        color: t.r > 0 ? win : loss,
        shape: "circle",
        text: `${t.r > 0 ? "+" : ""}${t.r.toFixed(2)}R`,
      });
    }
    markers.sort((a, b) => a.time - b.time);
    A.candles.setMarkers(markers);
    const idx = (time) => {
      let lo = 0;
      let hi = bars.length - 1;
      while (lo < hi) {
        const m = (lo + hi) >> 1;
        if (bars[m].time < time) lo = m + 1;
        else hi = m;
      }
      return lo;
    };
    if (Number.isInteger(selected)) {
      const t = res.report.trades[selected];
      line(t.entry, t.dir > 0 ? up : down, `${t.dir > 0 ? "Long" : "Short"} ${pct(t.prob, 0)}`);
      line(t.sl, loss, "SL", 2);
      line(t.tp, win, "TP", 2);
      const i = idx(t.entry_time);
      A.chart.timeScale().setVisibleLogicalRange({ from: i - 80, to: i + 60 });
    } else {
      A.chart.timeScale().setVisibleLogicalRange({ from: bars.length - 300, to: bars.length + 6 });
    }
    const s = res.report.stats;
    $("btBar").hidden = false;
    $("btBarText").textContent = `Backtest · Binance XAUUSDT 5m · ${s.trades} trades · win ${pct(s.win_rate)} · ${num(s.total_r, 1)} R`;
    $("legendSym").textContent = "XAUUSDT";
    $("legendMeta").textContent = "Backtest · 5m";
    A.setLamp("busy", "Backtest view");
    A.renderChartChrome();
    $("legendSym").textContent = "XAUUSDT";
    $("legendMeta").textContent = "Backtest · 5m";
  }

  function leaveBacktestView(silent) {
    if (!S.btView) return;
    S.btView = false;
    $("btBar").hidden = true;
    A.candles.setMarkers([]);
    clearLines();
    if (!silent) A.reloadLive();
  }

  // ---- expected entries on the live chart --------------------------------------------

  function scheduleLive(delay = 0) {
    clearTimeout(B.liveTimer);
    if (!B.info || !S.chart.broker || S.btView) return;
    B.liveTimer = setTimeout(refreshLive, delay);
  }

  async function refreshLive() {
    if (!S.chart.broker || S.btView || !B.params) return;
    try {
      const res = await invoke("bounce_live", { params: B.params });
      if (S.btView || !S.chart.broker) return;
      B.live = res;
      clearLines();
      // Levels and prices come from Binance XAUUSDT; other venues quote gold with a basis.
      if (S.chart.broker !== "binance") {
        const note = "Bounce: expected entries are computed on Binance XAUUSDT; pick Binance in the header to see them on the chart.";
        if (note !== B.lastSummary) log(note);
        B.lastSummary = note;
        scheduleLive(LIVE_EVERY_MS);
        return;
      }
      const ok = res.live.entries.filter((e) => e.ok);
      for (const e of ok) {
        line(e.entry, e.dir > 0 ? css("--mark-up") : css("--mark-down"), `Bounce ${e.dir > 0 ? "buy" : "sell"} ${pct(e.prob, 0)}`, 1);
      }
      const best = Math.max(0, ...res.live.entries.map((e) => (Number.isFinite(e.prob) ? e.prob : 0)));
      const summary = `Bounce: ${ok.length} expected entr${ok.length === 1 ? "y" : "ies"} on the chart · ${res.live.entries.length} armed levels near price, best ${pct(best, 0)} (threshold ${pct(B.params.min_prob, 0)})`;
      if (summary !== B.lastSummary) log(summary, ok.length ? "ok" : "");
      B.lastSummary = summary;
      if (!res.fresh) log(`Bounce: ${res.note}`, "warn");
    } catch (e) {
      log(`Bounce expected entries: ${e}`, "bad");
    }
    scheduleLive(LIVE_EVERY_MS);
  }

  function onLiveChart() {
    clearLines();
    if (!B.info) ensureInfo().then(() => scheduleLive(0)).catch(() => {});
    else scheduleLive(0);
  }

  if (window.__TAURI__) {
    window.__TAURI__.event.listen("bt_progress", ({ payload }) => {
      B.progress = payload.total > 1 ? `${payload.stage}: ${payload.done} / ${payload.total}` : payload.stage;
      if (B.running) {
        const p = document.querySelector("#btResults .bt-empty p");
        if (p) p.textContent = B.progress;
      }
    });
  }
  document.addEventListener("click", (e) => {
    if (e.target && e.target.id === "btBarLive") leaveBacktestView(false);
  });

  A.bounce = { render, leaveBacktestView, onLiveChart };
})();
