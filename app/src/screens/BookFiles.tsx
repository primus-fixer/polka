// Настройки: файлы книг на всех устройствах и в резервной копии. Переключатели — свои у каждого устройства.
import { useEffect, useState } from 'react';
import type { Nav } from '../App';
import { api, isMobileApp, message, type FilesSettings } from '../api';
import { NB, count } from '../format';
import { Cut } from '../ui';

function size(bytes: number): string {
  if (bytes < 1 << 20) return `${Math.max(1, Math.round(bytes / 1024))}${NB}КБ`;
  const mb = bytes / (1 << 20);
  return `${mb < 10 ? mb.toFixed(1).replace('.', ',') : Math.round(mb)}${NB}МБ`;
}

export default function BookFilesSection({ nav, onChange }: { nav: Nav; onChange?: () => void }) {
  const [st, setSt] = useState<FilesSettings | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    api
      .filesSettings()
      .then(setSt)
      .catch(() => undefined);
  }, []);

  const set = async (what: 'sync' | 'backup', on: boolean) => {
    setBusy(true);
    try {
      setSt(await api.filesSet(what, on));
      onChange?.();
      if (what === 'sync' && on) nav.toast('Файлы книг поедут при следующем обмене по Wi-Fi или через облако');
    } catch (e) {
      nav.toast(message(e));
    } finally {
      setBusy(false);
    }
  };

  if (!st) return null;
  return (
    <Cut id="files" title="Файлы книг" hint={st.count ? `${count(st.count, 'файл', 'файла', 'файлов')} здесь` : 'файлов нет'}>
      <p className="small muted">
        {st.count
          ? `На этом устройстве ${count(st.count, 'файл', 'файла', 'файлов')} книг — ${size(st.bytes)}.`
          : 'На этом устройстве пока нет файлов книг.'}{' '}
        Место чтения, закладки и цитаты переходят на другие устройства всегда; сами файлы — по желанию.
      </p>
      <label className="cloud-check">
        <input type="checkbox" checked={st.sync} disabled={busy} onChange={(e) => set('sync', e.target.checked)} />
        <span className="row-label">
          <span>Файлы книг на всех устройствах</span>
          <span className="small muted">
            Книги, открытые здесь, уходят на другие устройства по Wi-Fi и через облако (там — зашифрованными), а книги с других приходят сюда. Включите на тех устройствах, где хотите читать.
            {isMobileApp ? ' Файлы бывают большими — следите за мобильным трафиком.' : ''}
          </span>
        </span>
      </label>
      {!isMobileApp && (
        <label className="cloud-check">
          <input type="checkbox" checked={st.backup} disabled={busy} onChange={(e) => set('backup', e.target.checked)} />
          <span className="row-label">
            <span>Файлы книг в резервной копии</span>
            <span className="small muted">Копия станет больше на размер файлов{st.count ? ` (сейчас — ${size(st.bytes)})` : ''}, зато книги вернутся вместе с записями.</span>
          </span>
        </label>
      )}
    </Cut>
  );
}
