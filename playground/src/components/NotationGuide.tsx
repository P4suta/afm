// Notation reference modal, ported from the sibling aozora playground's
// components/NotationGuide.tsx. Renders `notation-guide.md` (aozora-md-specific:
// CommonMark + GFM + aozora) to HTML once via `marked`, with a TOC, a
// focus trap, body-scroll lock, and Escape / backdrop close.

import { marked, type Tokens } from 'marked';
import {
  createEffect,
  createMemo,
  createSignal,
  For,
  onCleanup,
  Show,
} from 'solid-js';

import notationGuideSource from '../notation-guide.md?raw';

interface NotationGuideProps {
  open: boolean;
  onClose: () => void;
}

interface TocEntry {
  id: string;
  text: string;
  level: number;
}

/**
 * Pre-compute markdown → HTML + TOC once. The TOC covers h2 / h3 only.
 * Stable ids are the running `nh-${index}` so Japanese headings sidestep
 * slugify issues; the inject pass and the TOC pass share one counter so
 * the ids line up.
 */
const { html: RENDERED_HTML, toc: TOC } = (() => {
  marked.setOptions({ gfm: true, breaks: false });

  const tokens = marked.lexer(notationGuideSource);
  const toc: TocEntry[] = [];
  let counter = 0;
  for (const tok of tokens) {
    if (tok.type !== 'heading') continue;
    const h = tok as Tokens.Heading;
    if (h.depth !== 2 && h.depth !== 3) continue;
    toc.push({ id: `nh-${counter}`, text: h.text, level: h.depth });
    counter++;
  }

  const raw = marked.parse(notationGuideSource, { async: false }) as string;
  let inject = 0;
  const html = raw.replace(/<h([23])(\s[^>]*)?>/g, (_match, lvl, attrs) => {
    const id = `nh-${inject++}`;
    return `<h${lvl} id="${id}"${attrs ?? ''}>`;
  });
  return { html, toc };
})();

export default function NotationGuide(props: NotationGuideProps) {
  const html = createMemo(() => RENDERED_HTML);
  const [activeId, setActiveId] = createSignal<string>(TOC[0]?.id ?? '');
  let bodyRef!: HTMLDivElement;
  let modalRef!: HTMLDivElement;

  // Escape close, body scroll lock, focus trap — only while open.
  createEffect(() => {
    if (!props.open) return;
    const previouslyFocused = document.activeElement as HTMLElement | null;
    const prevOverflow = document.body.style.overflow;
    document.body.style.overflow = 'hidden';

    queueMicrotask(() => {
      if (!modalRef) return;
      collectFocusables(modalRef)[0]?.focus();
    });

    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        props.onClose();
        return;
      }
      if (e.key !== 'Tab' || !modalRef) return;
      const focusables = collectFocusables(modalRef);
      if (focusables.length === 0) return;
      const first = focusables[0]!;
      const last = focusables[focusables.length - 1]!;
      if (e.shiftKey && document.activeElement === first) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && document.activeElement === last) {
        e.preventDefault();
        first.focus();
      }
    };
    window.addEventListener('keydown', onKey);
    onCleanup(() => {
      document.body.style.overflow = prevOverflow;
      window.removeEventListener('keydown', onKey);
      previouslyFocused?.focus?.();
    });
  });

  function collectFocusables(container: HTMLElement): HTMLElement[] {
    return Array.from(
      container.querySelectorAll<HTMLElement>(
        'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])',
      ),
    ).filter((el) => el.offsetParent !== null);
  }

  function jumpTo(id: string) {
    if (!bodyRef) return;
    const target = bodyRef.querySelector<HTMLElement>(`#${CSS.escape(id)}`);
    if (!target) return;
    const offset = target.offsetTop - bodyRef.offsetTop;
    bodyRef.scrollTo({ top: offset - 8, behavior: 'smooth' });
    setActiveId(id);
  }

  return (
    <Show when={props.open}>
      {/* biome-ignore lint/a11y/noStaticElementInteractions: the backdrop is
          the dialog's scrim, not a control — the dialog itself carries
          role/aria-modal, and giving the scrim a role of its own would put a
          second landmark in front of it.
          biome-ignore lint/a11y/useKeyWithClickEvents: the keyboard path to
          this same action is wired above — the `keydown` listener installed
          while the dialog is open closes it on Escape, which is what a scrim
          click does. A key handler here would be a second binding for one
          action. */}
      <div
        class="aozora-md-pg-guide-backdrop"
        onClick={(e) => {
          if (e.target === e.currentTarget) props.onClose();
        }}
      >
        <div
          class="aozora-md-pg-guide-modal"
          role="dialog"
          aria-modal="true"
          aria-label="aozora-md 記法リファレンス"
          ref={modalRef}
        >
          <header class="aozora-md-pg-guide-header">
            <h2>📖 aozora-md 記法リファレンス</h2>
            <button
              type="button"
              class="aozora-md-pg-guide-close"
              onClick={props.onClose}
              aria-label="閉じる"
            >
              ×
            </button>
          </header>
          <div class="aozora-md-pg-guide-content">
            <Show when={TOC.length > 0}>
              <nav class="aozora-md-pg-guide-toc" aria-label="目次">
                <ul>
                  <For each={TOC}>
                    {(entry) => (
                      <li class={`aozora-md-pg-toc-l${entry.level}`}>
                        <button
                          type="button"
                          class={
                            activeId() === entry.id
                              ? 'aozora-md-pg-toc-link active'
                              : 'aozora-md-pg-toc-link'
                          }
                          onClick={() => jumpTo(entry.id)}
                        >
                          {entry.text}
                        </button>
                      </li>
                    )}
                  </For>
                </ul>
              </nav>
            </Show>
            <div
              class="aozora-md-pg-guide-body"
              ref={bodyRef}
              innerHTML={html()}
            />
          </div>
        </div>
      </div>
    </Show>
  );
}
