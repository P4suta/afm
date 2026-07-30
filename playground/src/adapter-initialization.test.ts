import { beforeEach, describe, expect, it, vi } from 'vitest';

const engine = vi.hoisted(() => {
  let releaseInitialization = (): void => {};
  const initialization = new Promise<void>((resolve) => {
    releaseInitialization = resolve;
  });
  return {
    initialization,
    releaseInitialization,
    initializeEngine: vi.fn(() => initialization),
    analyze: vi.fn(async () => ({
      html: '<p>ready</p>',
      diagnostics: [],
      outline: [],
    })),
    createPreview: vi.fn(() => ({
      update: vi.fn(),
      destroy: vi.fn(),
    })),
  };
});

const features = vi.hoisted(() => ({
  createEngineFeatures: vi.fn(
    (settings: {
      readonly structureHighlight: boolean;
      readonly gaijiInlayHints: boolean;
    }) => ({
      extension: [],
      structureHighlight: [],
      gaijiInlayHints: [],
      settings,
    }),
  ),
}));

vi.mock('./adapter-engine', () => engine);
vi.mock('./editor/engineFeatures', () => features);

import { afmPlaygroundAdapter } from './adapter';

describe('AFM adapter initialization boundary', () => {
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

  it('mounts and edits before WASM, then upgrades with current settings', async () => {
    const discardedParent = document.createElement('div');
    const discardedEditor = await afmPlaygroundAdapter.createEditor(
      discardedParent,
      'discarded',
      () => {},
    );
    discardedEditor.destroy();

    const parent = document.createElement('div');
    document.body.append(parent);
    const onChange = vi.fn();
    const editor = await afmPlaygroundAdapter.createEditor(
      parent,
      'initial',
      onChange,
    );

    expect(parent.querySelector('.cm-content')).toHaveTextContent('initial');
    editor.setValue('latest');
    editor.setSetting('structureHighlight', false);
    editor.setSetting('gaijiInlayHints', false);

    const initialization = afmPlaygroundAdapter.initialize();
    await vi.waitFor(() => expect(engine.initializeEngine).toHaveBeenCalled());
    await expect(
      afmPlaygroundAdapter.analyze('source', {
        revision: 1,
        signal: new AbortController().signal,
      }),
    ).rejects.toThrow('before initialization');
    expect(() =>
      afmPlaygroundAdapter.createPreview(document.createElement('div')),
    ).toThrow('before initialization');

    engine.releaseInitialization();
    await initialization;

    expect(features.createEngineFeatures).toHaveBeenCalledOnce();
    expect(features.createEngineFeatures).toHaveBeenCalledWith({
      structureHighlight: false,
      gaijiInlayHints: false,
    });
    expect(parent.querySelector('.cm-content')).toHaveTextContent('latest');
    await expect(
      afmPlaygroundAdapter.analyze('source', {
        revision: 2,
        signal: new AbortController().signal,
      }),
    ).resolves.toMatchObject({ html: '<p>ready</p>' });
    const preview = afmPlaygroundAdapter.createPreview(
      document.createElement('div'),
    );
    expect(engine.createPreview).toHaveBeenCalledOnce();
    preview.destroy();

    const readyParent = document.createElement('div');
    const readyEditor = await afmPlaygroundAdapter.createEditor(
      readyParent,
      'created after readiness',
      () => {},
    );
    expect(features.createEngineFeatures).toHaveBeenCalledTimes(2);
    readyEditor.destroy();

    const normalizedParent = document.createElement('div');
    const normalizedEditor = await afmPlaygroundAdapter.createEditor(
      normalizedParent,
      'first\r\nsecond\rthird',
      () => {},
    );
    expect(
      [...normalizedParent.querySelectorAll('.cm-line')].map(
        (line) => line.textContent,
      ),
    ).toEqual(['first', 'second', 'third']);
    normalizedEditor.setValue('one\r\ntwo');
    expect(
      [...normalizedParent.querySelectorAll('.cm-line')].map(
        (line) => line.textContent,
      ),
    ).toEqual(['one', 'two']);
    const lineEndingContext = {
      revision: 3,
      signal: new AbortController().signal,
    };
    await afmPlaygroundAdapter.analyze(
      'first\r\nsecond\rthird',
      lineEndingContext,
    );
    expect(engine.analyze).toHaveBeenLastCalledWith(
      'first\nsecond\nthird',
      lineEndingContext,
    );
    normalizedEditor.destroy();

    editor.destroy();
    editor.destroy();
    parent.remove();
  });
});
