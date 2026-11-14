/**
 * Итог года картинкой: коллаж обложек за год, сколько книг и фильмов, страницы и часы, лучшее.
 * Рисуется на холсте из обложек в базе — без интернета. 1080 × 1350: удобно для мессенджеров и историй.
 */
import type { Title, YearStats } from './api';
import { thousands } from './format';
import { palette, shortBy } from './ui';

export const W = 1080;
export const H = 1350;
const PAD = 72;
const GAP = 20;
/** Больше обложек в коллаж не помещается: остальные — «+N» на последней. */
const MAX_TILES = 18;

export const BG = '#15232D';
export const INK = '#FFFFFF';
export const MUTED = '#A9B8C2';
export const LINE = 'rgba(255,255,255,0.14)';

export const NUM_FONT = "'Geologica', 'Golos Text', Arial, sans-serif";
export const TEXT_FONT = "'Golos Text', 'Segoe UI', Roboto, Arial, sans-serif";

export type YearInput = {
  year: number;
  stats: YearStats;
  /** Всё на полке: из него — что закончено за год. */
  titles: Title[];
  /** Обложка из базы: снимок по номеру. */
  cover: (id: string) => Promise<Blob>;
};

/** Законченное за год: сначала с высокой оценкой, потом недавнее. */
export function yearTitles(titles: Title[], year: number): Title[] {
  return titles
    .filter((t) => t.status === 'done' && t.finished?.startsWith(String(year)))
    .sort((a, b) => (b.rating ?? 0) - (a.rating ?? 0) || (b.finished ?? '').localeCompare(a.finished ?? ''));
}

/** Сетка коллажа: столько колонок, чтобы обложки вышли крупнее всего в отведённом месте. */
export function grid(n: number, width: number, height: number): { cols: number; rows: number; w: number; h: number } {
  let best = { cols: 1, rows: 1, w: 0, h: 0 };
  for (let cols = 1; cols <= 6; cols++) {
    const rows = Math.ceil(n / cols);
    if (rows > 3) continue;
    const w = Math.min((width - (cols - 1) * GAP) / cols, (height - (rows - 1) * GAP) / rows / 1.5);
    if (w > best.w) best = { cols, rows, w, h: w * 1.5 };
  }
  return best;
}

export function setSpacing(ctx: CanvasRenderingContext2D, px: number) {
  // Не везде есть — без него просто плотнее.
  if ('letterSpacing' in ctx) (ctx as CanvasRenderingContext2D & { letterSpacing: string }).letterSpacing = `${px}px`;
}

/** Строка, урезанная до ширины: «Длинное назва…». */
export function fit(ctx: CanvasRenderingContext2D, text: string, max: number): string {
  if (ctx.measureText(text).width <= max) return text;
  let s = text;
  while (s.length > 1 && ctx.measureText(s + '…').width > max) s = s.slice(0, -1);
  return s.trimEnd() + '…';
}

/** Перенос по словам, не больше `lines` строк. */
export function wrap(ctx: CanvasRenderingContext2D, text: string, max: number, lines: number): string[] {
  const out: string[] = [];
  let cur = '';
  for (const word of text.split(/\s+/).filter(Boolean)) {
    const next = cur ? `${cur} ${word}` : word;
    if (ctx.measureText(next).width <= max || !cur) {
      cur = next;
    } else {
      out.push(cur);
      cur = word;
    }
  }
  if (cur) out.push(cur);
  if (out.length > lines) {
    const kept = out.slice(0, lines);
    kept[lines - 1] = fit(ctx, out.slice(lines - 1).join(' '), max);
    return kept;
  }
  return out.map((l) => fit(ctx, l, max));
}

export async function loadImage(blob: Blob, urls: string[]): Promise<HTMLImageElement> {
  const url = URL.createObjectURL(blob);
  urls.push(url);
  const img = new Image();
  img.src = url;
  await img.decode();
  return img;
}

/** Обложка «как object-fit: cover» со скруглёнными углами. */
export function drawCover(ctx: CanvasRenderingContext2D, img: HTMLImageElement, x: number, y: number, w: number, h: number, r = 10) {
  const nw = img.naturalWidth || w;
  const nh = img.naturalHeight || h;
  const s = Math.max(w / nw, h / nh);
  ctx.save();
  ctx.beginPath();
  ctx.roundRect(x, y, w, h, r);
  ctx.clip();
  ctx.drawImage(img, x + (w - nw * s) / 2, y + (h - nh * s) / 2, nw * s, nh * s);
  ctx.restore();
}

/** Без обложки — плашка цвета названия, как на полке. */
export function drawPlaceholder(ctx: CanvasRenderingContext2D, t: Title, x: number, y: number, w: number, h: number, r = 10) {
  const [bg, fg] = palette(t.title);
  ctx.save();
  ctx.beginPath();
  ctx.roundRect(x, y, w, h, r);
  ctx.fillStyle = bg;
  ctx.fill();
  ctx.clip();
  const pad = Math.max(10, w * 0.09);
  ctx.fillStyle = fg;
  ctx.globalAlpha = 0.35;
  ctx.fillRect(x + pad, y + pad, w - 2 * pad, 2);
  ctx.globalAlpha = 1;
  const size = Math.max(14, Math.round(w * 0.13));
  ctx.font = `600 ${size}px ${TEXT_FONT}`;
  ctx.textBaseline = 'top';
  wrap(ctx, t.title, w - 2 * pad, 4).forEach((line, i) => ctx.fillText(line, x + pad, y + pad * 2 + i * size * 1.2));
  const by = shortBy(t);
  if (by) {
    ctx.font = `400 ${Math.max(11, Math.round(size * 0.7))}px ${TEXT_FONT}`;
    ctx.globalAlpha = 0.8;
    ctx.textBaseline = 'alphabetic';
    ctx.fillText(fit(ctx, by, w - 2 * pad), x + pad, y + h - pad);
  }
  ctx.restore();
}

/** Число, ужатое до ширины ячейки. */
function bigNumber(ctx: CanvasRenderingContext2D, text: string, x: number, y: number, max: number, size: number) {
  let s = size;
  ctx.font = `400 ${s}px ${NUM_FONT}`;
  while (s > 36 && ctx.measureText(text).width > max) {
    s -= 4;
    ctx.font = `400 ${s}px ${NUM_FONT}`;
  }
  ctx.fillText(text, x, y);
}

/** Шрифты приложения подгружаются по требованию: без этого холст нарисует запасным. */
export async function loadFonts() {
  await Promise.all(
    [`500 112px Geologica`, `400 84px Geologica`, `500 26px Geologica`, `400 28px "Golos Text"`, `600 28px "Golos Text"`].map((f) => document.fonts?.load(f).catch(() => undefined)),
  );
}

/** Холст в PNG. */
export function toPng(canvas: HTMLCanvasElement): Promise<Blob> {
  return new Promise<Blob>((resolve, reject) => canvas.toBlob((b) => (b ? resolve(b) : reject(new Error('Картинка не собралась'))), 'image/png'));
}

/** Рисует картинку года и отдаёт PNG. */
export async function drawYear({ year, stats, titles, cover }: YearInput): Promise<Blob> {
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
    const own = getComputedStyle(document.documentElement).getPropertyValue('--serOwn').trim() || '#5B9BD5';
    const work = getComputedStyle(document.documentElement).getPropertyValue('--serWork').trim() || '#E0894B';

    // Шапка.
    ctx.fillStyle = MUTED;
    ctx.textBaseline = 'alphabetic';
    ctx.font = `500 26px ${NUM_FONT}`;
    setSpacing(ctx, 6);
    ctx.fillText('ПОЛКА', PAD, PAD + 26);
    setSpacing(ctx, 0);
    ctx.fillStyle = INK;
    ctx.font = `500 112px ${NUM_FONT}`;
    ctx.fillText(`Мой ${year}`, PAD - 4, PAD + 26 + 128);

    // Цифры года: подпись над числом, как в «Итогах».
    const hours = Math.round((stats.audio_minutes + stats.screen_minutes) / 60);
    const cells: [string, string, string][] = [
      ['Книг', String(stats.books), own],
      ['Фильмов и сериалов', String(stats.films + stats.series), work],
      ['Страниц', thousands(stats.pages), INK],
      ['Часов', thousands(hours), INK],
    ];
    const top = 330;
    const cellW = (W - 2 * PAD - 3 * 24) / 4;
    ctx.fillStyle = LINE;
    ctx.fillRect(PAD, top - 34, W - 2 * PAD, 2);
    cells.forEach(([label, value, color], i) => {
      const x = PAD + i * (cellW + 24);
      // Цвет книг и кино — меткой у подписи: на тёмном фоне числа белые, так читаются лучше.
      const mark = color !== INK ? 24 : 0;
      if (mark) {
        ctx.fillStyle = color;
        ctx.beginPath();
        ctx.roundRect(x, top - 17, 14, 14, 3);
        ctx.fill();
      }
      ctx.fillStyle = MUTED;
      ctx.font = `400 24px ${TEXT_FONT}`;
      wrap(ctx, label, cellW - mark, 2).forEach((line, j) => ctx.fillText(line, x + mark, top + j * 28));
      ctx.fillStyle = INK;
      bigNumber(ctx, value, x, top + 118, cellW, 76);
    });

    // Коллаж.
    const done = yearTitles(titles, year);
    const shown = done.slice(0, MAX_TILES);
    const areaTop = 500;
    const areaH = 640;
    const areaW = W - 2 * PAD;
    if (shown.length === 0) {
      ctx.fillStyle = MUTED;
      ctx.font = `400 34px ${TEXT_FONT}`;
      ctx.fillText('Здесь будут обложки законченного за год', PAD, areaTop + 60);
    } else {
      const g = grid(shown.length, areaW, areaH);
      const usedW = g.cols * g.w + (g.cols - 1) * GAP;
      const usedH = g.rows * g.h + (g.rows - 1) * GAP;
      const x0 = PAD + (areaW - usedW) / 2;
      const y0 = areaTop + (areaH - usedH) / 2;
      const images = await Promise.all(
        shown.map(async (t) => {
          if (!t.cover) return null;
          try {
            return await loadImage(await cover(t.cover), urls);
          } catch {
            return null;
          }
        }),
      );
      shown.forEach((t, i) => {
        const x = x0 + (i % g.cols) * (g.w + GAP);
        const y = y0 + Math.floor(i / g.cols) * (g.h + GAP);
        const img = images[i];
        if (img) drawCover(ctx, img, x, y, g.w, g.h);
        else drawPlaceholder(ctx, t, x, y, g.w, g.h);
      });
      const more = done.length - shown.length;
      if (more > 0) {
        const i = shown.length - 1;
        const x = x0 + (i % g.cols) * (g.w + GAP);
        const y = y0 + Math.floor(i / g.cols) * (g.h + GAP);
        ctx.save();
        ctx.beginPath();
        ctx.roundRect(x, y, g.w, g.h, 10);
        ctx.fillStyle = 'rgba(21,35,45,0.78)';
        ctx.fill();
        ctx.fillStyle = INK;
        ctx.font = `500 ${Math.round(g.w * 0.3)}px ${NUM_FONT}`;
        ctx.textAlign = 'center';
        ctx.textBaseline = 'middle';
        ctx.fillText(`+${more + 1}`, x + g.w / 2, y + g.h / 2);
        ctx.restore();
      }
    }

    // Лучшее за год.
    const best = stats.best.map((id) => titles.find((t) => t.id === id)).filter((t): t is Title => !!t);
    const bestTop = 1196;
    ctx.fillStyle = LINE;
    ctx.fillRect(PAD, bestTop - 44, W - 2 * PAD, 2);
    if (best.length) {
      ctx.fillStyle = MUTED;
      ctx.font = `400 24px ${TEXT_FONT}`;
      ctx.fillText('Лучшее за год', PAD, bestTop);
      ctx.fillStyle = INK;
      ctx.font = `600 34px ${TEXT_FONT}`;
      const line = best.map((t) => `«${t.title}»${t.rating ? ` ${t.rating}/10` : ''}`).join('  ·  ');
      ctx.fillText(fit(ctx, line, W - 2 * PAD), PAD, bestTop + 50);
    } else {
      ctx.fillStyle = MUTED;
      ctx.font = `400 28px ${TEXT_FONT}`;
      ctx.fillText(stats.avg_rating ? `Средняя оценка ${stats.avg_rating.toFixed(1).replace('.', ',')}` : 'Оценки появятся, когда вы их поставите', PAD, bestTop + 30);
    }

    // Подпись внизу.
    ctx.fillStyle = MUTED;
    ctx.font = `400 22px ${TEXT_FONT}`;
    ctx.textAlign = 'right';
    ctx.fillText('Полка — книги, фильмы и сериалы', W - PAD, H - 40);
    ctx.textAlign = 'left';

    return await toPng(canvas);
  } finally {
    urls.forEach((u) => URL.revokeObjectURL(u));
  }
}

/** PNG в base64 — для сохранения на компьютере и «Поделиться» на телефоне. */
export async function toBase64(blob: Blob): Promise<string> {
  const buf = new Uint8Array(await blob.arrayBuffer());
  let bin = '';
  for (let i = 0; i < buf.length; i += 0x8000) bin += String.fromCharCode(...buf.subarray(i, i + 0x8000));
  return btoa(bin);
}
