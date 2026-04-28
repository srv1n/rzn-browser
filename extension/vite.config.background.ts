import { defineConfig } from 'vite';

export default defineConfig({
  define: {
    __RZN_BUILD_SIGNATURE__: JSON.stringify(process.env.RZN_BUILD_SIGNATURE || 'dev-unknown'),
  },
  build: {
    outDir: 'dist',
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
