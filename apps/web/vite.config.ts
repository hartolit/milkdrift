import { defineConfig } from 'vitest/config';
import { svelte } from '@sveltejs/vite-plugin-svelte';

export default defineConfig({
  plugins: [svelte()],
  resolve: { conditions: ['browser'] },
  test: {
    include: ['src/**/*.test.ts'],
    environment: 'jsdom',
    // Use the browser storage implementation instead of Node's experimental global.
    execArgv: ['--no-experimental-webstorage'],
    restoreMocks: true,
  },
});
