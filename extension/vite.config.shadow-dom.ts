import { defineConfig } from 'vite';

export default defineConfig({
  build: {
    outDir: 'dist',
    emptyOutDir: false,
    lib: {
      entry: 'src/content/shadow-dom-instrumentation.ts',
      name: 'shadowDomInstrumentation',
      fileName: 'shadow-dom-instrumentation',
      formats: ['iife']
    },
    rollupOptions: {
      output: {
        extend: true
      }
    }
  }
});