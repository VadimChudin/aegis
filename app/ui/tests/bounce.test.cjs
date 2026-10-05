const assert = require("node:assert/strict");
const { readFileSync } = require("node:fs");
const { test } = require("node:test");
const vm = require("node:vm");

function harness() {
  const logs = [];
  const window = {
    I18N: { t: (text) => text },
    AEGIS: {
      S: { panel: "bounce" },
      log: (...args) => logs.push(args),
      esc: (text) => String(text).replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]),
    },
  };
  const source = readFileSync(require.resolve("../bounce.js"), "utf8").replace(
    "  A.bounce = { render, leaveBacktestView, onLiveChart };",
    `  render = () => {};
       renderResults = () => {};
       A.test = { B, run, resultsHtml };`,
  );
  vm.runInNewContext(source, { window, document: { addEventListener() {} }, performance, setTimeout, clearTimeout });
  return { ...window.AEGIS.test, logs };
}

test("failed calculations retain previous results and record a tab-specific error", async () => {
  for (const kind of ["backtest", "ga", "checks"]) {
    const { B, run, logs } = harness();
    const previous = { report: "previous result" };
    B.result = previous;
    B.opt = previous;
    B.val = previous;
    await run(kind, async () => { throw new Error("Archive unavailable <offline>"); });
    assert.match(B.errors[kind], /Archive unavailable <offline>/);
    assert.equal(Object.keys(B.errors).length, 1);
    assert.equal(B.result, previous);
    assert.equal(B.opt, previous);
    assert.equal(B.val, previous);
    assert.equal(B.running, null);
    assert.equal(B.progress, "");
    assert.equal(logs[0][1], "bad");
  }
});

test("the results panel escapes errors and explains that previous results are unchanged", async () => {
  const { B, run, resultsHtml } = harness();
  await run("backtest", async () => { throw "<img src=x onerror=alert(1)>"; });
  const html = resultsHtml();
  assert.match(html, /role="alert"/);
  assert.match(html, /results below are from the previous run/);
  assert.match(html, /&lt;img src=x onerror=alert\(1\)&gt;/);
  assert.doesNotMatch(html, /<img src=x/);
  B.tab = "compare";
  assert.doesNotMatch(resultsHtml(), /role="alert"/);
});

test("retry clears only its own error and rejects overlapping calculations", async () => {
  const { B, run } = harness();
  B.errors = { backtest: "offline", checks: "check failed" };
  let finish;
  const pending = run("backtest", () => new Promise((resolve) => { finish = resolve; }));
  assert.equal(B.errors.backtest, undefined);
  assert.equal(B.errors.checks, "check failed");
  assert.equal(B.running, "backtest");
  let overlaps = 0;
  await run("checks", async () => { overlaps++; });
  assert.equal(overlaps, 0);
  finish();
  await pending;
  assert.equal(B.running, null);
});
