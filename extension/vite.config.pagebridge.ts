import { defineConfig } from 'vite';

export default defineConfig({
  define: {
    __RZN_BUILD_SIGNATURE__: JSON.stringify(process.env.RZN_BUILD_SIGNATURE || 'dev-unknown'),
    __RZN_EXTENSION_TARGET__: JSON.stringify(process.env.RZN_EXTENSION_TARGET || 'unknown'),
    __RZN_PAGE_TEST_BRIDGE_ENABLED__: JSON.stringify(process.env.RZN_PAGE_TEST_BRIDGE_ENABLED === '1'),
  },
  build: {
    target: 'es2020',
    lib: {
      entry: 'src/pageBridge.ts',
      name: 'pageBridge',
      formats: ['iife'],
      fileName: () => 'pageBridge.iife.js',
    },
    outDir: process.env.RZN_EXTENSION_OUT_DIR || 'dist',
    emptyOutDir: false,
    sourcemap: false,
    minify: true,
  },
});
