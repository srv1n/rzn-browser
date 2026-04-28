import path from 'path';
import { defineConfig } from 'vite';

export default defineConfig({
  define: {
    __RZN_BUILD_SIGNATURE__: JSON.stringify(process.env.RZN_BUILD_SIGNATURE || 'dev-unknown'),
  },
  build: {
    outDir: 'dist',
    emptyOutDir: false,
    rollupOptions: {
      input: {
        popup: path.resolve(__dirname, 'popup.html'),
      },
    },
  },
});
