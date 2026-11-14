/**
 * Виджет «Сейчас читаю» на главном экране Android: до трёх записей «в процессе» — обложка,
 * название, автор или следующая серия, прогресс сериала. Данные уходят в виджет, когда
 * полка меняется, пока «Полка» открыта; обложки — маленькими снимками из базы.
 */
import { api, type Title } from './api';
import { dayShort } from './format';
import { seriesLine } from './screens/Home';
import { canWidget, setWidget } from './share';
import { palette, shortBy } from './ui';
import { drawCover, loadImage } from './yearImage';

const thumbs = new Map<string, string>();
let last = '';

/** Обложка 96 × 144 в JPEG (base64) — виджету хватает. */
async function thumb(id: string): Promise<string> {
  const known = thumbs.get(id);
  if (known !== undefined) return known;
  const urls: string[] = [];
  let out = '';
  try {
    const img = await loadImage(await api.coverBlob(id), urls);
    const canvas = document.createElement('canvas');
    canvas.width = 96;
    canvas.height = 144;
    const ctx = canvas.getContext('2d');
    if (ctx) {
      drawCover(ctx, img, 0, 0, 96, 144, 6);
      out = canvas.toDataURL('image/jpeg', 0.82).split(',')[1] ?? '';
    }
  } catch {
    out = '';
  } finally {
    urls.forEach((u) => URL.revokeObjectURL(u));
  }
  thumbs.set(id, out);
  return out;
}

/** Подпись под названием: автор и с какого дня, следующая серия, год фильма. */
export function widgetLine(t: Title): string {
  if (t.kind === 'series') return seriesLine(t).text;
  const since = t.started ? `с ${dayShort(t.started)}` : '';
  if (t.kind === 'book') return [shortBy(t), since].filter(Boolean).join(' · ');
  return [t.year, since].filter(Boolean).join(' · ');
}

/** Что сейчас в процессе — сначала начатое последним. */
export function nowTitles(titles: Title[]): Title[] {
  return titles
    .filter((t) => t.status === 'doing')
    .sort((a, b) => (b.started ?? '').localeCompare(a.started ?? ''))
    .slice(0, 3);
}

export async function pushWidget(titles: Title[]): Promise<void> {
  if (!canWidget()) return;
  const items = await Promise.all(
    nowTitles(titles).map(async (t) => ({
      id: t.id,
      title: t.title,
      sub: widgetLine(t),
      pct: t.kind === 'series' ? seriesLine(t).pct : null,
      bg: palette(t.title)[0],
      cover: t.cover ? await thumb(t.cover) : '',
    })),
  );
  const json = JSON.stringify({ items });
  if (json === last) return;
  last = json;
  setWidget(json);
}
