import type { Plugin } from 'vite';
import solid from 'vite-plugin-solid';
import wasm from 'vite-plugin-wasm';
// `vitest/config` re-exports Vite's own `defineConfig` with the `test` key
// added to its type. Importing it from there rather than keeping a separate
// `vitest.config.ts` is what makes the unit tests run through THIS config:
// a second file would replace it, and the tested modules would then be
// transformed without `import.meta.glob` resolving `examples/*.md?raw` or the
// `?url` theme imports resolving at all — i.e. the suite would be testing a
// different build of the module than the one that ships.
import { defineConfig } from 'vitest/config';

// Strict Content-Security-Policy for the production bundle. Defense-in-depth
// layered *on top of* the renderer's escaping: the preview is mounted via
// `innerHTML` into `.aozora-md-root` (components/PreviewPane.tsx), but the aozora-md
// renderer (comrak + the 青空文庫 parser) entity-escapes all text and emits no
// active markup, so the CSP is a second wall — not the primary XSS guard.
//
// Directive rationale (kept as tight as the app allows):
//   default-src 'self'            — same-origin baseline for everything.
//   script-src 'self'             — our bundle/chunks only…
//     'wasm-unsafe-eval'          — …plus WebAssembly.instantiate for the
//                                   aozora-flavored-markdown-wasm module (no JS eval/unsafe-eval).
//   style-src 'self'              — hashed CSS assets, incl. the dynamically
//                                   swapped #aozora-md-theme <link href>…
//     'unsafe-inline'             — …plus the runtime <style> tags Solid and
//                                   CodeMirror inject (no nonce path).
//   img-src 'self' data:          — favicon + inline data: URIs.
//   font-src 'self'               — no external/CDN fonts are loaded.
//   connect-src 'self'            — covers the same-origin fetch() that
//                                   vite-plugin-wasm's instantiateStreaming()
//                                   uses to pull the .wasm asset
//                                   (assetsInlineLimit: 0 ⇒ no data:/blob:).
//   object-src 'none'             — no <object>/<embed>/<applet>.
//   base-uri 'self'               — block <base> tag hijacking.
//   frame-ancestors 'none'        — disallow embedding (clickjacking).
// GitHub-issue navigations are <a target="_blank"> link clicks, which are
// navigations (not subresource loads) and need no allowlist here.
const PROD_CSP = [
  "default-src 'self'",
  "script-src 'self' 'wasm-unsafe-eval'",
  "style-src 'self' 'unsafe-inline'",
  "img-src 'self' data:",
  "font-src 'self'",
  "connect-src 'self'",
  "object-src 'none'",
  "base-uri 'self'",
  "frame-ancestors 'none'",
].join('; ');

// Inject the CSP meta tag into the production build only. `vite dev` needs an
// HMR WebSocket back to localhost that a strict `connect-src 'self'` would
// block, and a `<meta>` CSP cannot be relaxed per-environment, so it is
// emitted at build time. (Mirrors the sibling aozora playground.)
function cspInProd(): Plugin {
  return {
    name: 'csp-in-prod',
    apply: 'build',
    transformIndexHtml: {
      order: 'pre',
      handler(html) {
        return html.replace(
          '<head>',
          `<head>\n    <meta http-equiv="Content-Security-Policy" content="${PROD_CSP}">`,
        );
      },
    },
  };
}

// Served at https://p4suta.github.io/aozora-flavored-markdown/playground/ in production.
// During `vite dev` we mount at root so assets resolve cleanly without
// the path prefix the GitHub Pages deploy demands.
//
// vite-plugin-wasm consumes wasm-pack `--target bundler` output
// (ESM-integrated .wasm with a top-level `await init()`). The companion
// vite-plugin-top-level-await is intentionally absent: per vite-plugin-wasm's
// docs the TLA plugin is only needed for non-`esnext` build targets, and
// `build.target` below is `esnext`, so the top-level await flows through
// natively — the only browsers that run this wasm support it. (vite 8
// dropped the bundled esbuild that the TLA plugin's transform relied on,
// so keeping it would have meant re-adding esbuild for no benefit.)
export default defineConfig(({ command }) => ({
  plugins: [wasm(), solid(), cspInProd()],
  base: command === 'build' ? '/aozora-flavored-markdown/playground/' : '/',
  server: {
    host: '0.0.0.0',
    port: 5173,
    strictPort: true,
    fs: {
      // crates/aozora-flavored-markdown/theme/*.css lives outside
      // playground/, in the crate that owns the classes. Vite's
      // default fs.allow restricts dev-server reads to the project root;
      // widen it so the theme `?url` imports in `src/styles/theme-urls.ts`
      // resolve. Production `build` does not consult this list.
      allow: ['..'],
    },
  },
  preview: {
    host: '0.0.0.0',
    port: 5173,
    strictPort: true,
  },
  build: {
    target: 'esnext',
    sourcemap: true,
    assetsInlineLimit: 0,
    rollupOptions: {
      output: {
        // Split vendor chunks so the initial download budget isn't
        // dominated by a single 800 KB blob that includes CodeMirror,
        // Solid, the lz-string codec, and the app code together.
        // Browsers can request these in parallel, and CodeMirror in
        // particular changes less often than the app code so its
        // chunk stays cached across deploys.
        manualChunks(id) {
          if (
            id.includes('node_modules/@codemirror/') ||
            id.includes('node_modules/@lezer/') ||
            id.includes('node_modules/codemirror/')
          ) {
            return 'vendor-codemirror';
          }
          if (id.includes('node_modules/solid-js/')) {
            return 'vendor-solid';
          }
          if (id.includes('node_modules/lz-string/')) {
            return 'vendor-lz-string';
          }
          // Everything else stays in the main entry chunk; aozora-flavored-markdown-wasm is
          // its own asset via vite-plugin-wasm and not bundled in JS.
          return undefined;
        },
      },
    },
  },
  // Unit tests for the modules that hold logic rather than DOM wiring
  // (`outline`, `examples`, `share`, …). `tsc --noEmit` was the whole of the
  // static analysis over this tree; a type checker cannot say whether
  // `outlineFromIr` walks into a list item.
  test: {
    // Beside the module they test, so a module without one is visible in the
    // same directory listing.
    include: ['src/**/*.test.{ts,tsx}'],
    // `outline.ts` reads heading text back out of rendered HTML with
    // `DOMParser`, which node has not got. happy-dom over jsdom for the
    // reason the sibling aozora playground picked it: same API surface for
    // what is used here, a fraction of the start-up.
    environment: 'happy-dom',
  },
}));
