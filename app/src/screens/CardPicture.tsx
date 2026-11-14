import { useState } from 'react';
import type { Nav } from '../App';
import { api, type Title } from '../api';
import { drawCard } from '../cardImage';
import PictureSheet from './PictureSheet';

/** Карточка → «Поделиться»: книга или фильм картинкой; заметка — по желанию. */
export default function CardPicture({ t, nav, onClose }: { t: Title; nav: Nav; onClose: () => void }) {
  const [note, setNote] = useState(false);
  const hasNote = !!t.note?.trim();
  return (
    <PictureSheet
      title="Карточка"
      make={() => drawCard(t, api.coverBlob, note && hasNote)}
      version={`${t.id}-${t.cover ?? ''}-${t.rating ?? ''}-${t.status}-${note}`}
      name={`Полка — ${t.title.replace(/[<>:"/\\|?*]+/g, ' ').slice(0, 60).trim()}.png`}
      alt={`Карточка «${t.title}»`}
      nav={nav}
      onClose={onClose}
    >
      {hasNote && (
        <label className="cloud-check">
          <input type="checkbox" checked={note} onChange={(e) => setNote(e.target.checked)} />
          <span className="row-label">
            <span>С заметкой</span>
            <span className="small muted">Заметка попадёт на картинку — её увидят те, кому вы её отправите.</span>
          </span>
        </label>
      )}
    </PictureSheet>
  );
}
