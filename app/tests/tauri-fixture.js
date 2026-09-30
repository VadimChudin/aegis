"use strict";

// Synthetic desktop responses: never connect to a broker or store real credentials.
(() => {
  const brokers = ["binance", "bybit"].map((id) => ({
    id, name: id === "binance" ? "Binance" : "Bybit", venue: "USDT Perpetual",
    symbol: "XAUUSDT", icon: `assets/brokers/${id === "binance" ? "binance.svg" : "bybit.png"}`,
    requirements: ["Use synthetic test credentials only."], keys_url: "",
    fields: [
      { key: "api_key", label: "API key", secret: false, optional: false, hint: "" },
      { key: "api_secret", label: "Secret key", secret: true, optional: false, hint: "" },
    ],
  }));
  const settings = {
    theme: "glass-dark", lang: "en", timeframe: "15m", chart_broker: null,
    brokers: Object.fromEntries(brokers.map(({ id }) => [id, {
      stored: [], values: {}, auto_connect: false, last_report: null,
    }])),
  };
  const fixture = window.fixture = { outcome: "fail", calls: [], sessions: [] };
  window.__TAURI__ = {
    event: { listen: async () => () => {} },
    core: { invoke: async (cmd, args) => {
      fixture.calls.push({ cmd, args });
      if (cmd === "bootstrap") return { version: "0.5.1", brokers, settings, sessions: fixture.sessions, timeframes: ["5m", "15m"], strategies: [] };
      if (cmd === "settings_get") return settings;
      if (cmd === "sessions") return fixture.sessions;
      if (cmd === "broker_connect") {
        if (fixture.outcome === "throw") throw new Error("Connection interrupted. Try again.");
        const connected = fixture.outcome === "success";
        const report = { connected, ready: connected, checks: connected ? [] : [{ status: "fail", label: "Key accepted", detail: "Key rejected. Check your credentials." }] };
        // Match the desktop: a failed reconnect keeps the previous saved credentials.
        if (connected || !fixture.sessions.some((s) => s.broker === args.broker)) {
          settings.brokers[args.broker].values = { api_key: args.form.api_key };
          settings.brokers[args.broker].stored = ["api_secret"];
        }
        settings.brokers[args.broker].auto_connect = args.autoConnect;
        if (connected) fixture.sessions = [{ broker: args.broker, account: "Demo", symbol: "XAUUSDT" }];
        return report;
      }
      if (cmd === "load_chart") return { generation: 1, candles: [] };
      if (cmd === "stop_chart" || cmd === "set_auto_connect") return;
      throw new Error(`Unimplemented fixture command: ${cmd}`);
    } },
  };
})();
