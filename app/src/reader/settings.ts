// Вид читалки на этом устройстве: размер и гарнитура, интервал, поля, тема, выравнивание.
import { store, stored } from '../storage';

export type ReaderTheme = 'auto' | 'light' | 'sepia' | 'dark';
export type ReaderFont = 'serif' | 'sans';
export type ReaderMargins = 'narrow' | 'normal' | 'wide';

export type ReaderSettings = {
  size: number;
  font: ReaderFont;
  leading: number;
  margins: ReaderMargins;
  theme: ReaderTheme;
  justify: boolean;
  /** Две страницы рядом на широком экране. */
  spread: boolean;
};

export const DEFAULTS: ReaderSettings = { size: 19, font: 'serif', leading: 1.5, margins: 'normal', theme: 'auto', justify: true, spread: true };
export const SIZE_MIN = 13;
export const SIZE_MAX = 34;
export const LEADINGS: [number, string][] = [
  [1.3, 'Плотно'],
  [1.5, 'Обычно'],
  [1.75, 'Свободно'],
];

export const FONTS: Record<ReaderFont, { label: string; stack: string; family: string }> = {
  serif: { label: 'С засечками', stack: "Literata, 'PT Serif', Georgia, 'Noto Serif', 'Times New Roman', serif", family: 'Literata' },
  sans: { label: 'Без засечек', stack: "'Golos Text', 'Segoe UI', Roboto, Arial, sans-serif", family: "'Golos Text'" },
};

/** Поля страницы по бокам: доля ширины и наименьшее значение в точках. */
export const MARGINS: Record<ReaderMargins, { label: string; share: number; min: number }> = {
  narrow: { label: 'Узкие', share: 0.03, min: 12 },
  normal: { label: 'Обычные', share: 0.06, min: 20 },
  wide: { label: 'Широкие', share: 0.11, min: 32 },
};

export const THEMES: Record<Exclude<ReaderTheme, 'auto'>, { label: string; bg: string; ink: string; muted: string; link: string; line: string; hit: string; bar: string }> = {
  light: { label: 'Светлая', bg: '#FBFAF7', ink: '#1E1E1C', muted: '#77756F', link: '#0E5A8A', line: '#DDD9CF', hit: '#FBE38E', bar: '#F2F0EA' },
  sepia: { label: 'Сепия', bg: '#F4ECD8', ink: '#3B2F22', muted: '#85745E', link: '#8A4B12', line: '#DCCDAF', hit: '#EBC86E', bar: '#EADFC6' },
  dark: { label: 'Тёмная', bg: '#141414', ink: '#D9D4CB', muted: '#8E8A82', link: '#8CB8DE', line: '#34322E', hit: '#6B5A1E', bar: '#1E1E1D' },
};

const KEY = 'polka.reader';

export function readSettings(): ReaderSettings {
  try {
    const raw = JSON.parse(stored(KEY, '{}')) as Partial<ReaderSettings>;
    const s = { ...DEFAULTS, ...raw };
    return {
      size: Number.isFinite(s.size) ? Math.min(SIZE_MAX, Math.max(SIZE_MIN, Math.round(s.size))) : DEFAULTS.size,
      font: s.font === 'sans' ? 'sans' : 'serif',
      leading: LEADINGS.some(([v]) => v === s.leading) ? s.leading : DEFAULTS.leading,
      margins: s.margins in MARGINS ? s.margins : DEFAULTS.margins,
      theme: s.theme === 'auto' || s.theme in THEMES ? s.theme : DEFAULTS.theme,
      justify: s.justify !== false,
      spread: s.spread !== false,
    };
  } catch {
    return DEFAULTS;
  }
}

export function saveSettings(s: ReaderSettings) {
  store(KEY, JSON.stringify(s));
}

/** Тема «как в приложении»: светлая или тёмная — смотря что выбрано в «Полке». */
export function resolveTheme(t: ReaderTheme): Exclude<ReaderTheme, 'auto'> {
  if (t !== 'auto') return t;
  return document.documentElement.style.colorScheme === 'dark' ? 'dark' : 'light';
}

/** Скорость чтения на этом устройстве, знаков в минуту (для «осталось около…»). */
export function readSpeed(): number {
  const v = Number(stored('polka.speed', '1200'));
  return Number.isFinite(v) && v >= 300 && v <= 6000 ? v : 1200;
}

export function saveSpeed(v: number) {
  if (Number.isFinite(v)) store('polka.speed', String(Math.round(Math.min(6000, Math.max(300, v)))));
}
