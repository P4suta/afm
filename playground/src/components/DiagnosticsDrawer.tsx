import { type Accessor, type Component, For, Show } from 'solid-js';

import type { Diagnostic } from '../diagnostics';

interface DiagnosticsDrawerProps {
  diagnostics: Accessor<readonly Diagnostic[]>;
  open: Accessor<boolean>;
  onToggle(): void;
}

const DiagnosticsDrawer: Component<DiagnosticsDrawerProps> = (props) => {
  const count = (): number => props.diagnostics().length;

  return (
    <div class="aozora-md-pg-drawer">
      <button
        type="button"
        class="aozora-md-pg-drawer-toggle"
        aria-expanded={props.open() ? 'true' : 'false'}
        onClick={() => props.onToggle()}
      >
        <span>
          診断 (
          <span
            class="aozora-md-pg-drawer-count"
            data-empty={count() === 0 ? '1' : '0'}
          >
            {count()}
          </span>
          )
        </span>
        <span class="aozora-md-pg-drawer-arrow" aria-hidden="true">
          ▾
        </span>
      </button>
      <div class="aozora-md-pg-drawer-body" hidden={!props.open()}>
        <Show
          when={count() > 0}
          fallback={
            <ul class="aozora-md-pg-diagnostics-list">
              <li class="aozora-md-pg-diagnostics-empty">
                診断メッセージはありません
              </li>
            </ul>
          }
        >
          <ul class="aozora-md-pg-diagnostics-list">
            <For each={props.diagnostics()}>
              {(d) => (
                <li
                  class="aozora-md-pg-diagnostics-item"
                  data-level={d.severity}
                >
                  <span class="aozora-md-pg-diagnostics-level">
                    {d.severity}
                  </span>
                  <code class="aozora-md-pg-diagnostics-code">{d.code}</code>
                  <span class="aozora-md-pg-diagnostics-message">
                    {d.message}
                  </span>
                </li>
              )}
            </For>
          </ul>
        </Show>
      </div>
    </div>
  );
};

export default DiagnosticsDrawer;
