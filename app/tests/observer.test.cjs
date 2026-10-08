const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");
const { test } = require("node:test");

function fixture() {
  const nodes = new Map();
  const calls = [];
  let finish;
  const elements = () => [];
  const document = {
    getElementById: (id) => nodes.get(id),
    createElement: () => ({
      dataset: {}, listeners: {}, buttons: [],
      addEventListener(name, handler) { this.listeners[name] = handler; },
      querySelectorAll: function () { return this.buttons; },
      set innerHTML(value) {
        this.buttons = [...value.matchAll(/<button[^>]*>/g)].map(([tag]) => ({
          disabled: false,
          dataset: { ticket: tag.match(/data-ticket="([^"]+)"/)?.[1], closeAll: tag.includes("data-close-all") ? "yes" : undefined },
          closest() { return this; },
        }));
      },
    }),
    body: { append(node) { nodes.set(node.id, node); } },
  };
  const window = {
    I18N: { lang: "en", t: (s) => s },
    AEGIS: {
      S: {}, log() {},
      invoke(command) {
        calls.push(command);
        if (command === "observer_close") return new Promise((resolve) => { finish = resolve; });
        return Promise.resolve({});
      },
    },
  };
  const context = { window, document, console, setInterval, clearInterval, setTimeout, URL, Blob };
  vm.runInNewContext(fs.readFileSync(path.join(__dirname, "../ui/observer.js"), "utf8"), context);
  return { window, nodes, calls, finish: (value) => finish(value), elements };
}

test("polling cannot re-enable or repeat a pending quick-close request", async () => {
  const f = fixture();
  const status = { account: { positions: [{ ticket: 501, magic: 26070552, symbol: "XAUUSD" }] } };
  f.window.AEGIS.observer.syncChartStatus(status);
  const box = f.nodes.get("positionActions");
  const request = box.listeners.click({ target: box.buttons[0] });
  assert.deepEqual(f.calls, ["observer_close"]);
  f.window.AEGIS.observer.syncChartStatus(status);
  assert.ok(box.buttons.every((button) => button.disabled));
  await box.listeners.click({ target: box.buttons[0] });
  assert.deepEqual(f.calls, ["observer_close"]);
  f.finish({ status: "filled" });
  await request;
  assert.deepEqual(f.calls, ["observer_close", "observer_status"]);
  assert.equal(box.hidden, true);
});

test("quick-close controls never include foreign account positions", () => {
  const f = fixture();
  f.window.AEGIS.observer.syncChartStatus({ account: { positions: [
    { ticket: 501, magic: 26070552, symbol: "XAUUSD" },
    { ticket: 502, magic: 1, symbol: "XAUUSD" },
    { ticket: 503, magic: 26070552, symbol: "EURUSD" },
  ] } });
  const buttons = f.nodes.get("positionActions").buttons;
  assert.deepEqual(buttons.filter((b) => b.dataset.ticket).map((b) => b.dataset.ticket), ["501"]);
});

// Active Settings -> AI observer controls, with all IPC calls mocked.
function settingsFixture(mode = "paper") {
  const nodes = new Map();
  const listeners = {};
  const node = selector => {
    if (!nodes.has(selector)) nodes.set(selector, { dataset: {}, disabled: false, checked: false,
      textContent: "", innerHTML: "", classList: { toggle() {} }, querySelectorAll: () => [],
    });
    return nodes.get(selector);
  };
  const root = { isConnected: true, dataset: {}, querySelector: node, querySelectorAll: () => [],
    addEventListener(name, callback) { listeners[name] = callback; },
  };
  const calls = [];
  let fail = null;
  let status = { running: false, money_armed: mode === "money" };
  const window = { I18N: { lang: "en", t: value => value }, AEGIS: { S: {}, esc: v => String(v ?? ""), log() {},
    invoke: async command => { calls.push(command); if (command === "observer_status" && fail) throw fail; return status; },
  } };
  const source = fs.readFileSync(path.join(__dirname, "../ui/observer.js"), "utf8").replace(
    "window.AEGIS.observer = { render, renderSettings,", "window.AEGIS.observer = { refreshStatus, updateStatus, bind, cfgFrom, saveConfig, render, renderSettings,"
  );
  vm.runInNewContext(source, { window, document: { getElementById: () => node("positionActions") }, console });
  const api = window.AEGIS.observer;
  const config = api.cfgFrom({ ai_enabled: true, mode });
  const state = { loaded: true, busy: false, config, saved: config, status, notice: "" };
  api.bind(root, state);
  return { api, state, root, node, listeners, calls, fail(value) { fail = value; }, setStatus(value) { status = value; } };
}

for (const mode of ["paper", "money"]) {
  test(`active observer cannot start with stale ${mode} status`, async () => {
    const f = settingsFixture(mode);
    await f.api.refreshStatus(f.root, f.state);
    const button = f.node('[data-action="run"]'); button.dataset.action = "run";
    assert.equal(button.disabled, false);
    f.fail(new Error("observer status unavailable"));
    await f.api.refreshStatus(f.root, f.state);
    assert.equal(button.disabled, true, mode);
    await f.listeners.click({ target: { closest: () => button } });
    assert.equal(f.calls.includes("observer_start"), false);
  });
}

test("active observer start recovers only after a successful status refresh", async () => {
  const f = settingsFixture();
  f.fail(new Error("offline")); await f.api.refreshStatus(f.root, f.state);
  assert.equal(f.node('[data-action="run"]').disabled, true);
  f.fail(null); await f.api.refreshStatus(f.root, f.state);
  assert.equal(f.node('[data-action="run"]').disabled, false);
});

test("active observer stop remains available when status refresh fails", async () => {
  const f = settingsFixture("money");
  f.setStatus({ running: true, money_armed: true });
  await f.api.refreshStatus(f.root, f.state);
  f.fail(new Error("offline")); await f.api.refreshStatus(f.root, f.state);
  const button = f.node('[data-action="run"]'); button.dataset.action = "run";
  assert.equal(button.disabled, false);
  assert.equal(button.dataset.run, "stop");
  await f.listeners.click({ target: { closest: () => button } });
  assert.ok(f.calls.includes("observer_stop"));
  assert.equal(f.calls.includes("observer_start"), false);
});

test("active observer Money mode requires explicit consent before saving or arming", async () => {
  const f = settingsFixture("money");
  await assert.rejects(f.api.saveConfig(f.root, f.state));
  assert.equal(f.calls.includes("observer_save"), false);
  assert.equal(f.calls.includes("observer_arm_money"), false);
  f.state.status.money_armed = false;
  f.api.updateStatus(f.root, f.state);
  assert.equal(f.node('[data-action="run"]').disabled, true);
});
