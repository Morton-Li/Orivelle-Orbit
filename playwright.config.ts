import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "./tests/e2e",
  fullyParallel: false,
  workers: 1,
  timeout: 30000,
  use: {
    baseURL: "http://127.0.0.1:1420",
    browserName: "chromium",
    channel: "chrome",
    launchOptions: { args: ["--disable-breakpad", "--disable-crash-reporter"] },
    trace: "retain-on-failure",
  },
  webServer: {
    command: "npm run dev",
    url: "http://127.0.0.1:1420",
    reuseExistingServer: false,
    timeout: 30000,
  },
});
