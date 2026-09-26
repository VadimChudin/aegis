"use strict";

const $ = (id) => document.getElementById(id);
const tauri = window.__TAURI__;
const invoke = (cmd, args) => tauri.core.invoke(cmd, args);

const state = {
  brokers: [],
  selected: "binance",
  session: null,
  timeframe: "15m",
  generation: 0,
  lastTime: 0,
};

// ---- chart ---------------------------------------------------------------

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
    textColor: "#8e9ab0",
    fontFamily: "-apple-system, BlinkMacSystemFont, 'Segoe UI', Inter, sans-serif",
  },
  grid: {
    vertLines: { color: "rgba(255,255,255,0.035)" },
    horzLines: { color: "rgba(255,255,255,0.035)" },
  },
  rightPriceScale: { borderColor: "rgba(255,255,255,0.08)" },
  timeScale: { borderColor: "rgba(255,255,255,0.08)", timeVisible: true, secondsVisible: false, rightOffset: 6 },
  crosshair: { mode: LightweightCharts.CrosshairMode.Normal },
});
const candleSeries = chart.addCandlestickSeries({
  upColor: "#2fbf71",
  downColor: "#e5484d",
  wickUpColor: "#2fbf71",
  wickDownColor: "#e5484d",
  borderVisible: false,
  priceFormat: { type: "price", precision: 2, minMove: 0.01 },
});
const volumeSeries = chart.addHistogramSeries({
  priceScaleId: "volume",
  priceFormat: { type: "volume" },
  lastValueVisible: false,
  priceLineVisible: false,
});
chart.priceScale("volume").applyOptions({ scaleMargins: { top: 0.84, bottom: 0 } });

const volumeBar = (c) => ({
  time: c.time,
  value: c.volume,
  color: c.close >= c.open ? "rgba(47,191,113,0.32)" : "rgba(229,72,77,0.32)",
});

function clearChart() {
  candleSeries.setData([]);
  volumeSeries.setData([]);
  state.lastTime = 0;
  $("lastPrice").textContent = "—";
}

function showLast(c) {
  if (!c) return;
  const el = $("lastPrice");
  el.textContent = c.close.toFixed(2);
  el.dataset.dir = c.close >= c.open ? "up" : "down";
}

function setFeed(kind, text) {
  $("feed").dataset.state = kind;
  $("feedText").textContent = text;
  $("feed").title = text;
}

async function loadChart() {
  if (!state.session) return;
  const timeframe = state.timeframe;
  setFeed("busy", "Loading…");
  try {
    const data = await invoke("load_chart", { timeframe });
    if (timeframe !== state.timeframe || !state.session) return;
    state.generation = data.generation;
    candleSeries.setData(data.candles.map(({ volume, ...bar }) => bar));
    volumeSeries.setData(data.candles.map(volumeBar));
    state.lastTime = data.candles.length ? data.candles[data.candles.length - 1].time : 0;
    chart.timeScale().setVisibleLogicalRange({ from: data.candles.length - 160, to: data.candles.length + 6 });
    showLast(data.candles[data.candles.length - 1]);
    $("chartMeta").textContent = `${data.symbol} · ${state.session.name} · ${timeframe}`;
    setFeed("ok", "Live");
  } catch (err) {
    if (String(err) !== "superseded") setFeed("error", String(err));
  }
}

// ---- header --------------------------------------------------------------

function renderTimeframes(list) {
  const nav = $("timeframes");
  nav.replaceChildren(
    ...list.map((tf) => {
      const b = document.createElement("button");
      b.type = "button";
      b.textContent = tf;
      b.dataset.value = tf;
      b.className = tf === state.timeframe ? "active" : "";
      b.onclick = () => {
        if (tf === state.timeframe) return;
        state.timeframe = tf;
        nav.querySelectorAll("button").forEach((x) => x.classList.toggle("active", x.dataset.value === tf));
        clearChart();
        loadChart();
      };
      return b;
    }),
  );
}

function renderSession() {
  const s = state.session;
  $("empty").classList.toggle("hidden", !!s);
  $("brokerButton").textContent = s ? `${s.name} · ${s.account}` : "Connect broker";
  $("brokerButton").classList.toggle("connected", !!s);
  $("symbol").textContent = s ? s.symbol : "XAU";
  $("disconnect").classList.toggle("hidden", !s);
  if (!s) {
    $("chartMeta").textContent = "No broker";
    setFeed("off", "Offline");
  }
}

function renderStrategies(list) {
  $("strategies").replaceChildren(
    ...list.map((s) => {
      const card = document.createElement("div");
      card.className = "strategy";
      card.innerHTML = `
        <div class="row"><span class="name"></span><span class="badge">Stub</span></div>
        <p class="summary"></p>
        <label class="switch" title="Not implemented yet"><input type="checkbox" disabled><span></span></label>`;
      card.querySelector(".name").textContent = s.name;
      card.querySelector(".summary").textContent = s.summary;
      card.dataset.id = s.id;
      return card;
    }),
  );
}

// ---- connect dialog ------------------------------------------------------

function openModal() {
  if (state.session) state.selected = state.session.broker;
  renderBrokerForm();
  $("formError").classList.add("hidden");
  $("modal").classList.remove("hidden");
  const first = $("fields").querySelector("input");
  if (first) first.focus();
}

function closeModal() {
  $("modal").classList.add("hidden");
}

function renderBrokerForm() {
  $("brokerTabs").replaceChildren(
    ...state.brokers.map((b) => {
      const t = document.createElement("button");
      t.type = "button";
      t.textContent = b.name;
      t.className = b.id === state.selected ? "active" : "";
      t.onclick = () => {
        state.selected = b.id;
        $("formError").classList.add("hidden");
        renderBrokerForm();
      };
      return t;
    }),
  );
  const info = state.brokers.find((b) => b.id === state.selected);
  $("venue").innerHTML = `<strong></strong><span></span><p class="muted small"></p>`;
  $("venue").querySelector("strong").textContent = info.symbol;
  $("venue").querySelector("span").textContent = ` · ${info.venue}`;
  $("venue").querySelector("p").textContent = info.note;
  $("fields").replaceChildren(
    ...info.fields.map((f) => {
      const label = document.createElement("label");
      label.innerHTML = `<span></span><input>`;
      label.querySelector("span").textContent = f.optional ? `${f.label} (optional)` : f.label;
      const input = label.querySelector("input");
      input.name = f.key;
      input.type = f.secret ? "password" : "text";
      input.placeholder = f.placeholder || "";
      input.spellcheck = false;
      return label;
    }),
  );
}

function showFormError(text) {
  const el = $("formError");
  el.textContent = text;
  el.classList.remove("hidden");
}

async function submitConnect() {
  if ($("connect").disabled) return;
  const info = state.brokers.find((b) => b.id === state.selected);
  const credentials = { broker: info.id };
  for (const f of info.fields) {
    const value = $("form").elements[f.key].value.trim();
    if (!value && !f.optional) return showFormError(`${f.label} is required`);
    if (value) credentials[f.key] = value;
  }
  const button = $("connect");
  button.disabled = true;
  button.textContent = "Connecting…";
  $("formError").classList.add("hidden");
  clearChart();
  state.session = null;
  renderSession();
  try {
    state.session = await invoke("connect", { credentials });
    $("form").querySelectorAll("input[type=password]").forEach((i) => (i.value = ""));
    closeModal();
    renderSession();
    await loadChart();
  } catch (err) {
    showFormError(String(err));
  } finally {
    button.disabled = false;
    button.textContent = "Connect";
  }
}

async function disconnect() {
  await invoke("disconnect");
  state.session = null;
  clearChart();
  renderSession();
  closeModal();
}

// ---- live feed -----------------------------------------------------------

function listenFeed() {
  tauri.event.listen("candle", ({ payload }) => {
    if (payload.generation !== state.generation) return;
    const c = payload.candle;
    // The feed resends the previous bar; the chart only accepts the newest one.
    if (c.time < state.lastTime) return;
    const { volume, ...bar } = c;
    candleSeries.update(bar);
    volumeSeries.update(volumeBar(c));
    state.lastTime = c.time;
    showLast(c);
  });
  tauri.event.listen("feed_status", ({ payload }) => {
    if (payload.generation !== state.generation) return;
    setFeed(payload.ok ? "ok" : "error", payload.message);
  });
}

// ---- boot ----------------------------------------------------------------

window.addEventListener("error", (e) => setFeed("error", `UI error: ${e.message}`));
window.addEventListener("unhandledrejection", (e) => setFeed("error", `UI error: ${e.reason}`));

async function boot() {
  if (!tauri) {
    document.body.classList.add("no-tauri");
    setFeed("error", "Open AEGIS through the desktop app");
    return;
  }
  $("brokerButton").onclick = openModal;
  $("emptyConnect").onclick = openModal;
  $("modalClose").onclick = closeModal;
  $("modal").onclick = (e) => e.target === $("modal") && closeModal();
  // Explicit handlers: native implicit submission is unreliable in some WebKitGTK builds.
  $("form").onsubmit = (e) => {
    e.preventDefault();
    submitConnect();
  };
  $("connect").onclick = submitConnect;
  $("fields").addEventListener("keydown", (e) => {
    if (e.key === "Enter") {
      e.preventDefault();
      submitConnect();
    }
  });
  $("disconnect").onclick = disconnect;
  document.addEventListener("keydown", (e) => e.key === "Escape" && closeModal());

  const [brokers, timeframes, strategies, session] = await Promise.all([
    invoke("brokers"),
    invoke("timeframes"),
    invoke("strategies"),
    invoke("session"),
  ]);
  state.brokers = brokers;
  state.session = session;
  renderTimeframes(timeframes);
  renderStrategies(strategies);
  renderSession();
  listenFeed();
  if (session) loadChart();
}

boot().catch((err) => setFeed("error", String(err)));
