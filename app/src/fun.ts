// Забавные сравнения для «Итогов»: часы за книгами и у экрана — в понятных мерах. Без интернета:
// мерки записаны здесь вместе с тем, на чём основана цифра.
import { NB, pluralWord } from './format';

type Stick = {
  /** Сколько часов в одной мерке. */
  hours: number;
  /** Слово к числу: одна, две, пять мерок. */
  unit: [string, string, string];
  /** Фраза с числом и словом. */
  say: (n: string) => string;
};

const STICKS: Stick[] = [
  // Большая панда тратит на бамбук около 12 часов в сутки.
  { hours: 12, unit: ['день', 'дня', 'дней'], say: (n) => `столько большая панда жуёт бамбук за ${n}` },
  // МКС облетает Землю примерно за 92 минуты.
  { hours: 92 / 60, unit: ['раз', 'раза', 'раз'], say: (n) => `МКС за это время облетела бы Землю ${n}` },
  // Прямой рейс Москва — Владивосток — около 9 часов.
  { hours: 9, unit: ['перелёт', 'перелёта', 'перелётов'], say: (n) => `это ${n} Москва — Владивосток` },
  // МКАД — 109 км, пешком по 5 км/ч — около 22 часов.
  { hours: 22, unit: ['раз', 'раза', 'раз'], say: (n) => `можно было бы обойти МКАД пешком ${n}` },
  // Коала спит до 20 часов в сутки.
  { hours: 20, unit: ['сутки', 'суток', 'суток'], say: (n) => `столько коала спит за ${n}` },
  // Трилогия «Властелин колец» в режиссёрской версии — около 11 часов.
  { hours: 11, unit: ['раз', 'раза', 'раз'], say: (n) => `хватило бы, чтобы посмотреть «Властелина колец» в режиссёрской версии ${n}` },
  // Поезд «Россия» Москва — Владивосток идёт около 166 часов — почти 7 суток.
  { hours: 166, unit: ['поездка', 'поездки', 'поездок'], say: (n) => `это ${n} на поезде «Россия» от Москвы до Владивостока` },
  // «Аполлон-11» летел до Луны около трёх суток.
  { hours: 76, unit: ['полёт', 'полёта', 'полётов'], say: (n) => `это ${n} до Луны, как у «Аполлона-11»` },
];

/** «12 перелётов», «3 раза». */
function count(n: number, unit: [string, string, string]): string {
  return `${n.toLocaleString('ru-RU')}${NB}${pluralWord(n, ...unit)}`;
}

/** Насколько хорошо число смотрится в фразе: лучше всего — от 2 до 30. */
function nice(n: number): number {
  if (n < 1) return -1;
  if (n >= 2 && n <= 30) return 3;
  if (n <= 100) return 2;
  return n < 1000 ? 1 : 0;
}

/** Лучшая мерка для часов, кроме уже взятых; при равенстве — выбор зависит от `seed` (года), чтобы итоги разных лет звучали по-разному. */
function pick(hours: number, used: Set<Stick>, seed: number): string | null {
  const fits = STICKS.filter((s) => !used.has(s))
    .map((s, i) => ({ s, n: Math.round(hours / s.hours), i }))
    .filter((x) => nice(x.n) > 0)
    .sort((a, b) => nice(b.n) - nice(a.n) || ((a.i + seed) % STICKS.length) - ((b.i + seed) % STICKS.length));
  const best = fits[0];
  if (!best) return null;
  used.add(best.s);
  return best.s.say(count(best.n, best.s.unit));
}

function hoursText(h: number): string {
  return `${Math.round(h).toLocaleString('ru-RU')}${NB}ч`;
}

/** Часы за книгами: прочитанные страницы по полторы минуты (как в читалке) и аудиокниги. */
export function bookHours(pages: number, audioMinutes: number, readerMinutes: number): number {
  return Math.max((pages * 1.5) / 60 + audioMinutes / 60, readerMinutes / 60);
}

/** До трёх строк: за книгами, у экрана и вместе. Пусто — если часов почти нет. */
export function comparisons(books: number, screen: number, year: number): string[] {
  const used = new Set<Stick>();
  const out: string[] = [];
  // Для книг первой пробуется панда (у неё — постоянное место), дальше мерки чередуются по годам.
  const line = (label: string, h: number, seed: number) => {
    if (h < 2) return;
    const p = pick(h, used, seed);
    if (p) out.push(`${label} — около ${hoursText(h)}: ${p}.`);
  };
  line('За книгами', books, 0);
  line('У экрана', screen, year);
  if (books >= 2 && screen >= 2) line('Вместе', books + screen, year + 3);
  // Только книги или только кино — второе сравнение для того же времени.
  if (out.length === 1) {
    const h = books >= 2 ? books : screen;
    const p = pick(h, used, year + 1);
    if (p) out.push(`А ещё ${p}.`);
  }
  return out;
}
