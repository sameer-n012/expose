// `vitest/config` extends the Vite config type with the `test` section.
import { defineConfig } from 'vitest/config';
import { svelte } from '@sveltejs/vite-plugin-svelte';

// Port 3100 is in the "Web UIs (stable)" range. It must match `devUrl` in
// `../src-tauri/tauri.conf.json`.
const DEV_PORT = 3100;

export default defineConfig({
  plugins: [svelte()],
  // Do not clear the terminal, so that Rust errors from `tauri dev` stay visible.
  clearScreen: false,
  server: {
    port: DEV_PORT,
    strictPort: true,
  },
  // Expose Tauri build variables (for example TAURI_ENV_PLATFORM) to the UI.
  envPrefix: ['VITE_', 'TAURI_ENV_'],
  build: {
    // The webview on macOS is WebKit (Safari 13+ features are available).
    target: 'safari13',
    outDir: 'dist',
    emptyOutDir: true,
  },
  test: {
    environment: 'node',
    include: ['src/**/*.test.ts'],
  },
});
