import { defineConfig } from 'vite';

export default defineConfig({
  define: {
    __RZN_BUILD_SIGNATURE__: JSON.stringify(process.env.RZN_BUILD_SIGNATURE || 'dev-unknown'),
    __RZN_EXTENSION_TARGET__: JSON.stringify(process.env.RZN_EXTENSION_TARGET || 'unknown'),
    __RZN_PAGE_TEST_BRIDGE_ENABLED__: JSON.stringify(process.env.RZN_PAGE_TEST_BRIDGE_ENABLED === '1'),
  },
  build: {
    target: 'es2020',
    outDir: process.env.RZN_EXTENSION_OUT_DIR || 'dist',
    emptyOutDir: false,
    lib: {
      entry: 'src/background.ts',
      name: 'background',
      fileName: 'background',
      formats: ['iife']
    },
    rollupOptions: {
      output: {
        extend: true
      }
    }
  }
});
