// Своя полоса окна на компьютере: без названия и значка, того же цвета, что и «Полка», —
// за неё окно перетаскивают (двойной щелчок — развернуть), справа — свернуть, развернуть, закрыть.
// Системная рамка выключена в tauri.conf.json (decorations: false). На телефоне и в демо полосы нет.
import { useEffect, useState } from 'react';
import { inApp, isMobileApp } from './api';

/** Полоса нужна только в приложении на компьютере; в демо её можно посмотреть по адресу с «?titlebar». */
export const customTitleBar = (inApp && !isMobileApp) || (!inApp && typeof location !== 'undefined' && new URLSearchParams(location.search).has('titlebar'));

/** Цвет полосы в читалке — как у её темы; `null` — как у «Полки». */
export function setTitleBarColors(bg: string | null, ink?: string) {
  const s = document.documentElement.style;
  if (bg) {
    s.setProperty('--tb-bg', bg);
    if (ink) s.setProperty('--tb-ink', ink);
  } else {
    s.removeProperty('--tb-bg');
    s.removeProperty('--tb-ink');
  }
}

type Win = Awaited<ReturnType<typeof load>>;
const load = async () => (await import('@tauri-apps/api/window')).getCurrentWindow();

export default function TitleBar() {
  const [win, setWin] = useState<Win | null>(null);
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    if (!customTitleBar || !inApp) return;
    let off: (() => void) | undefined;
    let gone = false;
    load()
      .then(async (w) => {
        if (gone) return;
        setWin(w);
        const sync = () => w.isMaximized().then(setMaximized, () => undefined);
        sync();
        off = await w.onResized(sync);
      })
      .catch(() => undefined);
    return () => {
      gone = true;
      off?.();
    };
  }, []);

  if (!customTitleBar) return null;
  return (
    <div className="titlebar" data-tauri-drag-region>
      <div className="tb-drag" data-tauri-drag-region />
      <button type="button" className="tb-btn" aria-label="Свернуть" title="Свернуть" onClick={() => win?.minimize()}>
        <svg viewBox="0 0 10 10" aria-hidden="true">
          <path d="M0 5.5h10" />
        </svg>
      </button>
      <button type="button" className="tb-btn" aria-label={maximized ? 'Восстановить' : 'Развернуть'} title={maximized ? 'Восстановить' : 'Развернуть'} onClick={() => win?.toggleMaximize()}>
        {maximized ? (
          <svg viewBox="0 0 10 10" aria-hidden="true">
            <path d="M2.5 2.5V.5h7v7h-2M.5 2.5h7v7h-7z" />
          </svg>
        ) : (
          <svg viewBox="0 0 10 10" aria-hidden="true">
            <path d="M.5.5h9v9h-9z" />
          </svg>
        )}
      </button>
      <button type="button" className="tb-btn tb-close" aria-label="Закрыть" title="Закрыть" onClick={() => win?.close()}>
        <svg viewBox="0 0 10 10" aria-hidden="true">
          <path d="M.5.5l9 9M9.5.5l-9 9" />
        </svg>
      </button>
    </div>
  );
}
