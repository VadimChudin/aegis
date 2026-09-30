const { defineConfig } = require("@playwright/test");

module.exports = defineConfig({
  testDir: "./tests",
  use: { baseURL: "http://127.0.0.1:3000", viewport: { width: 1360, height: 840 } },
  webServer: {
    command: "python3 -m http.server 3000 --bind 127.0.0.1 --directory ui",
    url: "http://127.0.0.1:3000",
    reuseExistingServer: !process.env.CI,
  },
});
