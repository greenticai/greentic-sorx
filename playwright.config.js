// @ts-check
const { defineConfig, devices } = require('@playwright/test');

const chromeChannel = process.env.PLAYWRIGHT_CHROME_CHANNEL;

module.exports = defineConfig({
  testDir: './tests/e2e',
  timeout: 45_000,
  expect: {
    timeout: 10_000
  },
  reporter: [['list'], ['html', { open: 'never' }]],
  use: {
    baseURL: process.env.WEBCHAT_E2E_URL || 'http://127.0.0.1:8080/v1/web/webchat/demo/',
    headless: true,
    trace: 'retain-on-failure',
    viewport: { width: 1280, height: 900 }
  },
  projects: [
    {
      name: 'chrome',
      use: {
        ...devices['Desktop Chrome'],
        ...(chromeChannel ? { channel: chromeChannel } : {})
      }
    }
  ]
});
