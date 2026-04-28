import { defineConfig } from 'vite';

export default defineConfig({
  define: {
    __RZN_BUILD_SIGNATURE__: JSON.stringify(process.env.RZN_BUILD_SIGNATURE || 'dev-unknown'),
  },
  build: {
    lib: {
      entry: 'src/pageBridge.ts',
      name: 'pageBridge',
      formats: ['iife'],
      fileName: () => 'pageBridge.iife.js',
    },
    outDir: 'dist',
    emptyOutDir: false,
    sourcemap: false,
    minify: true,
  },
});
