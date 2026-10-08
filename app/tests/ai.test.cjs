const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");
const { test } = require("node:test");

// Run the real UI module with a small DOM/IPC double. No backend, model,
// network, broker, or trading process is started by these tests.
function fixture() {
  const calls = [];
  const timers = [];
  let status = { running: false, busy: false, settings: { local_model: "saved-model" }, paper: { positions: [] } };
  let statusError = null;
  const handlers = {};
  const nodes = new Map();
  for (const id of ["ai-status", "ai-settings", "ai-operation-result", "ai-start", "ai-stop", "ai-step", "ai-setup", "ai-forget-key", "ai-setup-progress", "ai-api-key", "ai-broker"]) {
    nodes.set(id, { innerHTML: "", textContent: "", disabled: false, placeholder: "", value: id === "ai-broker" ? "binance" : "", querySelector() { return null; } });
  }
  let mounted = false;
  const root = {
    dataset: {}, isConnected: true,
    set innerHTML(value) { mounted = true; nodes.get("ai-settings").innerHTML = ""; },
    querySelector(selector) { return selector === "[data-testid='ai-panel']" ? (mounted ? {} : null) : nodes.get(selector.slice(1)) || null; },
    addEventListener(name, callback) { handlers[name] = callback; },
  };
  const S = { panel: "ai", sessions: { binance: {}, bybit: {} } };
  const window = { AEGIS: { S, esc: (v) => String(v ?? ""), invoke: async (command, args) => {
    calls.push({ command, args });
    if (command === "ai_status") { if (statusError) throw statusError; return status; }
    if (command === "ai_test") return "model answered: OK";
  } } };
  const source = fs.readFileSync(path.join(__dirname, "../ui/ai.js"), "utf8").replace(
    "window.AEGIS.ai = { render };",
    "window.AEGIS.ai = { render, refresh, action, handleClick, renderStatus };"
  );
  vm.runInNewContext(source, { window, document: { getElementById: () => root }, setInterval: (fn) => { timers.push(fn); return timers.length; }, clearInterval() {}, console });
  return { api: window.AEGIS.ai, nodes, root, S, calls, handlers, timers,
    setStatus(value) { status = value; }, setStatusError(value) { statusError = value; },
    unmount() { mounted = false; nodes.get("ai-settings").innerHTML = ""; },
    click(id) { const button = { id, disabled: nodes.get(id)?.disabled || false }; return { target: { closest: (selector) => selector === "button" ? button : null } }; },
  };
}
const flush = () => new Promise(resolve => setImmediate(resolve));

test("settings are repopulated after the AI panel is cleared and reopened", async () => {
  const f = fixture();
  f.api.render(f.root); await flush();
  assert.match(f.nodes.get("ai-settings").innerHTML, /saved-model/);
  f.unmount(); f.api.render(f.root); await flush();
  assert.match(f.nodes.get("ai-settings").innerHTML, /saved-model/);
});

test("failed status refresh disables both start and single step", async () => {
  const f = fixture();
  await f.api.refresh(f.root);
  assert.equal(f.nodes.get("ai-start").disabled, false);
  f.setStatusError(new Error("IPC unavailable"));
  await f.api.refresh(f.root);
  assert.equal(f.nodes.get("ai-start").disabled, true);
  assert.equal(f.nodes.get("ai-step").disabled, true);
  assert.match(f.nodes.get("ai-status").innerHTML, /IPC unavailable/);
  f.api.handleClick(f.root, f.click("ai-step")); await flush();
  assert.equal(f.calls.filter(x => x.command === "ai_step").length, 0);
});

test("backend recovery errors block start and single step equally", async () => {
  const f = fixture();
  f.setStatus({ message: "Paper recovery error", paper: {}, settings: {} });
  await f.api.refresh(f.root);
  assert.equal(f.nodes.get("ai-start").disabled, true);
  assert.equal(f.nodes.get("ai-step").disabled, true);
});

test("model test preserves its returned diagnostic result", async () => {
  const f = fixture();
  await f.api.refresh(f.root);
  await f.api.action(f.root, "Test local", "ai_test", { cloud: false });
  assert.match(f.nodes.get("ai-operation-result").textContent, /model answered: OK/);
});

test("stale Paper close controls cannot close a different broker position", async () => {
  const f = fixture();
  f.setStatus({ paper: { positions: [{ id: 42, broker: "binance" }] }, settings: {} });
  await f.api.refresh(f.root);
  f.nodes.get("ai-broker").value = "bybit";
  const close = { disabled: false, dataset: { positionId: "42" } };
  f.api.handleClick(f.root, { target: { closest: selector => selector === "[data-ai-close]" ? close : null } });
  await flush();
  assert.equal(f.calls.filter(x => x.command === "ai_close").length, 0);
});

test("broker selection change rerenders Paper close eligibility", async () => {
  const f = fixture();
  f.setStatus({ paper: { positions: [{ id: 42, broker: "binance" }] }, settings: {} });
  f.api.render(f.root); await flush();
  f.nodes.get("ai-broker").value = "bybit";
  assert.equal(typeof f.handlers.change, "function");
  f.handlers.change({ target: { id: "ai-broker" } });
  assert.match(f.nodes.get("ai-status").innerHTML, /data-position-id="42" disabled/);
});

test("successful refresh recovers controls without losing unsaved settings", async () => {
  const f = fixture();
  f.api.render(f.root); await flush();
  f.nodes.get("ai-settings").innerHTML = "unsaved edit";
  f.setStatusError(new Error("temporarily offline"));
  await f.api.refresh(f.root);
  f.setStatusError(null); await f.api.refresh(f.root);
  assert.equal(f.nodes.get("ai-start").disabled, false);
  assert.equal(f.nodes.get("ai-step").disabled, false);
  assert.equal(f.nodes.get("ai-settings").innerHTML, "unsaved edit");
});

test("Paper stop remains available after a failed running-status refresh", async () => {
  const f = fixture();
  f.setStatus({ running: true, paper: {}, settings: {} });
  await f.api.refresh(f.root);
  f.setStatusError(new Error("status failure")); await f.api.refresh(f.root);
  assert.equal(f.nodes.get("ai-stop").disabled, false);
  f.api.handleClick(f.root, f.click("ai-stop")); await flush();
  assert.equal(f.calls.filter(x => x.command === "ai_stop").length, 1);
});

test("eligible Paper start and close dispatch only Paper IPC commands", async () => {
  const f = fixture();
  f.setStatus({ paper: { positions: [{ id: 42, broker: "binance" }] }, settings: {} });
  await f.api.refresh(f.root);
  f.api.handleClick(f.root, f.click("ai-start")); await flush();
  const close = { disabled: false, dataset: { positionId: "42" } };
  f.api.handleClick(f.root, { target: { closest: selector => selector === "[data-ai-close]" ? close : null } });
  await flush();
  assert.equal(f.calls.filter(x => x.command === "ai_start").length, 1);
  assert.equal(f.calls.filter(x => x.command === "ai_close").length, 1);
  assert.ok(f.calls.every(x => ["ai_status", "ai_start", "ai_close"].includes(x.command)));
  assert.equal(f.calls.find(x => x.command === "ai_close").args.broker, "binance");
});

test("local URL hint matches the backend chat-completions endpoint contract", async () => {
  const f = fixture();
  f.api.render(f.root); await flush();
  assert.match(f.nodes.get("ai-settings").innerHTML, /name="local_url"[^>]*placeholder="http:\/\/127\.0\.0\.1:11434\/v1\/chat\/completions"/);
});
