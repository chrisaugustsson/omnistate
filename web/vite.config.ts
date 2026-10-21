import { fileURLToPath } from "node:url";
import tailwindcss from "@tailwindcss/vite";
import { tanstackStart } from "@tanstack/react-start/plugin/vite";
import viteReact from "@vitejs/plugin-react";
import { nitro } from "nitro/vite";
import { defineConfig } from "vitest/config";

export default defineConfig({
  resolve: {
    alias: { "@": fileURLToPath(new URL("./src", import.meta.url)) },
  },
  plugins: [tailwindcss(), tanstackStart(), nitro(), viteReact()],
  // Vitest does not process CSS by default, which leaves the text of the
  // stylesheet that the viewer frame embeds empty.
  test: { css: { include: [/viewer\/doc\.css/] } },
});
