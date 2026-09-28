"use strict";

// Bounce strategy panel: sliders and toggles with presets, backtest, genetic optimisation with
// its checks, and chart overlays (expected entries with probability on the live chart; signals
// and trades in the backtest view).
(() => {
  const A = window.AEGIS;
  if (!A) return;
  const { invoke, log, esc, S } = A;
  const t = (s, v) => window.I18N.t(s, v);
  const $ = (id) => document.getElementById(id);

  // Cost presets only: no setting is profitable out of sample yet (docs/research/bounce.md).
  const PRESETS = {
    roboforex: { label: "RoboForex ECN costs", set: { maker_bps: 0.2, taker_bps: 0.2, slippage: 0.15, spread: 0.2 } },
    binance: { label: "Binance costs", set: { maker_bps: 2, taker_bps: 5, slippage: 0.05, spread: 0.02 } },
  };
  const GROUP_ORDER = ["Probability", "Trade", "Costs", "Direction", "Levels", "Sessions", "Touch"];
  const KEY_METRICS = 10;
  const LIVE_EVERY_MS = 5 * 60 * 1000;
  const TABS = [
    ["backtest", "Backtest"],
    ["ga", "Genetic algorithm"],
    ["checks", "Checks"],
    ["compare", "Compare"],
  ];
  const CHECK_TEXT = {
    oos_positive: ["Positive expectancy", "Average R per trade after costs, with its 95% bootstrap interval over days. The lower end must be above zero."],
    psr: ["Probabilistic Sharpe ≥ 0.95", "Probability that the true Sharpe ratio of these trades is above zero, given their number, skew and fat tails (Bailey & López de Prado 2012)."],
    t_daily: ["Daily t-statistic > 3", "Harvey & Liu: with many tested ideas, a real effect needs t > 3 on daily results."],
    fills: ["Limit fills", "The same trades when a limit fills on a touch, as set, and only when price trades $0.10 through it. The strictest rule must stay positive."],
    dsr: ["Deflated Sharpe ≥ 0.95", "The chosen settings' Sharpe against the best Sharpe expected by luck from the number of settings the GA tried (Bailey & López de Prado 2014)."],
    control: ["Levels beat random prices", "Same settings on levels moved to meaningless prices. If this fails, the edge comes from the conditions the model picks, not from the levels."],
    permutation: ["Metrics matter", "Same settings with the metrics shuffled between touches. The edge must disappear."],
    pbo: ["Overfitting probability < 0.5", "Combinatorially symmetric cross-validation over the GA's final population (Bailey, Borwein, López de Prado & Zhu 2017)."],
    ga_vs_random: ["GA beats random search", "Out of sample, the GA's settings against random search with the same number of evaluations in every window."],
  };

  const B = {
    info: null,
    params: null,
    result: null,
    opt: null,
    val: null,
    spec: null,
    tab: "backtest",
    running: null,
    progress: "",
    lines: [],
    liveTimer: null,
    live: null,
    selected: null,
    available: null,
    open: { Probability: true, Trade: true, "Key metrics": true },
  };

  const clone = (x) => JSON.parse(JSON.stringify(x));
  const pct = (x, d = 1) => (Number.isFinite(x) ? `${(x * 100).toFixed(d)}%` : "—");
  const num = (x, d = 2) => (Number.isFinite(x) ? x.toFixed(d) : "—");
  const fmtDate = (ts) => new Date(ts * 1000).toISOString().slice(0, 16).replace("T", " ");
  const fmtDay = (ts) => new Date(ts * 1000).toISOString().slice(0, 10);
  const css = (n) => getComputedStyle(document.documentElement).getPropertyValue(n).trim();

  // ---- params by spec id ("long", "kinds.swing5", "scan.zone_atr", "filters.wick") --------

  function getParam(id) {
    const p = B.params;
    if (id === "entry") return p.entry === "close";
    const [head, key] = id.split(".");
    if (!key) return p[head];
    return p[head]?.[key];
  }

  function setParam(id, value) {
    const p = B.params;
    if (id === "entry") p.entry = value ? "close" : "limit";
    else {
      const [head, key] = id.split(".");
      if (!key) p[head] = value;
      else (p[head] = p[head] || {})[key] = value;
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
    B.spec = { ...B.info.ga, metrics: [...B.info.ga_metrics] };
  }

  function merge(target, patch) {
    for (const [k, v] of Object.entries(patch)) {
      if (v && typeof v === "object" && !Array.isArray(v)) merge((target[k] = target[k] || {}), v);
      else target[k] = v;
    }
  }

  function applyPreset(key) {
    if (key === "reset") B.params = clone(B.info.defaults);
    else if (PRESETS[key]) {
      B.params = clone(B.info.defaults);
      merge(B.params, PRESETS[key].set);
    } else if (B.info.presets[key]) B.params = clone(B.info.presets[key]);
    save();
    render();
  }

  async function savePreset() {
    const input = $("btPresetName");
    const name = (input?.value || "").trim();
    if (!name) return input?.focus();
    B.info.presets = await invoke("bounce_preset_save", { name, params: B.params });
    log(t("Preset saved: {name}", { name }), "ok");
    render();
  }

  async function deletePreset(name) {
    B.info.presets = await invoke("bounce_preset_save", { name, params: null });
    render();
  }

  // ---- settings column -------------------------------------------------------------------

  function researchBars(featureId) {
    const r = B.info.research?.metrics?.[featureId];
    if (!r) return "";
    const base = B.info.research.base;
    const cells = r.bins
      .map((b) => {
        const diff = b.win - base;
        const cls = diff > 0.02 ? "up" : diff < -0.02 ? "down" : "";
        const h = Math.max(4, Math.min(100, 50 + diff * 250));
        return `<i class="${cls}" style="height:${h}%" title="${esc(`${num(b.lo, 3)} … ${num(b.hi, 3)}: ${t("win")} ${pct(b.win)} (n=${b.n})`)}"></i>`;
      })
      .join("");
    return `<div class="rq" title="${esc(t("Win rate by quintile in the research run (base {base})", { base: pct(base) }))}">${cells}</div>`;
  }

  const stepDigits = (step) => (String(step).includes(".") ? String(step).split(".")[1].length : 0);

  function toggleRow(spec) {
    const on = !!getParam(spec.id);
    return `<div class="bt-row" title="${esc(t(spec.help))}"><span class="bt-label">${esc(t(spec.label))}${spec.rescan ? ` <span class="tag">${esc(t("rescan"))}</span>` : ""}</span>
      <button type="button" class="toggle${on ? " on" : ""}" data-id="${esc(spec.id)}" data-kind="toggle"><span class="knob"></span></button></div>`;
  }

  /** Why a setting has no effect with the current toggles ("" when it is active). */
  function inactive(id) {
    if (id === "min_prob" && !B.params.use_model) return t("Only with the probability model on.");
    if (id === "fill_through" && B.params.entry === "close") return t("Only for limit entries.");
    return "";
  }

  function sliderRow(spec) {
    const v = Number(getParam(spec.id));
    const d = stepDigits(spec.step);
    const off = inactive(spec.id);
    if (off) {
      return `<div class="bt-row slider off" title="${esc(off)}"><span class="bt-label">${esc(t(spec.label))} <span class="tag">${esc(off)}</span></span>
      <input type="range" min="${spec.lo}" max="${spec.hi}" step="${spec.step}" value="${v}" data-id="${esc(spec.id)}" data-kind="slider" data-digits="${d}" disabled>
      <output>${v.toFixed(d)}</output></div>`;
    }
    return `<div class="bt-row slider" title="${esc(t(spec.help))}"><span class="bt-label">${esc(t(spec.label))}${spec.rescan ? ` <span class="tag">${esc(t("rescan"))}</span>` : ""}</span>
      <input type="range" min="${spec.lo}" max="${spec.hi}" step="${spec.step}" value="${v}" data-id="${esc(spec.id)}" data-kind="slider" data-digits="${d}">
      <output>${v.toFixed(d)}</output></div>`;
  }

  function filterRow(spec) {
    const id = spec.id.slice("filters.".length);
    const f = getParam(spec.id) || { on: false, min: spec.lo, max: spec.hi };
    const d = stepDigits(spec.step);
    const inModel = B.info.model_features.includes(id);
    const share = B.available ? B.available[id] : 1;
    const noData = B.available && !(share > 0.05);
    const why = noData ? t("No data for this metric in the loaded history or in this entry mode.") : "";
    return `<div class="bt-filter${f.on ? " on" : ""}${noData ? " nodata" : ""}" data-id="${esc(spec.id)}" title="${esc(t(spec.help))}">
      <div class="bt-row"><span class="bt-label">${esc(t(spec.label))}${inModel ? ` <span class="tag">${esc(t("model"))}</span>` : ""}${
        noData ? ` <span class="tag warn" title="${esc(why)}">${esc(t("no data"))}</span>` : ""
      }</span>
        <output>${f.on ? `${Number(f.min).toFixed(d)} … ${Number(f.max).toFixed(d)}` : esc(t("any"))}</output>
        <button type="button" class="toggle${f.on ? " on" : ""}" data-kind="filter-on" ${noData ? "disabled" : ""}><span class="knob"></span></button></div>
      <div class="bt-range">
        <input type="range" min="${spec.lo}" max="${spec.hi}" step="${spec.step}" value="${Math.max(spec.lo, f.min)}" data-kind="filter-min" data-digits="${d}" ${noData ? "disabled" : ""}>
        <input type="range" min="${spec.lo}" max="${spec.hi}" step="${spec.step}" value="${Math.min(spec.hi, f.max)}" data-kind="filter-max" data-digits="${d}" ${noData ? "disabled" : ""}>
      </div>
      ${researchBars(id)}
    </div>`;
  }

  function group(name, rows, label = name) {
    const open = B.open[name] ? " open" : "";
    return `<details class="bt-group"${open} data-group="${esc(name)}"><summary>${esc(label)}</summary>${rows}</details>`;
  }

  function presetsHtml() {
    const built = Object.entries(PRESETS)
      .map(([k, p]) => `<button type="button" class="ghost sm" data-preset="${k}">${esc(t(p.label))}</button>`)
      .join("");
    const mine = Object.keys(B.info.presets || {})
      .map(
        (n) =>
          `<span class="bt-mine"><button type="button" class="ghost sm" data-preset="${esc(n)}">${esc(n)}</button><button type="button" class="ghost sm x" data-delpreset="${esc(
            n,
          )}" title="${esc(t("Delete"))}">✕</button></span>`,
      )
      .join("");
    return `<div class="bt-presets">${built}<button type="button" class="ghost sm" data-preset="reset">${esc(t("Reset"))}</button></div>
      <div class="bt-presets">${mine}<input id="btPresetName" class="bt-input" placeholder="${esc(t("Preset name"))}" maxlength="40">
      <button type="button" class="ghost sm" id="btPresetSave">${esc(t("Save preset"))}</button></div>`;
  }

  function settingsHtml() {
    const specs = B.info.specs;
    const html = [presetsHtml()];
    for (const g of GROUP_ORDER) {
      const rows = specs
        .filter((s) => s.group === g && s.kind !== "filter")
        .map((s) => (s.kind === "toggle" ? toggleRow(s) : sliderRow(s)))
        .join("");
      if (rows) html.push(group(g, rows, t(g)));
    }
    const filters = specs.filter((s) => s.kind === "filter");
    const spread = (s) => B.info.research?.metrics?.[s.id.slice(8)]?.spread ?? 0;
    const key = [...filters].sort((a, b) => spread(b) - spread(a)).slice(0, KEY_METRICS);
    const note = `<p class="hint">${esc(
      t(
        B.params.entry === "close"
          ? "Bars: win rate by quintile in the research run. Metrics of the touch bar."
          : "Bars: win rate by quintile in the research run. Metrics as of the bar before the touch (what a resting limit order can know).",
      ),
    )}${B.available ? "" : ` ${esc(t("Run a backtest to see which metrics the loaded history has."))}`}</p>`;
    html.push(group("Key metrics", note + key.map(filterRow).join(""), t("Key metrics")));
    const byGroup = {};
    for (const f of filters) if (!key.includes(f)) (byGroup[f.group] = byGroup[f.group] || []).push(f);
    for (const [g, list] of Object.entries(byGroup)) html.push(group(`Metrics · ${g}`, list.map(filterRow).join(""), `${t("Metrics")} · ${t(g)}`));
    return html.join("");
  }

  // ---- results: shared pieces --------------------------------------------------------------

  function statsHtml(s) {
    const cards = [
      [t("Trades"), s.trades, t("{n} / day", { n: num(s.trades_per_day, 1) })],
      [t("Win rate"), pct(s.win_rate), t("95% low {x}", { x: pct(s.win_rate_lo) })],
      [t("Avg R"), num(s.avg_r, 3), `${t("win")} ${num(s.avg_win_r)} · ${t("loss")} ${num(s.avg_loss_r)}`],
      [t("Total R"), num(s.total_r, 1), `PF ${num(s.profit_factor)}`],
      [t("Max drawdown"), `${num(s.max_dd_r, 1)} R`, t("worst streak {n}", { n: s.max_losing_streak })],
      [t("Daily Sharpe"), num(s.daily_sharpe, 3), `${t("per trade")} ${num(s.sharpe, 3)}`],
    ];
    return `<div class="bt-cards">${cards
      .map(([k, v, sub]) => `<div class="bt-card"><span>${esc(k)}</span><strong>${esc(v)}</strong><em>${esc(sub)}</em></div>`)
      .join("")}</div>`;
  }

  function lineChart(series, { h = 120, zero = true } = {}) {
    const w = 560;
    const all = series.flatMap((s) => s.points.map((p) => p[1])).filter(Number.isFinite);
    if (!all.length) return "";
    const lo = Math.min(zero ? 0 : Infinity, ...all);
    const hi = Math.max(zero ? 0 : -Infinity, ...all);
    const n = Math.max(...series.map((s) => s.points.length));
    const x = (i) => (i / Math.max(1, n - 1)) * w;
    const y = (v) => h - ((v - lo) / Math.max(1e-9, hi - lo)) * h;
    const paths = series
      .map((s) => {
        const d = s.points
          .filter((p) => Number.isFinite(p[1]))
          .map((p, i) => `${i ? "L" : "M"}${x(p[0]).toFixed(1)},${y(p[1]).toFixed(1)}`)
          .join("");
        return `<path d="${d}" style="stroke:${s.color}"/>`;
      })
      .join("");
    const legend = series.map((s) => `<span><i style="background:${s.color}"></i>${esc(s.label)}</span>`).join("");
    return `<svg class="bt-equity" viewBox="0 0 ${w} ${h}" preserveAspectRatio="none">${zero ? `<line x1="0" x2="${w}" y1="${y(0)}" y2="${y(0)}"/>` : ""}${paths}</svg>${
      series.length > 1 ? `<div class="bt-legend">${legend}</div>` : ""
    }`;
  }

  const equity = (eq) => lineChart([{ label: t("Equity, R"), color: css("--pos"), points: eq.map((p, i) => [i, p[1]]) }]);

  function tableHtml(title, map, labels = {}) {
    const rows = Object.entries(map)
      .map(
        ([k, s]) =>
          `<tr><td>${esc(labels[k] || k)}</td><td>${s.trades}</td><td class="${s.win_rate >= 0.7 ? "pos" : ""}">${pct(s.win_rate)}</td><td class="${
            s.total_r >= 0 ? "pos" : "neg"
          }">${num(s.total_r, 1)}</td><td>${num(s.avg_r, 3)}</td></tr>`,
      )
      .join("");
    return `<div class="bt-table"><div class="bt-sub">${esc(title)}</div><table><tr><th></th><th>${esc(t("Trades"))}</th><th>${esc(t("Win"))}</th><th>R</th><th>${esc(
      t("Avg R"),
    )}</th></tr>${rows}</table></div>`;
  }

  function tradesHtml(trades, source) {
    const last = trades.slice(-150).reverse();
    const rows = last
      .map((tr) => {
        const i = trades.indexOf(tr);
        return `<tr data-trade="${i}" data-src="${source}" class="${B.selected === `${source}:${i}` ? "sel" : ""}"><td>${esc(fmtDate(tr.entry_time))}</td><td class="${
          tr.dir > 0 ? "pos" : "neg"
        }">${esc(t(tr.dir > 0 ? "Long" : "Short"))}</td><td>${num(tr.entry)}</td><td>${pct(tr.prob, 0)}</td><td>${esc(t(tr.outcome))}</td><td class="${
          tr.r > 0 ? "pos" : "neg"
        }">${num(tr.r)}</td></tr>`;
      })
      .join("");
    return `<div class="bt-table"><div class="bt-sub">${esc(t("Trades (latest 150, click to show on chart)"))}</div><table class="trades"><tr><th>${esc(
      t("Entry (UTC)"),
    )}</th><th>${esc(t("Side"))}</th><th>${esc(t("Price"))}</th><th>${esc(t("Prob."))}</th><th>${esc(t("Exit"))}</th><th>R</th></tr>${rows}</table></div>`;
  }

  function spinner() {
    return `<div class="bt-empty"><div class="spinner"></div><p id="btProgress">${esc(B.progress || t("Running…"))}</p></div>`;
  }

  // ---- tab: backtest ---------------------------------------------------------------------

  function backtestHtml() {
    const run = `<div class="bt-run"><button type="button" class="cta" id="btRun" ${B.running ? "disabled" : ""}>${esc(
      t(B.running === "backtest" ? "Running…" : "Run backtest"),
    )}</button><span class="hint">${esc(t("No orders are sent: backtest and expected entries only."))}</span></div>`;
    if (B.running === "backtest") return run + spinner();
    const r = B.result?.report;
    if (!r) {
      return `${run}<div class="bt-empty"><p>${esc(
        t(
          "Press Run backtest. AEGIS downloads Binance XAUUSDT 5m history since the listing with open interest and funding (public archive, no key), finds every level touch, scores each touch with a model trained only on earlier months, and simulates the trades with costs.",
        ),
      )}</p></div>`;
    }
    const s = r.stats;
    const kinds = Object.fromEntries(B.info.kinds.map(([id, label]) => [id, t(label)]));
    const losing =
      s.trades >= 30 && s.avg_r <= 0
        ? `<p class="hint warn">${esc(t("These settings lose after costs ({r} R per trade). Do not trade them.", { r: num(s.avg_r, 3) }))}</p>`
        : "";
    const note =
      s.trades < 30
        ? `<p class="hint warn">${esc(t("Only {n} trades: too few to trust.", { n: s.trades }))}</p>`
        : `<p class="hint">${esc(B.result.source)} · ${esc(fmtDay(r.from))} → ${esc(fmtDay(r.to))} UTC · ${r.bars} ${esc(t("bars"))}. ${esc(
            t("Probabilities are out of sample: each month is scored and calibrated only from the months before it."),
          )}</p>`;
    return `${run}${statsHtml(s)}${losing}${note}${equity(r.equity)}
      <div class="bt-btns"><button type="button" class="ghost sm" data-show="backtest">${esc(t("Show on chart"))}</button></div>
      ${tableHtml(t("By month"), r.by_month)}${tableHtml(t("By session"), r.by_session)}${tableHtml(t("By level kind"), r.by_kind, kinds)}${tradesHtml(r.trades, "backtest")}`;
  }

  // ---- tab: genetic algorithm ------------------------------------------------------------------

  const GA_FIELDS = [
    ["target_win_rate", "Target win rate", 0.5, 0.9, 0.01, 2],
    ["target_trades_per_day", "Target trades per day", 1, 100, 1, 0],
    ["population", "Population", 16, 128, 8, 0],
    ["generations", "Generations", 5, 100, 1, 0],
    ["patience", "Stop after generations without progress", 3, 30, 1, 0],
    ["train_days", "Train window, days", 30, 120, 5, 0],
    ["test_days", "Test window, days", 7, 60, 1, 0],
    ["min_trades", "Min trades in a train window", 10, 300, 10, 0],
    ["complexity", "Cost per active filter", 0, 0.5, 0.01, 2],
    ["seed", "Random seed", 1, 99, 1, 0],
  ];

  function gaSpecHtml() {
    const rows = GA_FIELDS.map(
      ([id, label, lo, hi, step, d]) =>
        `<div class="bt-row slider"><span class="bt-label">${esc(t(label))}</span><input type="range" min="${lo}" max="${hi}" step="${step}" value="${
          B.spec[id]
        }" data-ga="${id}" data-digits="${d}"><output>${Number(B.spec[id]).toFixed(d)}</output></div>`,
    ).join("");
    const metrics = B.info.features
      .map((f) => {
        const on = B.spec.metrics.includes(f.id);
        const noData = B.available && !(B.available[f.id] > 0.05);
        return `<label class="bt-chk${noData ? " nodata" : ""}"><input type="checkbox" data-gametric="${esc(f.id)}" ${on ? "checked" : ""}> ${esc(t(f.label))}</label>`;
      })
      .join("");
    return `${group("GA settings", rows, t("GA settings"))}${group(
      "GA metrics",
      `<p class="hint">${esc(t("Metric filters the GA may switch on and set. Fewer metrics = less room to overfit."))}</p><div class="bt-chks">${metrics}</div>`,
      t("Metrics the GA may use"),
    )}`;
  }

  function convergenceHtml(conv) {
    const f = (k) => conv.map((c, i) => [i, Math.max(-10, c[k])]);
    return lineChart(
      [
        { label: t("Best (train)"), color: css("--pos"), points: f("best") },
        { label: t("Population mean"), color: css("--muted"), points: f("mean") },
        { label: t("Best on validation"), color: css("--apple"), points: f("validation") },
      ],
      { h: 110 },
    );
  }

  function checksHtml(v) {
    if (!v) return "";
    const rows = v.checks
      .map((c) => {
        const [title, help] = CHECK_TEXT[c.id] || [c.id, ""];
        return `<div class="chk chk-${c.pass ? "ok" : "fail"}"><span class="chk-ico">${c.pass ? "✓" : "✕"}</span><div><div>${esc(t(title))}: <b>${esc(
          c.value,
        )}</b></div><div class="hint">${esc(t(help))}</div></div></div>`;
      })
      .join("");
    const passed = v.checks.filter((c) => c.pass).length;
    const extra = [
      [t("Bootstrap 95%: win rate"), `${pct(v.boot_win_rate[0])} … ${pct(v.boot_win_rate[1])}`],
      [t("Bootstrap 95%: avg R"), `${num(v.boot_avg_r[0], 3)} … ${num(v.boot_avg_r[1], 3)}`],
      [t("Bootstrap 95%: trades per day"), `${num(v.boot_per_day[0], 1)} … ${num(v.boot_per_day[1], 1)}`],
      [t("Every touch, no filters"), `${v.base.trades} · ${pct(v.base.win_rate)} · ${num(v.base.avg_r, 3)} R`],
      [t("Random levels"), `${v.control.trades} · ${pct(v.control.win_rate)} · ${num(v.control.avg_r, 3)} R`],
      [t("Shuffled metrics"), `${v.permuted.trades} · ${pct(v.permuted.win_rate)} · ${num(v.permuted.avg_r, 3)} R`],
    ]
      .map(([k, x]) => `<tr><td>${esc(k)}</td><td>${esc(x)}</td></tr>`)
      .join("");
    return `<div class="bt-sub">${esc(t("Checks: {p} of {n} passed", { p: passed, n: v.checks.length }))}</div><div class="checks">${rows}</div>
      <div class="bt-table"><table>${extra}</table></div>`;
  }

  function diffHtml(a, b) {
    const rows = [];
    for (const s of B.info.specs) {
      const ids = s.kind === "filter" ? [`${s.id}`] : [s.id];
      for (const id of ids) {
        const get = (p) => {
          if (s.kind === "filter") {
            const f = p.filters[id.slice(8)];
            return f && f.on ? `${num(f.min, stepDigits(s.step))} … ${num(f.max, stepDigits(s.step))}` : t("any");
          }
          if (id === "entry") return t(p.entry === "close" ? "close" : "limit");
          const [h, k] = id.split(".");
          const v = k ? p[h]?.[k] : p[h];
          return typeof v === "boolean" ? t(v ? "on" : "off") : Number.isFinite(v) ? num(v, stepDigits(s.step)) : String(v ?? "—");
        };
        const x = get(a);
        const y = get(b);
        if (x !== y) rows.push(`<tr><td>${esc(t(s.label))}</td><td>${esc(x)}</td><td class="pos">${esc(y)}</td></tr>`);
      }
    }
    return rows.length
      ? `<div class="bt-table"><div class="bt-sub">${esc(t("What the GA changed"))}</div><table><tr><th></th><th>${esc(t("Now"))}</th><th>GA</th></tr>${rows.join("")}</table></div>`
      : `<p class="hint">${esc(t("The GA kept your settings."))}</p>`;
  }

  function gaHtml() {
    const run = `<div class="bt-run"><button type="button" class="cta" id="btOptimize" ${B.running ? "disabled" : ""}>${esc(
      t(B.running === "ga" ? "Running…" : "Run genetic algorithm"),
    )}</button><span class="hint">${esc(t("Walk-forward: the GA tunes on each train window, the next window tests it."))}</span></div>`;
    const head = run + gaSpecHtml();
    if (B.running === "ga") return `${head}<div class="bt-progress"><div class="bar" id="btGaBar"></div></div>${spinner()}`;
    const o = B.opt;
    if (!o) {
      return `${head}<p class="hint">${esc(
        t(
          "The GA evolves a population of settings (tournament selection, SBX crossover, polynomial mutation, elitism). Each window is split into train and validation; early stopping and the final choice use validation and the fitness of nearby settings. The honest number is the out-of-sample result of the test windows. A random search with the same budget runs next to it, and the checks tab reports the overfitting statistics.",
        ),
      )}</p>`;
    }
    const r = o.report;
    const ro = r.baseline.random_out_of_sample;
    const windows = r.windows
      .map(
        (w) =>
          `<tr><td>${esc(fmtDay(w.test_from))}</td><td>${w.train.trades} · ${pct(w.train.win_rate)} · ${num(w.train.avg_r, 3)}</td><td class="${
            w.test.avg_r > 0 ? "pos" : "neg"
          }">${w.test.trades} · ${pct(w.test.win_rate)} · ${num(w.test.avg_r, 3)}</td><td>${w.generations}</td></tr>`,
      )
      .join("");
    return `${head}
      <div class="bt-sub">${esc(t("Out of sample (test windows the GA never saw)"))}</div>${statsHtml(r.out_of_sample)}
      ${equity(r.oos_trades.map((x, i, arr) => [x.exit_time, arr.slice(0, i + 1).reduce((a, y) => a + y.r, 0)]))}
      <div class="bt-btns"><button type="button" class="cta" id="btApply">${esc(t("Apply to sliders"))}</button>
        <button type="button" class="ghost sm" data-show="oos">${esc(t("Show out-of-sample trades on chart"))}</button></div>
      <p class="hint">${esc(
        t("{e} evaluations in {s} s. Random search, same budget, out of sample: {n} trades · {w} · {r} R per trade.", {
          e: r.evaluations,
          s: num(o.seconds, 1),
          n: ro.trades,
          w: pct(ro.win_rate),
          r: num(ro.avg_r, 3),
        }),
      )}</p>
      <div class="bt-sub">${esc(t("Convergence of the latest GA run"))}</div>${convergenceHtml(r.convergence)}
      <div class="bt-table"><div class="bt-sub">${esc(t("Walk-forward windows"))}</div><table><tr><th>${esc(t("Test from"))}</th><th>${esc(t("Train"))}</th><th>${esc(
        t("Test"),
      )}</th><th>${esc(t("Gen."))}</th></tr>${windows}</table></div>
      ${checksHtml(o.validation)}${diffHtml(B.params, r.params)}`;
  }

  // ---- tab: checks and compare --------------------------------------------------------------

  function checksTabHtml() {
    const run = `<div class="bt-run"><button type="button" class="cta" id="btValidate" ${B.running ? "disabled" : ""}>${esc(
      t(B.running === "checks" ? "Running…" : "Check current settings"),
    )}</button></div>`;
    if (B.running === "checks") return run + spinner();
    if (!B.val)
      return `${run}<p class="hint">${esc(
        t("Statistical checks of the current settings: bootstrap intervals, Probabilistic Sharpe, daily t-statistic, fill rules, random levels and shuffled metrics."),
      )}</p>`;
    return `${run}${statsHtml(B.val.stats)}${checksHtml(B.val)}`;
  }

  function compareHtml() {
    const cols = [];
    if (B.result) cols.push([t("Your settings (backtest)"), B.result.report.stats]);
    if (B.opt) {
      cols.push([t("GA, out of sample"), B.opt.report.out_of_sample]);
      cols.push([t("Random search, out of sample"), B.opt.report.baseline.random_out_of_sample]);
      cols.push([t("GA settings, full backtest"), B.opt.backtest.stats]);
    }
    if (!cols.length) return `<p class="hint">${esc(t("Run a backtest and the genetic algorithm to compare them."))}</p>`;
    const rows = [
      ["Trades", (s) => s.trades],
      ["Trades per day", (s) => num(s.trades_per_day, 1)],
      ["Win rate", (s) => pct(s.win_rate)],
      ["Avg R", (s) => num(s.avg_r, 3)],
      ["Total R", (s) => num(s.total_r, 1)],
      ["Profit factor", (s) => num(s.profit_factor)],
      ["Max drawdown", (s) => `${num(s.max_dd_r, 1)} R`],
      ["Daily Sharpe", (s) => num(s.daily_sharpe, 3)],
    ]
      .map(([k, f]) => `<tr><td>${esc(t(k))}</td>${cols.map(([, s]) => `<td>${esc(String(f(s)))}</td>`).join("")}</tr>`)
      .join("");
    return `<div class="bt-table"><table><tr><th></th>${cols.map(([h]) => `<th>${esc(h)}</th>`).join("")}</tr>${rows}</table></div>
      <p class="hint">${esc(
        t("The full backtest of the GA settings includes the months they were chosen on, so it is optimistic. Compare decisions on the out-of-sample columns."),
      )}</p>`;
  }

  function resultsHtml() {
    const tabs = `<div class="bt-tabs">${TABS.map(
      ([id, label]) => `<button type="button" class="bt-tab${B.tab === id ? " on" : ""}" data-tab="${id}">${esc(t(label))}</button>`,
    ).join("")}</div>`;
    const body = { backtest: backtestHtml, ga: gaHtml, checks: checksTabHtml, compare: compareHtml }[B.tab]();
    return tabs + body;
  }

  // ---- render and bind ---------------------------------------------------------------------

  function render(body) {
    body = body || $("panelBody");
    if (!B.info) {
      body.innerHTML = `<p class="hint">${esc(t("Loading…"))}</p>`;
      ensureInfo()
        .then(() => S.panel === "bounce" && render())
        .catch((e) => (body.innerHTML = `<p class="hint warn">${esc(e)}</p>`));
      return;
    }
    const keep = [$("btSettings")?.scrollTop, $("btResults")?.scrollTop];
    body.innerHTML = `<div class="bt-cols">
      <div class="bt-col" id="btSettings">${settingsHtml()}</div>
      <div class="bt-col" id="btResults">${resultsHtml()}</div></div>`;
    if (keep[0]) $("btSettings").scrollTop = keep[0];
    if (keep[1]) $("btResults").scrollTop = keep[1];
    bindSettings(body);
    bindResults();
  }

  function renderResults() {
    const el = $("btResults");
    if (!el) return;
    const top = el.scrollTop;
    el.innerHTML = resultsHtml();
    el.scrollTop = top;
    bindResults();
  }

  function bindSettings(body) {
    body.querySelectorAll("details.bt-group").forEach((d) => d.addEventListener("toggle", () => (B.open[d.dataset.group] = d.open)));
    body.querySelectorAll("[data-preset]").forEach((b) => (b.onclick = () => applyPreset(b.dataset.preset)));
    body.querySelectorAll("[data-delpreset]").forEach((b) => (b.onclick = () => deletePreset(b.dataset.delpreset)));
    const ps = $("btPresetSave");
    if (ps) ps.onclick = savePreset;
    body.querySelectorAll('#btSettings [data-kind="toggle"]').forEach((el) => {
      el.onclick = () => {
        el.classList.toggle("on");
        setParam(el.dataset.id, el.classList.contains("on"));
        if (el.dataset.id === "entry") B.available = null;
        if (el.dataset.id === "entry" || el.dataset.id === "use_model") render();
      };
    });
    body.querySelectorAll('#btSettings [data-kind="slider"]').forEach((r) => {
      r.oninput = () => {
        const v = Number(r.value);
        r.nextElementSibling.textContent = v.toFixed(Number(r.dataset.digits));
        const id = r.dataset.id;
        setParam(id, ["max_bars", "max_open", "scan.swing_n"].includes(id) ? Math.round(v) : v);
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
        out.textContent = f.on ? `${a.toFixed(d)} … ${b.toFixed(d)}` : t("any");
        setParam(id, f);
      };
      on.onclick = () => update(!on.classList.contains("on"));
      lo.oninput = () => update(true);
      hi.oninput = () => update(true);
    });
  }

  function bindResults() {
    const root = $("btResults");
    if (!root) return;
    root.querySelectorAll("[data-tab]").forEach((b) => {
      b.onclick = () => {
        B.tab = b.dataset.tab;
        renderResults();
      };
    });
    root.querySelectorAll("details.bt-group").forEach((d) => d.addEventListener("toggle", () => (B.open[d.dataset.group] = d.open)));
    const on = (id, f) => {
      const el = $(id);
      if (el) el.onclick = f;
    };
    on("btRun", runBacktest);
    on("btOptimize", runOptimize);
    on("btValidate", runValidate);
    on("btApply", applyGa);
    root.querySelectorAll("[data-show]").forEach((b) => (b.onclick = () => enterBacktestView(b.dataset.show)));
    root.querySelectorAll("tr[data-trade]").forEach((tr) => {
      tr.onclick = () => {
        B.selected = `${tr.dataset.src}:${tr.dataset.trade}`;
        enterBacktestView(tr.dataset.src, Number(tr.dataset.trade));
      };
    });
    root.querySelectorAll("[data-ga]").forEach((r) => {
      r.oninput = () => {
        const v = Number(r.value);
        r.nextElementSibling.textContent = v.toFixed(Number(r.dataset.digits));
        B.spec[r.dataset.ga] = v;
      };
    });
    root.querySelectorAll("[data-gametric]").forEach((c) => {
      c.onchange = () => {
        const id = c.dataset.gametric;
        B.spec.metrics = c.checked ? [...new Set([...B.spec.metrics, id])] : B.spec.metrics.filter((m) => m !== id);
      };
    });
  }

  // ---- actions ---------------------------------------------------------------------------------

  async function run(kind, f) {
    if (B.running) return;
    B.running = kind;
    B.progress = t("Loading history…");
    renderResults();
    const t0 = performance.now();
    try {
      await f();
    } catch (e) {
      log(`Bounce: ${e}`, "bad");
    }
    B.running = null;
    B.progress = "";
    if (S.panel === "bounce") render();
    return (performance.now() - t0) / 1000;
  }

  function runBacktest() {
    return run("backtest", async () => {
      B.selected = null;
      B.result = await invoke("bounce_backtest", { params: B.params });
      B.available = B.result.available;
      const s = B.result.report.stats;
      log(
        t("Bounce backtest: {n} trades, win {w}, avg {r} R, total {tr} R, {d} per day", {
          n: s.trades,
          w: pct(s.win_rate),
          r: num(s.avg_r, 3),
          tr: num(s.total_r, 1),
          d: num(s.trades_per_day, 1),
        }),
        s.avg_r > 0 ? "ok" : "warn",
      );
    });
  }

  function runValidate() {
    return run("checks", async () => {
      B.val = await invoke("bounce_validate", { params: B.params });
      const passed = B.val.checks.filter((c) => c.pass).length;
      log(t("Bounce checks: {p} of {n} passed", { p: passed, n: B.val.checks.length }), passed === B.val.checks.length ? "ok" : "warn");
    });
  }

  function runOptimize() {
    return run("ga", async () => {
      B.opt = await invoke("bounce_optimize", { params: B.params, spec: B.spec });
      const s = B.opt.report.out_of_sample;
      log(
        t("Genetic algorithm: out of sample {n} trades, win {w}, avg {r} R, {d} per day ({e} evaluations)", {
          n: s.trades,
          w: pct(s.win_rate),
          r: num(s.avg_r, 3),
          d: num(s.trades_per_day, 1),
          e: B.opt.report.evaluations,
        }),
        s.avg_r > 0 ? "ok" : "warn",
      );
    });
  }

  function applyGa() {
    if (!B.opt) return;
    B.params = clone(B.opt.report.params);
    save();
    log(t("GA settings applied to the sliders."), "ok");
    render();
  }

  // ---- chart ---------------------------------------------------------------------------------

  function clearLines() {
    for (const l of B.lines) A.candles.removePriceLine(l);
    B.lines = [];
  }

  function line(price, color, title, style = 0) {
    B.lines.push(A.candles.createPriceLine({ price, color, lineWidth: 1, lineStyle: style, axisLabelVisible: true, title }));
  }

  function enterBacktestView(source, selected) {
    const res = source === "oos" ? B.opt : B.result;
    if (!res) return;
    const trades = source === "oos" ? res.report.oos_trades : res.report.trades;
    const signals = source === "oos" ? [] : res.report.signals;
    S.btView = true;
    S.chart.generation = -1; // drop live feed events while the backtest is shown
    A.closePanel();
    clearLines();
    const bars = res.bars.map(([time, open, high, low, close]) => ({ time, open, high, low, close }));
    A.candles.setData(bars);
    A.volumes.setData([]);
    const [up, down, win, loss, muted] = ["--mark-up", "--mark-down", "--mark-win", "--mark-loss", "--muted"].map(css);
    const markers = [];
    for (const s of signals) {
      if (!s.taken) markers.push({ time: s.time, position: s.dir > 0 ? "belowBar" : "aboveBar", color: muted, shape: "circle", size: 0.4 });
    }
    for (const tr of trades) {
      markers.push({
        time: tr.entry_time,
        position: tr.dir > 0 ? "belowBar" : "aboveBar",
        color: tr.dir > 0 ? up : down,
        shape: tr.dir > 0 ? "arrowUp" : "arrowDown",
        text: pct(tr.prob, 0),
      });
      markers.push({
        time: tr.exit_time - 300,
        position: tr.dir > 0 ? "aboveBar" : "belowBar",
        color: tr.r > 0 ? win : loss,
        shape: "circle",
        text: `${tr.r > 0 ? "+" : ""}${tr.r.toFixed(2)}R`,
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
    if (Number.isInteger(selected) && trades[selected]) {
      const tr = trades[selected];
      line(tr.entry, tr.dir > 0 ? up : down, `${t(tr.dir > 0 ? "Long" : "Short")} ${pct(tr.prob, 0)}`);
      line(tr.sl, loss, "SL", 2);
      line(tr.tp, win, "TP", 2);
      const i = idx(tr.entry_time);
      A.chart.timeScale().setVisibleLogicalRange({ from: i - 80, to: i + 60 });
    } else {
      A.chart.timeScale().setVisibleLogicalRange({ from: bars.length - 300, to: bars.length + 6 });
    }
    const wins = trades.filter((x) => x.r > 0).length;
    const total = trades.reduce((a, x) => a + x.r, 0);
    $("btBar").hidden = false;
    $("btBarText").textContent = t("{what} · Binance XAUUSDT 5m · {n} trades · win {w} · {r} R", {
      what: t(source === "oos" ? "GA out of sample" : "Backtest"),
      n: trades.length,
      w: pct(trades.length ? wins / trades.length : NaN),
      r: num(total, 1),
    });
    A.setLamp("busy", t("Backtest view"));
    A.renderChartChrome();
    $("legendSym").textContent = "XAUUSDT";
    $("legendMeta").textContent = `${t("Backtest")} · 5m`;
  }

  function leaveBacktestView(silent) {
    if (!S.btView) return;
    S.btView = false;
    $("btBar").hidden = true;
    A.candles.setMarkers([]);
    clearLines();
    if (!silent) A.reloadLive();
  }

  // ---- expected entries on the live chart ---------------------------------------------------

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
      // Levels come from Binance XAUUSDT; other venues quote gold with a basis, so their
      // lines are shifted by the current price difference.
      const chartLast = S.bars.length ? S.bars[S.bars.length - 1].close : NaN;
      const basis = S.chart.broker === "binance" || !Number.isFinite(chartLast) ? 0 : chartLast - res.live.price;
      const ok = res.live.entries.filter((e) => e.ok);
      for (const e of ok) {
        const title = `${t("Bounce")} ${t(e.dir > 0 ? "buy" : "sell")} ${pct(e.prob, 0)}${basis ? ` (${t("basis")} ${basis >= 0 ? "+" : ""}${num(basis)})` : ""}`;
        line(e.entry + basis, e.dir > 0 ? css("--mark-up") : css("--mark-down"), title, 1);
      }
      const best = Math.max(0, ...res.live.entries.map((e) => (Number.isFinite(e.prob) ? e.prob : 0)));
      const summary = t("Bounce: {n} expected entries on the chart · {m} armed levels near price, best {b} (threshold {th})", {
        n: ok.length,
        m: res.live.entries.length,
        b: pct(best, 0),
        th: pct(B.params.min_prob, 0),
      });
      if (summary !== B.lastSummary) log(summary, ok.length ? "ok" : "");
      B.lastSummary = summary;
      if (!res.fresh) log(`Bounce: ${res.note}`, "warn");
    } catch (e) {
      log(`${t("Bounce expected entries")}: ${e}`, "bad");
    }
    scheduleLive(LIVE_EVERY_MS);
  }

  function onLiveChart() {
    clearLines();
    if (!B.info) ensureInfo().then(() => scheduleLive(0)).catch(() => {});
    else scheduleLive(0);
  }

  // ---- progress events -------------------------------------------------------------------------

  function setProgress(text, frac) {
    B.progress = text;
    const p = $("btProgress");
    if (p) p.textContent = text;
    const bar = $("btGaBar");
    if (bar && Number.isFinite(frac)) bar.style.width = `${Math.round(frac * 100)}%`;
  }

  if (window.__TAURI__) {
    window.__TAURI__.event.listen("bt_progress", ({ payload }) => {
      setProgress(payload.total > 1 ? `${t(payload.stage)}: ${payload.done} / ${payload.total}` : t(payload.stage));
    });
    window.__TAURI__.event.listen("ga_progress", ({ payload: p }) => {
      if (p.stage === "checks") return setProgress(t("Checks: DSR, PBO, bootstrap, random levels, shuffled metrics"), 1);
      const frac = p.windows ? (p.window - 1 + p.generation / Math.max(1, p.generations)) / p.windows : 0;
      setProgress(
        t("{stage}: window {w} of {n}, generation {g} · best fitness {f}", {
          stage: t(p.stage),
          w: p.window,
          n: p.windows,
          g: p.generation,
          f: num(p.best, 2),
        }),
        frac,
      );
    });
  }
  document.addEventListener("click", (e) => {
    if (e.target && e.target.id === "btBarLive") leaveBacktestView(false);
  });

  A.bounce = { render, leaveBacktestView, onLiveChart };
})();
