import { fileURLToPath } from "node:url";
import tailwindcss from "@tailwindcss/vite";
import { tanstackStart } from "@tanstack/react-start/plugin/vite";
import viteReact from "@vitejs/plugin-react";
import { nitro } from "nitro/vite";
import { defineConfig } from "vitest/config";

/** The running Compose stack, at its fallback port (OMNI_PORT, as in docker-compose.yml). */
const STACK = `http://127.0.0.1:${process.env.OMNI_PORT ?? "8420"}`;

export default defineConfig({
  resolve: {
    alias: { "@": fileURLToPath(new URL("./src", import.meta.url)) },
  },
  // `vite dev` serves the local source with the API of the running Compose
  // stack (docker-compose.yml): /api and /ws go to the stack's proxy on its
  // fallback port rather than its hostname, so the browser's own Host header
  // reaches the API unchanged; the websocket accepts a human only when the
  // Origin and Host headers agree. Nitro's dev handler answers requests before
  // Vite's proxy does, so /api goes through Nitro's own proxy; the websocket
  // upgrade never reaches Nitro, so /ws goes through Vite's.
  plugins: [
    tailwindcss(),
    tanstackStart(),
    nitro({ devProxy: { "/api/**": { target: STACK } } }),
    viteReact(),
  ],
  server: {
    proxy: { "/ws": { target: STACK, ws: true } },
  },
  // Vitest does not process CSS by default, which leaves the text of the
  // stylesheet that the viewer frame embeds empty.
  test: { css: { include: [/viewer\/doc\.css/] } },
});
