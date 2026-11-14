// Общее для читалок EPUB/FB2 и PDF: панели, списки закладок и цитат, подсветка, время до конца,
// «Назад» на Android и экран, который не гаснет, пока читаете.
import { useEffect, useRef, type ReactNode } from 'react';
import { isMobileApp, type Mark } from '../api';
import { plural } from '../format';
import { Icon } from '../ui';
import { THEMES, type ReaderTheme } from './settings';

export function timeLeft(minutes: number): string {
  if (minutes < 1) return 'меньше минуты';
  if (minutes < 60) return plural(Math.max(1, Math.round(minutes)), 'минута', 'минуты', 'минут');
  const h = Math.floor(minutes / 60);
  const m = Math.round((minutes - h * 60) / 5) * 5;
  return h < 10 && m > 0 && m < 60 ? `${h} ч ${m} мин` : `${Math.round(minutes / 60)} ч`;
}

/** Процент для строки состояния: до 10 % — с десятыми. */
export const pct = (n: number) => `${n < 10 ? n.toFixed(1).replace('.', ',') : Math.floor(n)} %`;

/** Подсветка найденного (CSS Custom Highlight API; где его нет — без подсветки). */
export type Highlights = { set(name: string, h: unknown): void; delete(name: string): void };
export function highlights(): Highlights | null {
  const reg = (CSS as unknown as { highlights?: Highlights }).highlights;
  return reg && typeof (window as unknown as { Highlight?: unknown }).Highlight === 'function' ? reg : null;
}
export function highlight(name: string, ranges: Range[]) {
  const reg = highlights();
  if (!reg) return;
  if (!ranges.length) return reg.delete(name);
  const H = (window as unknown as { Highlight: new (...r: Range[]) => unknown }).Highlight;
  reg.set(name, new H(...ranges));
}

export function Sheet({ title, side, onClose, children }: { title: string; side?: boolean; onClose: () => void; children: ReactNode }) {
  return (
    <div className="rd-sheet-back" onClick={onClose}>
      <div className={`rd-sheet${side ? ' rd-right' : ''}`} role="dialog" aria-label={title} onClick={(e) => e.stopPropagation()}>
        <div className="rd-sheet-head">
          <h2>{title}</h2>
          <button type="button" className="icon-btn" aria-label="Закрыть" onClick={onClose}>
            <Icon name="x" />
          </button>
        </div>
        {children}
      </div>
    </div>
  );
}

/** Закладки или цитаты в панели: открыть место, цитату — картинкой, удалить (со вторым нажатием). */
export function MarkList({ list, empty, where, confirmId, onOpen, onShare, onRemove }: { list: Mark[]; empty: string; where: (m: Mark) => string; confirmId: string | null; onOpen: (m: Mark) => void; onShare: (m: Mark) => void; onRemove: (m: Mark) => void }) {
  if (!list.length) return <p className="muted">{empty}</p>;
  return (
    <ul className="rd-marks">
      {list.map((m) => (
        <li key={m.id}>
          <button type="button" className="rd-mark" onClick={() => onOpen(m)}>
            <span className="small muted">{where(m)}</span>
            <span className={m.kind === 'quote' ? 'rd-quote-text' : 'rd-mark-text'}>{m.kind === 'quote' ? `«${m.text}»` : m.text || 'Закладка'}</span>
          </button>
          <span className="rd-mark-tools">
            {m.kind === 'quote' && (
              <button type="button" className="icon-btn" aria-label="Цитата картинкой" onClick={() => onShare(m)}>
                <Icon name="share" size={18} />
              </button>
            )}
            <button type="button" className={confirmId === m.id ? 'link-btn warn-text small' : 'icon-btn'} aria-label={m.kind === 'quote' ? 'Удалить цитату' : 'Удалить закладку'} onClick={() => onRemove(m)}>
              {confirmId === m.id ? 'Удалить?' : <Icon name="trash" size={18} />}
            </button>
          </span>
        </li>
      ))}
    </ul>
  );
}

export const THEME_ORDER: [ReaderTheme, string][] = [
  ['auto', 'Как в «Полке»'],
  ['light', THEMES.light.label],
  ['sepia', THEMES.sepia.label],
  ['dark', THEMES.dark.label],
];

/** Android: системная кнопка «Назад» — сначала закрывает панели читалки, потом книгу. */
export function useAndroidBack(handler: () => void) {
  const ref = useRef(handler);
  ref.current = handler;
  useEffect(() => {
    if (!isMobileApp) return;
    let listener: { unregister(): Promise<void> } | null = null;
    let gone = false;
    import('@tauri-apps/api/app')
      .then(({ onBackButtonPress }) => onBackButtonPress(() => ref.current()))
      .then((l) => {
        if (gone) l.unregister().catch(() => undefined);
        else listener = l;
      })
      .catch(() => undefined);
    return () => {
      gone = true;
      listener?.unregister().catch(() => undefined);
    };
  }, []);
}

/** Экран не гаснет, пока книга открыта; ушли из приложения — `onHide` (записать место). */
export function useWakeLock(onHide: () => void) {
  const ref = useRef(onHide);
  ref.current = onHide;
  useEffect(() => {
    let lock: { release(): Promise<void> } | null = null;
    const wake = async () => {
      try {
        lock = (await (navigator as unknown as { wakeLock?: { request(t: 'screen'): Promise<{ release(): Promise<void> }> } }).wakeLock?.request('screen')) ?? null;
      } catch {
        lock = null;
      }
    };
    wake();
    const onVis = () => {
      if (document.visibilityState === 'hidden') ref.current();
      else wake();
    };
    document.addEventListener('visibilitychange', onVis);
    return () => {
      document.removeEventListener('visibilitychange', onVis);
      lock?.release().catch(() => undefined);
    };
  }, []);
}
