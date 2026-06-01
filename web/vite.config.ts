import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

// The mirror serves the built panel under /admin, so asset URLs must resolve
// there. `vite dev` proxies the API + auth routes to a locally running mirror
// so the panel can be developed against live data without embedding.
export default defineConfig({
  base: '/admin/',
  plugins: [svelte()],
  build: {
    outDir: 'dist',
    emptyOutDir: true,
  },
  server: {
    proxy: {
      '/v1': 'http://127.0.0.1:9000',
      '/admin/api': 'http://127.0.0.1:9000',
    },
  },
});
