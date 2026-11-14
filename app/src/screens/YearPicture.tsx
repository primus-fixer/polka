import type { Nav } from '../App';
import { api, type Title, type YearStats } from '../api';
import { drawYear, yearTitles } from '../yearImage';
import PictureSheet from './PictureSheet';

/** «Итоги» → «Картинка года»: собрать, посмотреть и сохранить или отправить. */
export default function YearPicture({ year, stats, titles, nav, onClose }: { year: number; stats: YearStats; titles: Title[]; nav: Nav; onClose: () => void }) {
  const count = yearTitles(titles, year).length;
  return (
    <PictureSheet
      title={`Итог ${year} года`}
      make={() => drawYear({ year, stats, titles, cover: api.coverBlob })}
      version={`${year}-${titles.length}-${stats.books}-${stats.films}-${stats.series}`}
      name={`Полка — итоги ${year}.png`}
      alt={`Итог ${year} года: обложки и цифры`}
      hint={count > 18 ? `На картинке 18 обложек из ${count}: сначала с высокой оценкой.` : 'Обложки — из базы на устройстве, интернет не нужен.'}
      nav={nav}
      onClose={onClose}
    />
  );
}
