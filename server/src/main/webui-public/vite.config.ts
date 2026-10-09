import { fileURLToPath, URL } from 'node:url'
import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'

// The site is served at the root of the community server, out of its binary: built straight
// into the resources Quarkus serves statically (see the exec plugin in pom.xml), beside the
// back-office at /admin/. `SitePages` serves its index.html for the site's own routes, with
// the page's title and preview (OpenGraph) filled in.
export default defineConfig({
  base: '/',
  plugins: [vue()],
  resolve: {
    alias: { '@': fileURLToPath(new URL('./src', import.meta.url)) },
  },
  build: {
    outDir: '../../../target/classes/META-INF/resources',
    // Never emptied: the back-office was copied there from the sources.
    emptyOutDir: false,
    assetsDir: 'site',
  },
  server: {
    port: 5174,
    strictPort: true,
    // In development the site runs off Vite and the API stays on Quarkus (`./mvnw quarkus:dev`):
    // one origin, the same relative calls as once packaged.
    proxy: {
      '/api': 'http://localhost:8080',
    },
  },
})
