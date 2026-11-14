/**
 * Карточка книги или фильма картинкой — для «Поделиться»: обложка, название, автор,
 * когда прочитано или просмотрено, оценка и, по желанию, заметка. 1080 × 1350, всё по центру.
 */
import type { Title } from './api';
import { BG, INK, LINE, MUTED, NUM_FONT, TEXT_FONT, drawCover, drawPlaceholder, fit, H, loadFonts, loadImage, setSpacing, toPng, W, wrap } from './yearImage';

const PAD = 84;

/** «12 ноября 2026». */
export function fullDate(iso: string): string {
  const [y, m, d] = iso.split('-').map(Number);
  if (!y || !m || !d) return iso;
  return new Date(y, m - 1, d).toLocaleDateString('ru-RU', { day: 'numeric', month: 'long', year: 'numeric' }).replace(/\s*г\.?$/, '');
}

/** Что с записью: «Прочитано 12 ноября 2026», «Смотрю с 15 октября 2026», «Хочу прочитать». */
export function statusLine(t: Title): string {
  const book = t.kind === 'book';
  switch (t.status) {
    case 'done':
      return `${book ? 'Прочитано' : 'Просмотрено'}${t.finished ? ` ${fullDate(t.finished)}` : ''}`;
    case 'doing':
      return `${book ? 'Читаю' : 'Смотрю'}${t.started ? ` с ${fullDate(t.started)}` : ' сейчас'}`;
    case 'dropped':
      return `Брошено${t.finished ? ` ${fullDate(t.finished)}` : ''}`;
    default:
      return book ? 'Хочу прочитать' : 'Хочу посмотреть';
  }
}

const KIND = { book: 'КНИГА', film: 'ФИЛЬМ', series: 'СЕРИАЛ' } as const;

/** Автор книги, режиссёр и год фильма. */
function byLine(t: Title): string {
  const who = t.creators.slice(0, 2).join(', ');
  return t.kind === 'book' ? who : [who, t.year].filter(Boolean).join(' · ');
}

function centered(ctx: CanvasRenderingContext2D, text: string, y: number) {
  ctx.textAlign = 'center';
  ctx.fillText(text, W / 2, y);
  ctx.textAlign = 'left';
}

export async function drawCard(t: Title, cover: ((id: string) => Promise<Blob>) | null, withNote: boolean): Promise<Blob> {
  await loadFonts();
  const canvas = document.createElement('canvas');
  canvas.width = W;
  canvas.height = H;
  const ctx = canvas.getContext('2d');
  if (!ctx) throw new Error('Холст недоступен — картинку не нарисовать');
  const urls: string[] = [];
  try {
    ctx.fillStyle = BG;
    ctx.fillRect(0, 0, W, H);
    // Светлые оттенки книг и кино: на тёмном фоне читаются лучше цветов расцветки.
    const accent = t.kind === 'book' ? '#8CC2E8' : '#F0A56B';
    const note = withNote ? (t.note ?? '').trim() : '';

    ctx.fillStyle = MUTED;
    ctx.font = `500 24px ${NUM_FONT}`;
    setSpacing(ctx, 6);
    centered(ctx, 'ПОЛКА', 92);
    setSpacing(ctx, 0);

    // Обложка: крупно, по центру, с тенью.
    const ch = note ? 600 : 720;
    const cw = Math.round(ch / 1.5);
    const cx = (W - cw) / 2;
    const cy = 136;
    let img: HTMLImageElement | null = null;
    if (t.cover && cover) {
      try {
        img = await loadImage(await cover(t.cover), urls);
      } catch {
        img = null;
      }
    }
    ctx.save();
    ctx.shadowColor = 'rgba(0,0,0,0.45)';
    ctx.shadowBlur = 48;
    ctx.shadowOffsetY = 18;
    ctx.fillStyle = BG;
    ctx.beginPath();
    ctx.roundRect(cx, cy, cw, ch, 18);
    ctx.fill();
    ctx.restore();
    if (img) drawCover(ctx, img, cx, cy, cw, ch, 18);
    else drawPlaceholder(ctx, t, cx, cy, cw, ch, 18);

    let y = cy + ch + 70;
    ctx.fillStyle = accent;
    ctx.font = `500 24px ${NUM_FONT}`;
    setSpacing(ctx, 5);
    centered(ctx, KIND[t.kind], y);
    setSpacing(ctx, 0);

    y += 66;
    ctx.fillStyle = INK;
    ctx.font = `600 52px ${TEXT_FONT}`;
    for (const line of wrap(ctx, t.title, W - 2 * PAD, 2)) {
      centered(ctx, line, y);
      y += 62;
    }

    const by = byLine(t);
    if (by) {
      ctx.fillStyle = MUTED;
      ctx.font = `400 30px ${TEXT_FONT}`;
      centered(ctx, fit(ctx, by, W - 2 * PAD), y + 2);
      y += 50;
    }

    // Когда и с какой оценкой.
    ctx.fillStyle = INK;
    ctx.font = `400 30px ${TEXT_FONT}`;
    const status = statusLine(t) + (t.rating ? `   ★ ${t.rating}/10` : '');
    centered(ctx, fit(ctx, status, W - 2 * PAD), y + 12);
    y += 40;

    if (note) {
      y += 24;
      ctx.fillStyle = LINE;
      ctx.fillRect(W / 2 - 60, y, 120, 2);
      y += 52;
      ctx.fillStyle = '#DCE4EA';
      ctx.font = `400 30px ${TEXT_FONT}`;
      const room = Math.max(1, Math.min(4, Math.floor((H - 96 - y) / 42)));
      for (const line of wrap(ctx, `«${note}»`, W - 2 * PAD, room)) {
        centered(ctx, line, y);
        y += 42;
      }
    }

    ctx.fillStyle = MUTED;
    ctx.font = `400 22px ${TEXT_FONT}`;
    centered(ctx, 'Полка — книги, фильмы и сериалы', H - 44);
    return await toPng(canvas);
  } finally {
    urls.forEach((u) => URL.revokeObjectURL(u));
  }
}
