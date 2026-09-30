const path = require("node:path");
const { test, expect } = require("@playwright/test");

test.beforeEach(async ({ page }) => {
  await page.addInitScript({ path: path.join(__dirname, "tauri-fixture.js") });
  await page.goto("/");
  await page.getByRole("button", { name: "Connect a broker", exact: true }).click();
});

async function fillCredentials(page, broker, key, secret) {
  await page.locator(`#broker-${broker}-api_key`).fill(key);
  await page.locator(`#broker-${broker}-api_secret`).fill(secret);
}

test("failed reconnect preserves edited credentials and connect-on-start choice", async ({ page }) => {
  await page.evaluate(() => {
    window.fixture.sessions = [{ broker: "binance", account: "Demo", symbol: "XAUUSDT" }];
    window.AEGIS.S.sessions.binance = window.fixture.sessions[0];
    window.AEGIS.S.settings.brokers.binance.values = { api_key: "previous-key" };
    window.AEGIS.S.settings.brokers.binance.stored = ["api_secret"];
  });
  await fillCredentials(page, "binance", "replacement-key", "replacement-secret");
  await page.getByRole("button", { name: "Connect on start", exact: true }).first().click();
  await page.locator('[data-broker="binance"] .act-connect').click();
  await expect(page.getByText("Key rejected. Check your credentials.", { exact: true })).toBeVisible();
  await expect(page.locator("#broker-binance-api_key")).toHaveValue("replacement-key");
  await expect(page.locator("#broker-binance-api_secret")).toHaveValue("replacement-secret");
  await expect(page.locator('[data-broker="binance"] .toggle')).not.toHaveClass(/\bon\b/);
  await expect(page.locator('[data-broker="binance"] .act-connect')).toBeEnabled();
});

test("unexpected error is visible and a subsequent successful connection clears secrets", async ({ page }) => {
  await page.evaluate(() => { window.fixture.outcome = "throw"; });
  await fillCredentials(page, "binance", "test-key", "test-secret");
  await page.locator('[data-broker="binance"] .act-connect').click();
  await expect(page.getByRole("alert")).toContainText("Connection interrupted. Try again.");
  await expect(page.locator("#broker-binance-api_secret")).toHaveValue("test-secret");
  await page.evaluate(() => { window.fixture.outcome = "success"; });
  await page.locator("#broker-binance-api_secret").press("Enter");
  await expect(page.locator('[data-broker="binance"] .act-connect')).toHaveText("Reconnect");
  await expect(page.getByRole("alert")).toHaveCount(0);
  await expect(page.locator("#broker-binance-api_secret")).toHaveValue("");
  await expect(page.locator("#broker-binance-api_secret")).toHaveAttribute("placeholder", "Stored · leave empty to keep");
});

test("connecting another broker leaves its neighbour's draft intact", async ({ page }) => {
  await fillCredentials(page, "binance", "draft-key", "draft-secret");
  await page.locator('[data-broker="bybit"] .broker-head').click();
  await fillCredentials(page, "bybit", "test-key", "test-secret");
  await page.evaluate(() => { window.fixture.outcome = "success"; });
  await page.locator('[data-broker="bybit"] .act-connect').click();
  await expect(page.locator('[data-broker="bybit"] .act-connect')).toHaveText("Reconnect");
  await expect(page.locator("#broker-binance-api_key")).toHaveValue("draft-key");
  await expect(page.locator("#broker-binance-api_secret")).toHaveValue("draft-secret");
  await expect(page.locator("#broker-bybit-api_secret")).toHaveValue("");
});
