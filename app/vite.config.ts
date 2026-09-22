import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

declare const process: { env: Record<string, string | undefined> };

// Tauri expects a fixed dev port and must see Rust errors, so don't clear the screen.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: { port: 5173, strictPort: true },
  // The rendering spike page is only built when asked for (see docs/spike-rendering.md).
  build: process.env.LOCUS_SPIKE
    ? { rollupOptions: { input: { main: "index.html", spike: "spike.html" } } }
    : {},
});
