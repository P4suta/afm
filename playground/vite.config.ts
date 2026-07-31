import { resolve } from 'node:path';
import optimizeLocales from '@react-aria/optimize-locales-plugin';
import react from '@vitejs/plugin-react';
import macros from 'unplugin-parcel-macros';
import type { Plugin } from 'vite';
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
// `innerHTML` into `.aozora-md-root` (adapter-engine.ts), but the aozora-md
// renderer (comrak + the Aozora parser) entity-escapes all text and emits no
// active markup, so the CSP is a second wall — not the primary XSS guard.
//
// Directive rationale (kept as tight as the app allows):
//   default-src 'self'            — same-origin baseline for everything.
//   script-src 'self'             — our bundle/chunks only…
//     'wasm-unsafe-eval'          — …plus WebAssembly.instantiate for the
//                                   aozora-flavored-markdown-wasm module (no JS eval/unsafe-eval).
//   style-src 'self'              — hashed CSS assets, incl. the dynamically
//                                   swapped #aozora-md-theme <link href>…
//     'unsafe-inline'             — …plus the runtime <style> tags CodeMirror
//                                   injects (no nonce path).
//   img-src 'self' data:          — favicon + inline data: URIs.
//   font-src 'self'               — no external/CDN fonts are loaded.
//   connect-src 'self'            — covers the same-origin fetch() that
//                                   vite-plugin-wasm's instantiateStreaming()
//                                   uses to pull the .wasm asset
//                                   (assetsInlineLimit: 0 ⇒ no data:/blob:).
//   object-src 'none'             — no <object>/<embed>/<applet>.
//   base-uri 'self'               — block <base> tag hijacking.
// `frame-ancestors` is deliberately absent: browsers ignore it in a meta CSP,
// and GitHub Pages cannot attach a response-header CSP. Keeping the ignored
// directive would only generate a console error without adding protection.
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

const OFFLINE_SPECTRUM_FONTS = '\0offline-spectrum-fonts';
const PRODUCTION_BASE = '/aozora-flavored-markdown/playground/';

// Spectrum's Provider includes an Adobe Typekit loader. This static GitHub
// Pages app intentionally permits only same-origin fonts/scripts, and the
// shipped Spectrum font stacks already include suitable system fallbacks.
// Replace only that network loader; Provider, its design tokens, components,
// locale handling, and color-scheme behavior remain the official S2 code.
function offlineSpectrumFonts(): Plugin {
  return {
    name: 'offline-spectrum-fonts',
    enforce: 'pre',
    resolveId(source, importer) {
      if (
        importer?.includes('@react-spectrum/s2/') &&
        /\/Provider\.(?:mjs|tsx)$/.test(importer) &&
        /^\.\/Fonts(?:\.mjs)?$/.test(source)
      ) {
        return OFFLINE_SPECTRUM_FONTS;
      }
      return null;
    },
    load(id) {
      if (id === OFFLINE_SPECTRUM_FONTS) {
        return 'export function Fonts() { return null; }';
      }
      return null;
    },
  };
}

// Served at https://p4suta.github.io/aozora-flavored-markdown/playground/ in production.
// During `vite dev` we mount at root so assets resolve cleanly without
// the path prefix the GitHub Pages deploy demands.
//
// vite-plugin-wasm consumes wasm-pack `--target bundler` output. The generated
// module has top-level await, which is supported by the declared browser
// policy; Vite downlevels the remaining JavaScript and CSS to the explicit
// targets below.
export default defineConfig(({ command, isPreview }) => ({
  plugins: [
    offlineSpectrumFonts(),
    macros.vite(),
    react(),
    {
      ...optimizeLocales.vite({ locales: ['en', 'ja'] }),
      enforce: 'pre',
    },
    wasm(),
    cspInProd(),
  ],
  resolve: {
    // The private file dependency is copied into node_modules by Bun. Point
    // Vite at its vendored source so the Spectrum style macro transforms it;
    // macro plugins intentionally skip arbitrary code under node_modules.
    alias: [
      {
        find: /^@aozora\/playground-ui$/,
        replacement: resolve('vendor/playground-ui/src/index.ts'),
      },
    ],
  },
  base: command === 'build' || isPreview ? PRODUCTION_BASE : '/',
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
    target: ['es2022', 'safari16.2'],
    cssTarget: 'safari16.2',
    modulePreload: { polyfill: false },
    sourcemap: false,
    assetsInlineLimit: 0,
    cssCodeSplit: false,
    cssMinify: 'lightningcss',
    // CodeMirror is an intentionally deferred, cache-stable chunk. The
    // compressed total and critical-path rules are enforced separately.
    chunkSizeWarningLimit: 640,
    rollupOptions: {
      output: {
        // Split vendor chunks so the initial download budget isn't
        // dominated by a single 800 KB blob that includes CodeMirror,
        // React Spectrum, the lz-string codec, and the app code together.
        // Browsers can request these in parallel, and CodeMirror in
        // particular changes less often than the app code so its
        // chunk stays cached across deploys.
        manualChunks(id) {
          if (
            /macro-(.*)\.css$/.test(id) ||
            /@react-spectrum\/s2\/.*\.css$/.test(id)
          ) {
            return 's2-styles';
          }
          if (
            id.includes('node_modules/@codemirror/') ||
            id.includes('node_modules/@lezer/') ||
            id.includes('node_modules/codemirror/')
          ) {
            return 'vendor-codemirror';
          }
          if (
            id.includes('node_modules/react/') ||
            id.includes('node_modules/react-dom/')
          ) {
            return 'vendor-react';
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
    include: [
      'scripts/**/*.test.ts',
      'src/**/*.test.{ts,tsx}',
      'vendor/playground-ui/src/**/*.test.{ts,tsx}',
    ],
    // `outline.ts` reads heading text back out of rendered HTML with
    // `DOMParser`, which node has not got. happy-dom over jsdom for the
    // reason the sibling aozora playground picked it: same API surface for
    // what is used here, a fraction of the start-up.
    environment: 'happy-dom',
    setupFiles: ['src/test-setup.ts'],
    server: {
      deps: {
        inline: [/@react-spectrum\/s2/],
      },
    },
    coverage: {
      provider: 'v8',
      reporter: ['text', 'json-summary'],
      include: ['src/**/*.{ts,tsx}', 'vendor/playground-ui/src/**/*.{ts,tsx}'],
      exclude: [
        '**/*.test.{ts,tsx}',
        '**/*.d.ts',
        // These are declarative entry/type-contract modules exercised by the
        // production build and browser suite rather than meaningful unit
        // coverage targets.
        'src/App.tsx',
        'src/main.tsx',
        'src/wasm-package-contract.ts',
        // The adapter contract is test infrastructure, not shipped runtime.
        'vendor/playground-ui/src/testing/**',
      ],
      thresholds: {
        statements: 89,
        branches: 80,
        functions: 92,
        lines: 91,
      },
    },
  },
}));
