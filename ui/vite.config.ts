import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  root: __dirname,
  plugins: [react()],
  server: {
    host: "127.0.0.1",
    port: 1420,
    strictPort: true
  },
  clearScreen: false,
  build: {
    outDir: "dist",
    emptyOutDir: true,
    target: "es2020"
  },
  test: {
    environment: "jsdom",
    globals: true
  }
});
