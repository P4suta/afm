import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createEditor } from './editor';
import { AozoraDocument } from './wasm-loader';

describe('CodeMirror WASM document ownership', () => {
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
    const editor = createEditor(parent, '一', () => {});

    editor.setValue('二');
    expect(freed).toHaveBeenCalledTimes(1);

    editor.view.destroy();
    expect(freed).toHaveBeenCalledTimes(2);
    parent.remove();
  });
});
