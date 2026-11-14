import type { Nav } from '../App';
import { api, type Mark, type Title } from '../api';
import { drawQuote } from '../quoteImage';
import PictureSheet from './PictureSheet';

/** Цитата → картинкой: отправить, сохранить или положить в «Галерею». */
export default function QuotePicture({ t, mark, nav, onClose }: { t: Title; mark: Mark; nav: Nav; onClose: () => void }) {
  return (
    <PictureSheet
      title="Цитата"
      make={() => drawQuote(t, mark, api.coverBlob)}
      version={`${mark.id}-${t.cover ?? ''}-${t.title}`}
      name={`Цитата — ${t.title.replace(/[<>:"/\\|?*]+/g, ' ').slice(0, 60).trim()}.png`}
      alt={`Цитата из «${t.title}»`}
      nav={nav}
      onClose={onClose}
    />
  );
}
