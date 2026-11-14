import { useEffect, useState, type ReactNode } from 'react';
import type { Nav } from '../App';
import { api, inApp, message } from '../api';
import { canSaveToGallery, canShareImage, saveToGallery, shareImage } from '../share';
import { Icon } from '../ui';
import { toBase64 } from '../yearImage';

type Props = {
  /** Заголовок окна: «Итог 2026 года», «Карточка». */
  title: string;
  /** Как нарисовать картинку; перерисовывается, когда меняется `version`. */
  make: () => Promise<Blob>;
  version: string;
  /** Имя файла с .png. */
  name: string;
  alt: string;
  hint?: ReactNode;
  /** Настройки картинки над кнопками (например, «С заметкой»). */
  children?: ReactNode;
  nav: Nav;
  onClose: () => void;
};

/** Окно с готовой картинкой: посмотреть, сохранить на компьютере, отправить или положить в «Галерею» на телефоне. */
export default function PictureSheet({ title, make, version, name, alt, hint, children, nav, onClose }: Props) {
  const [blob, setBlob] = useState<Blob | null>(null);
  const [url, setUrl] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const share = canShareImage();
  const gallery = canSaveToGallery();

  useEffect(() => {
    let alive = true;
    let made: string | null = null;
    setError(null);
    make()
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
    // Перерисовка — по версии: функция make новая на каждом рендере.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [version]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && onClose();
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose]);

  const run = async (how: 'send' | 'gallery') => {
    if (!blob) return;
    setBusy(true);
    setError(null);
    try {
      const data = await toBase64(blob);
      if (how === 'gallery') {
        const where = saveToGallery(data, name);
        if (!where) throw new Error('В «Галерею» не сохранилось — отправьте через «Поделиться»');
        nav.toast(`Сохранено в «${where}»`);
      } else if (share) {
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

  return (
    <div className="sheet-back" role="dialog" aria-modal="true" aria-label={title} onClick={(e) => e.target === e.currentTarget && onClose()}>
      <div className="sheet">
        <div className="sheet-head">
          <h2 className="sec">{title}</h2>
          <button type="button" className="icon-btn" aria-label="Закрыть" onClick={onClose}>
            <Icon name="x" stroke={2} />
          </button>
        </div>
        <div className="year-pic">{url ? <img src={url} alt={alt} /> : <p className="muted">{error ?? 'Собираю картинку…'}</p>}</div>
        {hint && <p className="small muted">{hint}</p>}
        {children}
        {error && url && <p className="small warn-text">{error}</p>}
        <div className="key-actions sheet-actions">
          <button type="button" className="btn primary small" disabled={!blob || busy} onClick={() => run('send')}>
            <Icon name={share ? 'share' : 'image'} size={18} />
            <span>{share ? 'Поделиться' : inApp ? 'Сохранить…' : 'Скачать'}</span>
          </button>
          {gallery && (
            <button type="button" className="btn line small" disabled={!blob || busy} onClick={() => run('gallery')}>
              <Icon name="image" size={18} />
              <span>В Галерею</span>
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
