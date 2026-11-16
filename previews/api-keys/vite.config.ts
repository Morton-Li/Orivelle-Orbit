import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";
import tailwindcss from "@tailwindcss/vite";
import path from "node:path";

export default defineConfig({
  root: import.meta.dirname,
  base: "./",
  plugins: [vue(), tailwindcss()],
  server: {
    host: "127.0.0.1",
    port: 1436,
    strictPort: true,
    fs: {
      allow: [
        path.resolve(import.meta.dirname, "../.."),
        path.resolve(import.meta.dirname, "../../node_modules"),
      ],
    },
  },
  build: {
    outDir: path.resolve(
      import.meta.dirname,
      "../../.cache/api-key-preview/dist",
    ),
    emptyOutDir: true,
    target: ["es2022", "safari18"],
  },
});
