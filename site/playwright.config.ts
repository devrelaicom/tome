import {defineConfig} from '@playwright/test';

export default defineConfig({
  testDir: './e2e',
  webServer: {
    command: 'pnpm build && pnpm serve --port 3210',
    port: 3210,
    reuseExistingServer: false,
    timeout: 180_000,
  },
  use: {baseURL: 'http://localhost:3210'},
});
