// Даты, числа и склонения по-русски.

const GEN = ['января', 'февраля', 'марта', 'апреля', 'мая', 'июня', 'июля', 'августа', 'сентября', 'октября', 'ноября', 'декабря'];
const SHORT = ['янв.', 'февр.', 'мар.', 'апр.', 'мая', 'июн.', 'июл.', 'авг.', 'сент.', 'окт.', 'нояб.', 'дек.'];
const MONTHS = ['Январь', 'Февраль', 'Март', 'Апрель', 'Май', 'Июнь', 'Июль', 'Август', 'Сентябрь', 'Октябрь', 'Ноябрь', 'Декабрь'];
export const MONTH_LETTERS = ['Я', 'Ф', 'М', 'А', 'М', 'И', 'И', 'А', 'С', 'О', 'Н', 'Д'];
export const NB = ' ';

/** Сегодня по местному времени: «2026-11-13». */
export function todayIso(): string {
  const d = new Date();
  const p = (n: number) => String(n).padStart(2, '0');
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}`;
}

function parts(iso: string): [number, number, number] {
  const [y, m, d] = iso.split('-').map(Number);
  return [y, m, d];
}

export function yearOf(iso: string): number {
  return parts(iso)[0];
}

export function monthOf(iso: string): number {
  return parts(iso)[1];
}

/** «12 ноября» */
export function dayMonth(iso: string): string {
  const [, m, d] = parts(iso);
  return `${d}${NB}${GEN[m - 1]}`;
}

/** «12 нояб.» */
export function dayShort(iso: string): string {
  const [, m, d] = parts(iso);
  return `${d}${NB}${SHORT[m - 1]}`;
}

/** «Ноябрь» или «Ноябрь 2025», если год не текущий. */
export function monthTitle(iso: string, thisYear: number): string {
  const [y, m] = parts(iso);
  return y === thisYear ? MONTHS[m - 1] : `${MONTHS[m - 1]} ${y}`;
}

/** «2–12 ноября», «22 октября – 6 ноября», «12 ноября». */
export function span(start?: string, end?: string): string {
  if (start && end && start !== end) {
    const [y1, m1, d1] = parts(start);
    const [y2, m2, d2] = parts(end);
    if (y1 === y2 && m1 === m2) return `${d1}–${d2}${NB}${GEN[m2 - 1]}`;
    return `${dayMonth(start)} – ${dayMonth(end)}`;
  }
  const one = end || start;
  return one ? dayMonth(one) : '';
}

/** Слово к числу: «книга», «книги», «книг». */
export function pluralWord(n: number, one: string, few: string, many: string): string {
  const m10 = n % 10;
  const m100 = n % 100;
  return m10 === 1 && m100 !== 11 ? one : m10 >= 2 && m10 <= 4 && (m100 < 12 || m100 > 14) ? few : many;
}

/** «1 книга», «2 книги», «5 книг». */
export function plural(n: number, one: string, few: string, many: string): string {
  return `${n}${NB}${pluralWord(n, one, few, many)}`;
}

/** «1 234 записи» — с разделителем тысяч. */
export function count(n: number, one: string, few: string, many: string): string {
  return `${n.toLocaleString('ru-RU')}${NB}${pluralWord(n, one, few, many)}`;
}

/** «6 240» */
export function thousands(n: number): string {
  return String(Math.round(n)).replace(/\B(?=(\d{3})+(?!\d))/g, NB);
}

/** «8,9» */
export function decimal(n: number): string {
  return n.toFixed(1).replace('.', ',');
}

/** Дней до конца года. */
export function daysLeftInYear(today: string): number {
  const [y, m, d] = parts(today);
  const now = Date.UTC(y, m - 1, d);
  return Math.round((Date.UTC(y, 11, 31) - now) / 86400000) + 1;
}
