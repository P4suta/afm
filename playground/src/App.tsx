// Top-level Solid component. Owns the shared signals:
//   - source     : current editor text
//   - rendered   : { html, diagnostics } from the wasm pipeline
//   - theme      : 'vertical' | 'horizontal' (driven by createTheme)
// and wires the toolbar / editor / preview / diagnostics together.

import { EditorView } from '@codemirror/view';
import { type Component, createMemo, createSignal, onMount } from 'solid-js';
import { createColorScheme } from './color-scheme';
import DiagnosticsDrawer from './components/DiagnosticsDrawer';
import EditorPane from './components/EditorPane';
import NotationGuide from './components/NotationGuide';
import OutlinePanel from './components/OutlinePanel';
import PreviewPane from './components/PreviewPane';
import Toolbar from './components/Toolbar';
import { loadExamples } from './examples';
import { outlineFromIr } from './outline';
import { copyShareLink, decodeSourceFromHash } from './share';
import { createTheme } from './theme-toggle';
import {
  type Diagnostic,
  hashSource,
  type IrDocument,
  render,
} from './wasm-loader';

const FALLBACK_SOURCE =
  '# aozora-md playground\n\nここに ｜文章《ぶんしょう》 を書いてみてください。\n';

interface Rendered {
  readonly html: string;
  readonly diagnostics: readonly Diagnostic[];
  readonly ir: IrDocument | null;
}

const EMPTY_RENDER: Rendered = { html: '', diagnostics: [], ir: null };

function pickInitial(examples: ReturnType<typeof loadExamples>): string {
  const fromHash = decodeSourceFromHash(globalThis.location.hash);
  if (fromHash !== null) return fromHash;
  return examples[0]?.source ?? FALLBACK_SOURCE;
}

const App: Component = () => {
  const examples = loadExamples();
  const [source, setSource] = createSignal(pickInitial(examples));
  const [rendered, setRendered] = createSignal<Rendered>(EMPTY_RENDER);
  const [drawerOpen, setDrawerOpen] = createSignal(true);
  const [toast, setToast] = createSignal<{
    message: string;
    ok: boolean;
  } | null>(null);
  const [editorView, setEditorView] = createSignal<EditorView | null>(null);
  const [guideOpen, setGuideOpen] = createSignal(false);

  const theme = createTheme();
  const colorScheme = createColorScheme();

  // Render gate: hashSource short-circuits identical source runs through
  // the cache instead of re-invoking the wasm pipeline. Cheap to call
  // (xxh3-64) so it stays in the synchronous reactive graph.
  let lastHash: bigint | null = null;
  let pending: ReturnType<typeof setTimeout> | null = null;

  function runRender(text: string): void {
    let h: bigint;
    try {
      h = hashSource(text);
    } catch (err) {
      setRendered({
        html: '',
        ir: null,
        diagnostics: [
          {
            severity: 'error',
            source: 'internal',
            code: 'playground::hash',
            message: String(err),
            span: { start: 0, end: 0 },
          },
        ],
      });
      return;
    }
    if (lastHash === h) return;
    lastHash = h;

    try {
      const result = render(text);
      setRendered({
        html: result.html,
        diagnostics: result.diagnostics,
        ir: result.ir,
      });
      if (result.diagnostics.length > 0) setDrawerOpen(true);
    } catch (err) {
      setRendered({
        html: '',
        ir: null,
        diagnostics: [
          {
            severity: 'error',
            source: 'internal',
            code: 'playground::render',
            message: String(err),
            span: { start: 0, end: 0 },
          },
        ],
      });
      setDrawerOpen(true);
    }
  }

  function scheduleRender(text: string): void {
    if (pending !== null) clearTimeout(pending);
    pending = setTimeout(() => {
      pending = null;
      runRender(text);
    }, 100);
  }

  function loadExample(slug: string): void {
    const ex = examples.find((e) => e.slug === slug);
    if (ex === undefined) return;
    setSource(ex.source);
    if (pending !== null) clearTimeout(pending);
    pending = null;
    lastHash = null;
    runRender(ex.source);
  }

  function flashToast(message: string, ok: boolean): void {
    setToast({ message, ok });
    setTimeout(() => setToast(null), 1800);
  }

  function share(): void {
    void copyShareLink(source()).then(
      () => flashToast('共有リンクをコピーしました', true),
      (err: unknown) => flashToast(`コピーに失敗: ${String(err)}`, false),
    );
  }

  onMount(() => {
    // Solid's `render()` APPENDS to the mount node — it does not replace
    // existing children. The inline `<div id="boot-overlay">` from
    // index.html therefore survives the mount and (because shell.css
    // styles it `position: fixed; inset: 0; z-index: 10`) covers the
    // whole viewport until we explicitly take it down. Removing it here
    // means the user keeps seeing the "aozora-md を読み込み中…" message until
    // the first render lands; once we reach this callback the editor +
    // preview are already in the DOM and the overlay's job is done.
    document.getElementById('boot-overlay')?.remove();
    runRender(source());
  });

  const html = createMemo(() => rendered().html);
  const diagnostics = createMemo(() => rendered().diagnostics);
  const ir = createMemo(() => rendered().ir);
  const outline = createMemo(() => {
    const doc = ir();
    return doc ? outlineFromIr(doc) : [];
  });

  // Scroll the editor to a 1-based source line and put the cursor there
  // (outline click target).
  function jumpToLine(sourceLine: number): void {
    const view = editorView();
    if (!view) return;
    const lineCount = view.state.doc.lines;
    const n = Math.min(Math.max(sourceLine, 1), lineCount);
    const line = view.state.doc.line(n);
    view.dispatch({
      selection: { anchor: line.from },
      effects: EditorView.scrollIntoView(line.from, { y: 'start' }),
    });
    view.focus();
  }

  return (
    <>
      <Toolbar
        themeMode={theme.mode}
        onThemeChange={(m) => theme.setMode(m)}
        colorSchemePref={colorScheme.pref}
        onCycleColorScheme={() => colorScheme.cyclePref()}
        examples={examples}
        onLoadExample={loadExample}
        onShare={share}
        editorView={editorView}
        onShowGuide={() => setGuideOpen(true)}
      />
      <main class="aozora-md-pg-panes">
        <aside class="aozora-md-pg-outline" aria-label="アウトライン">
          <OutlinePanel entries={outline()} onJump={jumpToLine} />
        </aside>
        <section
          class="aozora-md-pg-pane aozora-md-pg-pane-editor"
          aria-label="エディタ"
        >
          <EditorPane
            value={source()}
            onChange={(value) => {
              setSource(value);
              scheduleRender(value);
            }}
            onReady={setEditorView}
          />
        </section>
        <section
          class="aozora-md-pg-pane aozora-md-pg-pane-preview"
          aria-label="プレビュー"
        >
          <PreviewPane html={html} ir={ir} />
          <DiagnosticsDrawer
            diagnostics={diagnostics}
            open={drawerOpen}
            onToggle={() => setDrawerOpen((v) => !v)}
          />
        </section>
      </main>
      <footer class="aozora-md-pg-footer">
        <span>
          Powered by{' '}
          <a
            href="https://github.com/P4suta/aozora-flavored-markdown"
            target="_blank"
            rel="noopener"
          >
            aozora-md
          </a>{' '}
          — Aozora Flavored Markdown
        </span>
      </footer>
      {toast() !== null && (
        <div class="aozora-md-pg-toast" data-ok={toast()!.ok ? '1' : '0'}>
          {toast()!.message}
        </div>
      )}
      <NotationGuide open={guideOpen()} onClose={() => setGuideOpen(false)} />
    </>
  );
};

export default App;
