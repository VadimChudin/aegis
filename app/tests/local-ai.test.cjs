const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");
const { test } = require("node:test");

function fixture() {
  const nodes = new Map();
  const root = { isConnected: true, querySelector(selector) {
    if (!nodes.has(selector)) nodes.set(selector, { checked: true, disabled: false, innerHTML: "", textContent: "", replaceChildren() {} });
    return nodes.get(selector);
  } };
  let failure = null;
  const status = { installed: true, model_downloaded: true, server_ready: true, owned_server: true };
  const window = { AEGIS: {
    S: { panel: "local_ai" }, esc: value => String(value ?? ""), log() {},
    invoke: async () => { if (failure) throw failure; return status; },
  } };
  const source = fs.readFileSync(path.join(__dirname, "../ui/local-ai.js"), "utf8").replace(
    "window.AEGIS.localAI = { render };", "window.AEGIS.localAI = { render, refreshStatus };"
  );
  vm.runInNewContext(source, { window, console });
  return { root, nodes, api: window.AEGIS.localAI, fail(value) { failure = value; } };
}

test("failed local model status disables start, ping and ask but preserves owned stop", async () => {
  const f = fixture();
  await f.api.refreshStatus(f.root);
  assert.equal(f.nodes.get('[data-lai="start"]').disabled, false);
  f.fail(new Error("runtime status unavailable"));
  await f.api.refreshStatus(f.root);
  for (const selector of ['[data-lai="install"]', '[data-lai="start"]', '[data-lai="ping"]', '[data-lai-ask]']) {
    assert.equal(f.nodes.get(selector).disabled, true, selector);
  }
  assert.equal(f.nodes.get('[data-lai="stop"]').disabled, false);
  assert.match(f.nodes.get('[data-lai-status-error]').textContent, /runtime status unavailable/);
});

test("local model controls recover after successful status retry", async () => {
  const f = fixture();
  await f.api.refreshStatus(f.root);
  f.fail(new Error("offline")); await f.api.refreshStatus(f.root);
  f.fail(null); await f.api.refreshStatus(f.root);
  for (const selector of ['[data-lai="start"]', '[data-lai="ping"]', '[data-lai-ask]']) {
    assert.equal(f.nodes.get(selector).disabled, false, selector);
  }
  assert.equal(f.nodes.get('[data-lai-status-error]').hidden, true);
});
