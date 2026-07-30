import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import 'aozora-flavored-markdown-wasm';
import { createEditor } from './editor';
import { createEngineFeatures } from './editor/engineFeatures';
import { AozoraDocument } from './wasm-loader';

describe('CodeMirror editor lifecycle', () => {
  beforeEach(() => {
    vi.stubGlobal(
      'ResizeObserver',
      class {
        observe(): void {}
        unobserve(): void {}
        disconnect(): void {}
      },
    );
  });

  afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  it('frees the replaced document and the final document on view destruction', () => {
    const freed = vi.spyOn(AozoraDocument.prototype, 'free');
    const parent = document.createElement('div');
    document.body.append(parent);
    const editor = createEditor(parent, '一', () => {}, createEngineFeatures);

    editor.setValue('二');
    expect(freed).toHaveBeenCalledTimes(1);

    editor.destroy();
    expect(freed).toHaveBeenCalledTimes(2);
    parent.remove();
  });

  it('reports author edits without echoing controlled values', () => {
    const onChange = vi.fn();
    const parent = document.createElement('div');
    document.body.append(parent);
    const editor = createEditor(parent, '一', onChange);

    editor.setValue('一');
    expect(onChange).not.toHaveBeenCalled();
    editor.setValue('二');
    expect(onChange).not.toHaveBeenCalled();
    editor.revealRange({ start: 0, end: 1 });
    expect(editor.runCommand('aozora-md.wrap.ruby')).toBe(true);
    expect(onChange).toHaveBeenCalledOnce();
    expect(onChange).toHaveBeenLastCalledWith('｜二《》');

    editor.destroy();
    parent.remove();
  });

  it('selects UTF-16 ranges and executes every published wrap command', () => {
    const onChange = vi.fn();
    const parent = document.createElement('div');
    document.body.append(parent);
    const editor = createEditor(parent, '漢字', onChange);

    editor.revealRange({ start: 0, end: 2 });
    expect(editor.runCommand('aozora-md.wrap.ruby')).toBe(true);
    expect(onChange).toHaveBeenLastCalledWith('｜漢字《》');
    expect(editor.runCommand('unknown')).toBe(false);
    expect(() => editor.revealRange({ start: -10, end: 10_000 })).not.toThrow();

    editor.destroy();
    parent.remove();
  });

  it('reconfigures authoring assists without recreating the editor', () => {
    const parent = document.createElement('div');
    document.body.append(parent);
    const editor = createEditor(parent, '※［＃二の字点、1-2-22］', () => {});

    editor.setSetting('structureHighlight', false);
    editor.setSetting('gaijiInlayHints', false);
    editor.enableEngineFeatures(createEngineFeatures);
    editor.enableEngineFeatures(createEngineFeatures);

    for (const id of ['structureHighlight', 'gaijiInlayHints']) {
      expect(() => editor.setSetting(id, false)).not.toThrow();
      expect(() => editor.setSetting(id, true)).not.toThrow();
    }
    expect(() => editor.setSetting('unknown', true)).not.toThrow();

    editor.destroy();
    editor.destroy();
    parent.remove();
  });

  it('preserves current content and latest settings during the engine upgrade', () => {
    const onChange = vi.fn();
    const parent = document.createElement('div');
    document.body.append(parent);
    const editor = createEditor(parent, 'initial', onChange);

    editor.setValue('latest');
    editor.setSetting('structureHighlight', false);
    editor.setSetting('gaijiInlayHints', false);
    editor.enableEngineFeatures(createEngineFeatures);

    expect(parent.querySelector('.cm-content')).toHaveTextContent('latest');
    expect(onChange).not.toHaveBeenCalled();

    editor.destroy();
    expect(() =>
      editor.enableEngineFeatures(createEngineFeatures),
    ).not.toThrow();
    expect(() => editor.setSetting('structureHighlight', true)).not.toThrow();
    parent.remove();
  });
});
