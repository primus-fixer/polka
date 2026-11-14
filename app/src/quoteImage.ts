/**
 * Цитата картинкой — для «Поделиться»: текст цитаты шрифтом читалки, ниже обложка, название и автор.
 * 1080 × 1350, как карточка и картинка года. Длинная цитата мельчает, очень длинная — обрезается.
 */
import type { Mark, Title } from './api';
import { addReadingFonts } from './reader/fonts';
import { BG, INK, LINE, MUTED, NUM_FONT, TEXT_FONT, drawCover, drawPlaceholder, fit, H, loadFonts, loadImage, setSpacing, toPng, W, wrap } from './yearImage';

const PAD = 96;
const SERIF = "Literata, 'PT Serif', Georgia, 'Times New Roman', serif";
const ACCENT = '#8CC2E8';

/** Перенос по словам без обрезки; пустая строка — граница абзацев. */
function lines(ctx: CanvasRenderingContext2D, text: string, max: number): string[] {
  const out: string[] = [];
  text
    .split(/\n+/)
    .map((p) => p.trim())
    .filter(Boolean)
    .forEach((para, i) => {
      if (i > 0) out.push('');
      let cur = '';
      for (const word of para.split(/\s+/)) {
        const next = cur ? `${cur} ${word}` : word;
        if (ctx.measureText(next).width <= max || !cur) cur = next;
        else {
          out.push(cur);
          cur = word;
        }
      }
      if (cur) out.push(cur);
    });
  return out;
}

const height = (ls: string[], lh: number) => ls.reduce((h, l) => h + (l === '' ? lh * 0.55 : lh), 0);

export async function drawQuote(t: Title, m: Mark, cover: ((id: string) => Promise<Blob>) | null): Promise<Blob> {
  addReadingFonts();
  await loadFonts();
  await Promise.all(['400 56px Literata', 'italic 400 56px Literata', '400 200px Literata'].map((f) => document.fonts?.load(f, '«Ая»').catch(() => undefined)));
  const canvas = document.createElement('canvas');
  canvas.width = W;
  canvas.height = H;
  const ctx = canvas.getContext('2d');
  if (!ctx) throw new Error('Холст недоступен — картинку не нарисовать');
  const urls: string[] = [];
  try {
    ctx.fillStyle = BG;
    ctx.fillRect(0, 0, W, H);
    ctx.textBaseline = 'alphabetic';

    ctx.fillStyle = MUTED;
    ctx.font = `500 24px ${NUM_FONT}`;
    setSpacing(ctx, 6);
    ctx.textAlign = 'center';
    ctx.fillText('ПОЛКА', W / 2, 92);
    ctx.textAlign = 'left';
    setSpacing(ctx, 0);

    // Текст: крупно, пока помещается; иначе мельче, в самом мелком — обрезаем.
    const top = 350;
    const bottom = H - 380;
    const width = W - 2 * PAD;
    const text = (m.text ?? '').trim() || '…';
    let size = 60;
    let ls: string[] = [];
    for (; size >= 30; size -= 2) {
      ctx.font = `400 ${size}px ${SERIF}`;
      ls = lines(ctx, text, width);
      if (height(ls, size * 1.4) <= bottom - top) break;
    }
    size = Math.max(30, size);
    const lh = size * 1.4;
    ctx.font = `400 ${size}px ${SERIF}`;
    const all = lines(ctx, text, width);
    ls = all.slice();
    while (ls.length > 1 && height(ls, lh) > bottom - top) ls.pop();
    if (ls[ls.length - 1] === '') ls.pop();
    // Закрывающая кавычка — в конце; не поместилось — многоточие перед ней.
    const tail = ls.length < all.length ? '…»' : '»';
    const last = ls.length - 1;
    let end = ls[last];
    while (end.length > 1 && ctx.measureText(end + tail).width > width) end = end.slice(0, -1).trimEnd();
    ls[last] = end + tail;
    // Короткая цитата — посередине свободного места, кавычка — над ней.
    const block = height(ls, lh);
    const y0 = top + Math.max(0, (bottom - top - block) / 2);
    ctx.fillStyle = ACCENT;
    ctx.font = `400 210px ${SERIF}`;
    ctx.fillText('«', PAD - 14, y0 - 38);
    ctx.font = `400 ${size}px ${SERIF}`;
    ctx.fillStyle = INK;
    let y = y0 + size;
    for (const l of ls) {
      if (l === '') {
        y += lh * 0.55;
        continue;
      }
      ctx.fillText(l, PAD, y);
      y += lh;
    }

    // Откуда: обложка, название, автор.
    const by = H - 322;
    ctx.fillStyle = LINE;
    ctx.fillRect(PAD, by, 120, 2);
    const cw = 120;
    const ch = 180;
    const cy = by + 40;
    let img: HTMLImageElement | null = null;
    if (t.cover && cover) {
      try {
        img = await loadImage(await cover(t.cover), urls);
      } catch {
        img = null;
      }
    }
    if (img) drawCover(ctx, img, PAD, cy, cw, ch, 10);
    else drawPlaceholder(ctx, t, PAD, cy, cw, ch, 10);
    const tx = PAD + cw + 36;
    const tw = W - PAD - tx;
    ctx.textBaseline = 'alphabetic';
    ctx.fillStyle = INK;
    ctx.font = `600 38px ${TEXT_FONT}`;
    let ty = cy + 48;
    for (const line of wrap(ctx, t.title, tw, 2)) {
      ctx.fillText(line, tx, ty);
      ty += 48;
    }
    const who = t.creators.slice(0, 2).join(', ');
    if (who) {
      ctx.fillStyle = MUTED;
      ctx.font = `400 30px ${TEXT_FONT}`;
      ctx.fillText(fit(ctx, who, tw), tx, ty + 4);
    }

    ctx.fillStyle = MUTED;
    ctx.font = `400 22px ${TEXT_FONT}`;
    ctx.textAlign = 'center';
    ctx.fillText('Полка — книги, фильмы и сериалы', W / 2, H - 44);
    ctx.textAlign = 'left';
    return await toPng(canvas);
  } finally {
    urls.forEach((u) => URL.revokeObjectURL(u));
  }
}
