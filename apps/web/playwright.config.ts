import { defineConfig, devices } from '@playwright/test';

const port = Number(process.env.MILKDRIFT_BROWSER_PORT ?? '4173');
if (!Number.isInteger(port) || port < 1024 || port > 65534) throw new Error('MILKDRIFT_BROWSER_PORT must be an unprivileged port with room for the refused-origin server.');
const origin = `http://127.0.0.1:${port}`;

export default defineConfig({
  testDir: './tests/browser',
  timeout: 60000,
  expect: { timeout: 15000 },
  fullyParallel: false,
  workers: 1,
  retries: 0,
  reporter: [['list'], ['html', { open: 'never' }]],
  use: {
    baseURL: origin,
    actionTimeout: 15000,
    // Traces can capture credential form values and bearer headers.
    trace: 'off',
    screenshot: 'only-on-failure',
  },
  projects: [{ name: 'chromium', use: { ...devices['Desktop Chrome'] } }],
  webServer: [
    { command: `npm run preview -- --port ${port} --strictPort`, url: origin, reuseExistingServer: false, timeout: 30000 },
    { command: `npm run preview -- --port ${port + 1} --strictPort`, url: `http://127.0.0.1:${port + 1}`, reuseExistingServer: false, timeout: 30000 },
  ],
});
