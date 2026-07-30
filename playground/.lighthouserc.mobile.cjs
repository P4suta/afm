const { chromium } = require('playwright');
const port = process.env.PLAYGROUND_PORT || '4173';
const url = `http://127.0.0.1:${port}/aozora-flavored-markdown/playground/`;

module.exports = {
  ci: {
    collect: {
      chromePath: chromium.executablePath(),
      numberOfRuns: 3,
      startServerCommand: 'bun run preview:production',
      startServerReadyPattern: `http://127.0.0.1:${port}`,
      url: [url],
      settings: {
        chromeFlags: '--headless --no-sandbox --disable-dev-shm-usage',
      },
    },
    assert: {
      assertions: {
        'categories:accessibility': ['error', { minScore: 1 }],
        'categories:best-practices': ['error', { minScore: 1 }],
        // The basic editor remains deferred, while the mobile-readable default
        // sample keeps late CodeMirror content from replacing the initial LCP.
        'categories:performance': ['error', { minScore: 0.95 }],
        'categories:seo': ['error', { minScore: 1 }],
        'cumulative-layout-shift': ['error', { maxNumericValue: 0.05 }],
        'largest-contentful-paint': ['error', { maxNumericValue: 2400 }],
        'total-blocking-time': ['error', { maxNumericValue: 200 }],
      },
    },
    upload: {
      outputDir: '.lighthouseci/reports/mobile',
      target: 'filesystem',
    },
  },
};
