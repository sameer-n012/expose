// `vitest/config` extends the Vite config type with the `test` section.
import { defineConfig } from 'vitest/config';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { svelteTesting } from '@testing-library/svelte/vite';

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
    // Two test projects:
    // - "unit": pure logic in `*.test.ts`. Node environment, no DOM, fast.
    // - "dom":  stores and components in `*.dom.test.ts`. happy-dom
    //           environment, with the browser build of Svelte (runes are
    //           reactive). happy-dom is used because jsdom 30 needs Node 22+.
    projects: [
      {
        extends: true,
        test: {
          name: 'unit',
          environment: 'node',
          include: ['src/**/*.test.ts'],
          exclude: ['src/**/*.dom.test.ts'],
        },
      },
      {
        extends: true,
        plugins: [svelteTesting()],
        resolve: { conditions: ['browser'] },
        test: {
          name: 'dom',
          environment: 'happy-dom',
          include: ['src/**/*.dom.test.ts'],
          // Vitest replaces CSS with an empty module by default. A test reads
          // the theme tokens, so process this one file.
          css: { include: [/tokens\.css/] },
        },
      },
    ],
  },
});
