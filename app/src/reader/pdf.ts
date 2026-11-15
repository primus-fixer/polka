// PDF: pdf.js грузится, только когда открывают PDF (около двух мегабайт). Страницы рисует его PDFViewer,
// текст страниц — для поиска, цитат и закладок — берётся из того же текстового слоя, что под выделением:
// места в тексте считаются в знаках UTF-16 от начала текста страницы, как у глав EPUB.
import type * as PdfLib from 'pdfjs-dist/legacy/build/pdf.mjs';
import type { PDFDocumentProxy } from 'pdfjs-dist/legacy/build/pdf.mjs';

export type Pdfjs = { lib: typeof PdfLib; viewer: typeof import('pdfjs-dist/legacy/web/pdf_viewer.mjs') };

let loading: Promise<Pdfjs> | null = null;

export function loadPdf(): Promise<Pdfjs> {
  loading ??= (async () => {
    // Сначала библиотека: просмотрщик берёт её из globalThis.pdfjsLib.
    const lib = await import('pdfjs-dist/legacy/build/pdf.mjs');
    // Фоновый разбор PDF — свой поток (Web Worker), один на всё время работы «Полки».
    const { default: PdfWorker } = await import('pdfjs-dist/legacy/build/pdf.worker.min.mjs?worker');
    lib.GlobalWorkerOptions.workerPort = new PdfWorker();
    const viewer = await import('pdfjs-dist/legacy/web/pdf_viewer.mjs');
    await import('pdfjs-dist/legacy/web/pdf_viewer.css');
    return { lib, viewer };
  })();
  loading.catch(() => {
    loading = null;
  });
  return loading;
}

/** Параметры открытия: шрифты, таблицы CMap и декодеры картинок — из папки приложения, без интернета. */
export function docOptions(data: Uint8Array) {
  const base = new URL('pdfjs/', document.baseURI).href;
  return {
    data,
    cMapUrl: `${base}cmaps/`,
    cMapPacked: true,
    standardFontDataUrl: `${base}standard_fonts/`,
    wasmUrl: `${base}wasm/`,
    iccUrl: `${base}iccs/`,
    disableWorkerRendering: true,
    enableXfa: false,
    verbosity: 0,
  };
}

/** Понятное сообщение вместо ошибки pdf.js. */
export function pdfError(e: unknown): string {
  const name = (e as { name?: string })?.name ?? '';
  if (name === 'PasswordException') return 'PDF защищён паролем на открытие — такой «Полка» не откроет';
  if (name === 'InvalidPDFException') return 'Файл повреждён или это не PDF';
  return `PDF не открылся: ${(e as Error)?.message ?? String(e)}`;
}

/** Текст страницы: `text` — ровно как в текстовом слое (по нему места цитат и найденного),
 *  `shown` — с пробелами на переносах строк (для отрывков), `norm` — для поиска: нижний регистр,
 *  «ё» → «е», пробелы схлопнуты; `back[i]` — место в `text` для знака `norm[i]` (у `shown` те же номера). */
export type PageText = { text: string; shown: string; norm: string; back: number[] };

type TextItem = { str?: string; hasEOL?: boolean };

export function buildText(items: TextItem[]): PageText {
  let text = '';
  let shown = '';
  let norm = '';
  const back: number[] = [];
  const space = (at: number) => {
    if (norm && !norm.endsWith(' ')) {
      norm += ' ';
      shown += ' ';
      back.push(at);
    }
  };
  for (const it of items) {
    if (typeof it.str !== 'string') continue;
    const s = it.str;
    for (let i = 0; i < s.length; i++) {
      const c = s[i];
      const at = text.length + i;
      if (/\s/.test(c)) {
        space(at);
        continue;
      }
      const low = c === 'Ё' || c === 'ё' ? 'е' : c.toLowerCase();
      norm += low.length === 1 ? low : c;
      shown += c;
      back.push(at);
    }
    text += s;
    if (it.hasEOL) space(text.length);
  }
  return { text, shown, norm, back };
}

/** Запрос в том же виде, что `norm`. */
export function normQuery(q: string): string {
  return q
    .replace(/\s+/g, ' ')
    .trim()
    .replace(/[Ёё]/g, 'е')
    .toLowerCase();
}

export type PdfHit = { page: number; offset: number; len: number; before: string; found: string; after: string };

/** Все места запроса на странице (не больше `limit`). */
export function findInPage(pt: PageText, page: number, q: string, limit: number): PdfHit[] {
  const out: PdfHit[] = [];
  if (!q) return out;
  let from = 0;
  while (out.length < limit) {
    const i = pt.norm.indexOf(q, from);
    if (i < 0) break;
    const offset = pt.back[i];
    const end = pt.back[i + q.length - 1] + 1;
    const head = pt.shown.slice(Math.max(0, i - 50), i);
    const sp = head.indexOf(' ');
    const before = i > 50 && sp >= 0 && sp < head.length - 1 ? head.slice(sp + 1) : head;
    const tail = pt.shown.slice(i + q.length, i + q.length + 50);
    const lp = tail.lastIndexOf(' ');
    const after = tail.length < 50 || lp <= 0 ? tail : tail.slice(0, lp);
    out.push({ page, offset, len: end - offset, before, found: pt.shown.slice(i, i + q.length), after });
    from = i + q.length;
  }
  return out;
}

/** Текст страницы из документа (номер — с единицы). */
export async function pageText(doc: PDFDocumentProxy, n: number): Promise<PageText> {
  const page = await doc.getPage(n);
  // Как у текстового слоя просмотрщика: без нормализации, иначе места разъедутся.
  const tc = await page.getTextContent({ includeMarkedContent: false, disableNormalization: true });
  return buildText(tc.items as TextItem[]);
}

/** Отрезок текста по местам внутри элемента (текстового слоя страницы). */
export function rangeIn(root: Node, offset: number, len: number): Range | null {
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  const end = offset + Math.max(1, len);
  let pos = 0;
  let start: [Node, number] | null = null;
  for (let n = walker.nextNode(); n; n = walker.nextNode()) {
    const l = n.nodeValue?.length ?? 0;
    if (!start && offset < pos + l) start = [n, offset - pos];
    if (start && end <= pos + l) {
      const r = document.createRange();
      r.setStart(start[0], start[1]);
      r.setEnd(n, end - pos);
      return r;
    }
    pos += l;
  }
  return null;
}

/** Место точки (узел и смещение) в тексте элемента. */
export function offsetIn(root: Node, node: Node, nodeOffset: number): number {
  const pre = document.createRange();
  pre.selectNodeContents(root);
  pre.setEnd(node, nodeOffset);
  return pre.toString().length;
}

/** Первая страница открытого документа — картинкой: обложка для записи без обложки. */
export async function coverFromDoc(doc: PDFDocumentProxy, width = 600): Promise<string | null> {
  const page = await doc.getPage(1);
  const base = page.getViewport({ scale: 1 });
  const viewport = page.getViewport({ scale: Math.min(4, width / base.width) });
  const canvas = document.createElement('canvas');
  canvas.width = Math.round(viewport.width);
  canvas.height = Math.round(viewport.height);
  const ctx = canvas.getContext('2d');
  if (!ctx) return null;
  ctx.fillStyle = '#fff';
  ctx.fillRect(0, 0, canvas.width, canvas.height);
  await page.render({ canvas, canvasContext: ctx, viewport }).promise;
  return canvas.toDataURL('image/jpeg', 0.86);
}
