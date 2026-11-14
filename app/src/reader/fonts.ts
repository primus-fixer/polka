// Шрифт для чтения — Literata (кириллица и латиница), подключается только в читалке.
import cyr400 from '@fontsource/literata/files/literata-cyrillic-400-normal.woff2?url';
import cyr400i from '@fontsource/literata/files/literata-cyrillic-400-italic.woff2?url';
import cyr700 from '@fontsource/literata/files/literata-cyrillic-700-normal.woff2?url';
import cyr700i from '@fontsource/literata/files/literata-cyrillic-700-italic.woff2?url';
import lat400 from '@fontsource/literata/files/literata-latin-400-normal.woff2?url';
import lat400i from '@fontsource/literata/files/literata-latin-400-italic.woff2?url';
import lat700 from '@fontsource/literata/files/literata-latin-700-normal.woff2?url';
import lat700i from '@fontsource/literata/files/literata-latin-700-italic.woff2?url';

const CYRILLIC = 'U+0301,U+0400-045F,U+0490-0491,U+04B0-04B1,U+2116';
const LATIN = 'U+0000-00FF,U+0131,U+0152-0153,U+02BB-02BC,U+02C6,U+02DA,U+02DC,U+0304,U+0308,U+0329,U+2000-206F,U+20AC,U+2122,U+2191,U+2193,U+2212,U+2215,U+FEFF,U+FFFD';

let added = false;

/** Добавляет начертания Literata (один раз); файлы грузятся, когда нужны тексту. */
export function addReadingFonts() {
  if (added || typeof FontFace === 'undefined') return;
  added = true;
  const faces: [string, string, string, string][] = [
    [cyr400, '400', 'normal', CYRILLIC],
    [cyr400i, '400', 'italic', CYRILLIC],
    [cyr700, '700', 'normal', CYRILLIC],
    [cyr700i, '700', 'italic', CYRILLIC],
    [lat400, '400', 'normal', LATIN],
    [lat400i, '400', 'italic', LATIN],
    [lat700, '700', 'normal', LATIN],
    [lat700i, '700', 'italic', LATIN],
  ];
  for (const [url, weight, style, unicodeRange] of faces) {
    try {
      document.fonts.add(new FontFace('Literata', `url(${url}) format('woff2')`, { weight, style, unicodeRange, display: 'swap' }));
    } catch {
      /* без шрифта — системный с засечками */
    }
  }
}

/** Ждёт основные начертания перед раскладкой страниц, но не дольше секунды. */
export async function readingFontsReady(family: string, size: number): Promise<void> {
  if (typeof document === 'undefined' || !document.fonts) return;
  const loads = Promise.all([document.fonts.load(`400 ${size}px ${family}`, 'Ая'), document.fonts.load(`italic 400 ${size}px ${family}`, 'Ая')]).then(() => undefined);
  await Promise.race([loads.catch(() => undefined), new Promise<void>((r) => setTimeout(r, 1000))]);
}
