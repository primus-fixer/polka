import { useEffect, useState } from 'react';
import type { Nav } from '../App';
import { api, inApp, message, type Title, type YearStats } from '../api';
import { canShareImage, shareImage } from '../share';
import { Icon } from '../ui';
import { drawYear, toBase64, yearTitles } from '../yearImage';

/** «Итоги» → «Картинка года»: собрать, посмотреть и сохранить или отправить. */
export default function YearPicture({ year, stats, titles, nav, onClose }: { year: number; stats: YearStats; titles: Title[]; nav: Nav; onClose: () => void }) {
  const [blob, setBlob] = useState<Blob | null>(null);
  const [url, setUrl] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const share = canShareImage();
  const name = `Полка — итоги ${year}.png`;

  useEffect(() => {
    let alive = true;
    let made: string | null = null;
    drawYear({ year, stats, titles, cover: api.coverBlob })
      .then((b) => {
        if (!alive) return;
        made = URL.createObjectURL(b);
        setBlob(b);
        setUrl(made);
      })
      .catch((e) => alive && setError(message(e)));
    return () => {
      alive = false;
      if (made) URL.revokeObjectURL(made);
    };
  }, [year, stats, titles]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && onClose();
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose]);

  const send = async () => {
    if (!blob) return;
    setBusy(true);
    try {
      const data = await toBase64(blob);
      if (share) {
        if (!shareImage(data, name)) throw new Error('«Поделиться» не открылось — попробуйте ещё раз');
      } else {
        const path = await api.saveImage(name, data);
        if (path) nav.toast(inApp ? `Картинка сохранена: ${path}` : 'Картинка скачана');
      }
    } catch (e) {
      setError(message(e));
    } finally {
      setBusy(false);
    }
  };

  const count = yearTitles(titles, year).length;
  return (
    <div className="sheet-back" role="dialog" aria-modal="true" aria-label={`Картинка итогов ${year} года`} onClick={(e) => e.target === e.currentTarget && onClose()}>
      <div className="sheet">
        <div className="sheet-head">
          <h2 className="sec">Итог {year} года</h2>
          <button type="button" className="icon-btn" aria-label="Закрыть" onClick={onClose}>
            <Icon name="x" stroke={2} />
          </button>
        </div>
        <div className="year-pic">{url ? <img src={url} alt={`Итог ${year} года: обложки и цифры`} /> : <p className="muted">{error ?? 'Собираю картинку…'}</p>}</div>
        <p className="small muted">
          {count > 18 ? `На картинке 18 обложек из ${count}: сначала с высокой оценкой.` : 'Обложки — из базы на устройстве, интернет не нужен.'}
        </p>
        {error && url && <p className="small warn-text">{error}</p>}
        <div className="key-actions">
          <button type="button" className="btn primary small" disabled={!blob || busy} onClick={send}>
            <Icon name={share ? 'share' : 'image'} size={18} />
            <span>{share ? 'Поделиться' : inApp ? 'Сохранить…' : 'Скачать'}</span>
          </button>
          <button type="button" className="link-btn" onClick={onClose}>
            Закрыть
          </button>
        </div>
      </div>
    </div>
  );
}
