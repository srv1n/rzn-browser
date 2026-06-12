import { defineConfig } from 'vitest/config';

export default defineConfig({
  test: {
    // Unit tests only. Playwright e2e specs live under tests/e2e and should be
    // run via `bun x playwright test`.
    passWithNoTests: true,
    exclude: [
      'tests/e2e/**',
      'dist/**',
      'dist-chrome/**',
      'dist-edge/**',
      'dist-chromium/**',
      'dist-firefox/**',
      'node_modules/**',
    ],
  },
});
