import { defineConfig } from "vitest/config";
import vue from "@vitejs/plugin-vue";
import tailwindcss from "@tailwindcss/vite";

export default defineConfig({
  plugins: [vue(), tailwindcss()],
  clearScreen: false,
  server: { host: "127.0.0.1", port: 1420, strictPort: true },
  build: { target: ["es2022", "chrome111", "safari18"] },
  test: {
    environment: "jsdom",
    include: ["src/**/*.test.ts", "tests/unit/**/*.test.ts"],
    maxWorkers: 2,
    testTimeout: 10000,
    restoreMocks: true,
  },
});
