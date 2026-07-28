import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// Port tetap: `devUrl` di tauri.conf.json menunjuk ke sini.
export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
    watch: {
      // Perubahan di sisi Rust ditangani cargo, bukan Vite.
      ignored: ["**/src-tauri/**", "**/crates/**"],
    },
  },
  build: {
    target: "chrome105",
    minify: "esbuild",
    sourcemap: false,
  },
});
