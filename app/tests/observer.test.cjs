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
