import { defineConfig, devices } from '@playwright/test';
import path from 'path';
import { fileURLToPath } from 'url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const extensionPath = path.resolve(__dirname, 'dist/chrome');
const headless = process.env.RZN_PW_HEADFUL !== '1';
const channel = (process.env.RZN_PW_CHANNEL || (headless ? 'chromium' : undefined)) as any;

export default defineConfig({
  testDir: 'tests/e2e',
  timeout: 60_000,
  retries: 0,
  fullyParallel: true,
  reporter: 'list',
  use: {
    // Default to headless so local e2e runs do not steal focus. Set
    // RZN_PW_HEADFUL=1 when visually debugging extension behavior.
    headless,
    // Use Chrome channel only when explicitly requested
    channel,
    viewport: { width: 1280, height: 800 },
    video: 'off',
    trace: 'off',
  },
  projects: [
    {
      name: 'chromium-extension',
      use: {
        ...devices['Desktop Chrome'],
      },
    },
  ],
  // Launch Chromium with the extension loaded
  // We use a persistent context so the extension can initialize
  // The tests themselves create the context
  workers: 1,
});
