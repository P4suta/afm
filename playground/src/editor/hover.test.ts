import { EditorState } from '@codemirror/state';
import { activateHover, EditorView, type Tooltip } from '@codemirror/view';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import { setEditorLocale } from '../i18n';
import { AozoraDocument, initializeWasm } from '../wasm-loader';
import { aozoraMdHover } from './hover';
import { parserStateField } from './parserState';

function createView(source: string): EditorView {
  const parent = document.createElement('div');
  document.body.append(parent);
  return new EditorView({
    parent,
    state: EditorState.create({
      doc: source,
      extensions: [parserStateField, aozoraMdHover],
    }),
  });
}

function destroyView(view: EditorView): void {
  view.state.field(parserStateField).doc?.free();
  const parent = view.dom.parentElement;
  view.destroy();
  parent?.remove();
}

function hoverAt(view: EditorView, pos: number): Tooltip | null {
  activateHover(view, pos, 1, { tooltip: aozoraMdHover });
  return view.state.field(aozoraMdHover.active)[0] ?? null;
}

describe('gaiji hover', () => {
  beforeAll(initializeWasm);

  afterEach(() => {
    setEditorLocale('ja');
    vi.restoreAllMocks();
    document.body.replaceChildren();
  });

  it('uses the real resolver and maps its byte span around astral text', () => {
    const gaiji = '※［＃二の字点、1-2-22］';
    const source = `😀本文 ${gaiji} 後`;
    const view = createView(source);

    try {
      const parserState = view.state.field(parserStateField);
      const resolution = parserState.gaijiResolutions[0];
      expect(resolution?.resolved).toBeTruthy();

      const tooltip = hoverAt(view, source.indexOf('二の字点'));
      expect(tooltip).not.toBeNull();
      if (!tooltip || !resolution) return;

      expect(source.slice(tooltip.pos, tooltip.end)).toBe(gaiji);
      const dom = tooltip.create(view).dom;
      expect(dom.querySelector('strong')).toHaveTextContent(
        resolution.resolved ?? '',
      );
      expect(dom).toHaveTextContent(resolution.description);
      expect(dom).toHaveTextContent(resolution.mencode ?? '');
      if (resolution.codepoint !== null) {
        expect(dom).toHaveTextContent(
          `U+${resolution.codepoint.toString(16).toUpperCase().padStart(4, '0')}`,
        );
      }
    } finally {
      destroyView(view);
    }
  });

  it('does not activate over ordinary text or an empty document', () => {
    const ordinary = createView('plain ※［＃二の字点、1-2-22］');
    const empty = createView('');

    try {
      expect(hoverAt(ordinary, 0)).toBeNull();
      expect(hoverAt(empty, 0)).toBeNull();
    } finally {
      destroyView(ordinary);
      destroyView(empty);
    }
  });

  it('treats malformed resolver output as unavailable', () => {
    const view = createView('※［＃二の字点、1-2-22］');
    vi.spyOn(AozoraDocument.prototype, 'resolveGaijiAt').mockReturnValue(
      '{not json',
    );

    try {
      expect(hoverAt(view, 1)).toBeNull();
    } finally {
      destroyView(view);
    }
  });

  it('escapes every resolver string before building tooltip markup', () => {
    const source = '😀※［＃danger］';
    const prefixBytes = new TextEncoder().encode('😀').length;
    const sourceBytes = new TextEncoder().encode(source).length;
    const view = createView(source);
    vi.spyOn(AozoraDocument.prototype, 'resolveGaijiAt').mockReturnValue(
      JSON.stringify({
        span: { start: prefixBytes, end: sourceBytes },
        description: '"><img src=x onerror=alert(1)>',
        mencode: '<&">',
        codepoint: 0x1f600,
        resolved: `<script>'"&</script>`,
      }),
    );

    try {
      const tooltip = hoverAt(view, source.indexOf('danger'));
      expect(tooltip).not.toBeNull();
      if (!tooltip) return;

      expect(source.slice(tooltip.pos, tooltip.end)).toBe('※［＃danger］');
      const dom = tooltip.create(view).dom;
      expect(dom.querySelector('script, img')).toBeNull();
      expect(dom.querySelector('strong')).toHaveTextContent(
        `<script>'"&</script>`,
      );
      expect(dom).toHaveTextContent('"><img src=x onerror=alert(1)>');
      expect(dom).toHaveTextContent('mencode: <&">');
      expect(dom).toHaveTextContent('U+1F600');
    } finally {
      destroyView(view);
    }
  });

  it('localizes an unresolved gaiji without exposing empty metadata', () => {
    const source = '※［＃unknown］';
    const sourceBytes = new TextEncoder().encode(source).length;
    const view = createView(source);
    vi.spyOn(AozoraDocument.prototype, 'resolveGaijiAt').mockReturnValue(
      JSON.stringify({
        span: { start: 0, end: sourceBytes },
        description: 'Unknown gaiji',
        mencode: null,
        codepoint: null,
        resolved: null,
      }),
    );

    try {
      setEditorLocale('en');
      const tooltip = hoverAt(view, 1);
      expect(tooltip).not.toBeNull();
      if (!tooltip) return;

      const english = tooltip.create(view).dom;
      expect(english).toHaveTextContent('(unresolved)');
      expect(english).toHaveTextContent('Unknown gaiji');
      expect(english).not.toHaveTextContent('mencode:');
      expect(english).not.toHaveTextContent('U+');

      setEditorLocale('ja');
      expect(tooltip.create(view).dom).toHaveTextContent('（未解決）');
    } finally {
      destroyView(view);
    }
  });
});
