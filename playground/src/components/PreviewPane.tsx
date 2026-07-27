// Preview pane with tabs: HTML preview / IR JSON.
//
// The HTML is trusted output from aozora-md's own renderer, mounted via
// innerHTML; the `.aozora-md-root` wrapper scopes the book theme. The JSON tab
// renders the same IR the renderer already produced. The heading outline
// is a separate always-on left column (OutlinePanel), not a tab here.
// The active tab persists to localStorage.

import {
  type Accessor,
  type Component,
  createSignal,
  For,
  Show,
} from 'solid-js';

import type { IrDocument } from '../wasm-loader';
import CodeView from './CodeView';

type TabId = 'html' | 'json';

const TABS: ReadonlyArray<{ id: TabId; label: string }> = [
  { id: 'html', label: 'プレビュー' },
  { id: 'json', label: 'IR JSON' },
];

const STORAGE_KEY = 'aozora-md-playground:preview-tab';

function loadTab(): TabId {
  try {
    return globalThis.localStorage?.getItem(STORAGE_KEY) === 'json'
      ? 'json'
      : 'html';
  } catch {
    return 'html';
  }
}

interface PreviewPaneProps {
  html: Accessor<string>;
  ir: Accessor<IrDocument | null>;
}

const PreviewPane: Component<PreviewPaneProps> = (props) => {
  const [tab, setTab] = createSignal<TabId>(loadTab());

  function selectTab(id: TabId): void {
    setTab(id);
    try {
      globalThis.localStorage?.setItem(STORAGE_KEY, id);
    } catch {
      // localStorage may be unavailable (private mode / strict CSP).
    }
  }

  return (
    <div class="aozora-md-pg-preview">
      <div class="aozora-md-pg-tab-bar" role="tablist">
        <For each={TABS}>
          {(t) => (
            <button
              type="button"
              role="tab"
              class="aozora-md-pg-tab"
              aria-selected={tab() === t.id ? 'true' : 'false'}
              onClick={() => selectTab(t.id)}
            >
              {t.label}
            </button>
          )}
        </For>
      </div>
      <div class="aozora-md-pg-preview-content" data-tab={tab()}>
        <Show when={tab() === 'html'}>
          <div class="aozora-md-root" innerHTML={props.html()} />
        </Show>
        <Show when={tab() === 'json'}>
          <CodeView
            value={props.ir() ? JSON.stringify(props.ir(), null, 2) : ''}
          />
        </Show>
      </div>
    </div>
  );
};

export default PreviewPane;
