import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "./tests/browser",
  use: {
    baseURL: "http://127.0.0.1:4173",
  },
  webServer: {
    command: "busybox httpd -f -p 4173 -h playground",
    url: "http://127.0.0.1:4173/smoke.html",
    reuseExistingServer: false,
  },
});
