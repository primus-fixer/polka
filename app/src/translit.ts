/**
 * Поиск с транслитом: «Стругацкие» находит «Strugatsky», «Strugatsky» — «Стругацкие».
 * Всё приводится к латинице; для слов подлиннее сравнивается ещё и «скелет» — согласные,
 * чтобы сошлись разные написания: Strugatsky, Strugatskiy, Стругацкие.
 */

const CYR: Record<string, string> = {
  а: 'a', б: 'b', в: 'v', г: 'g', д: 'd', е: 'e', ё: 'e', ж: 'zh', з: 'z', и: 'i', й: 'y', к: 'k', л: 'l', м: 'm',
  н: 'n', о: 'o', п: 'p', р: 'r', с: 's', т: 't', у: 'u', ф: 'f', х: 'kh', ц: 'ts', ч: 'ch', ш: 'sh', щ: 'shch',
  ъ: '', ы: 'y', ь: '', э: 'e', ю: 'yu', я: 'ya', і: 'i', ї: 'yi', є: 'ye', ґ: 'g', ў: 'u',
};

/** Нижний регистр, без «ё», латиницей: «Стругацкие» → «strugatskie». */
export function latin(s: string): string {
  let out = '';
  for (const ch of s.toLocaleLowerCase('ru')) out += CYR[ch] ?? ch;
  return out
    .normalize('NFD')
    .replace(/[̀-ͯ]/g, '')
    .replace(/shch|sch/g, 'sh')
    .replace(/kh/g, 'h')
    .replace(/tz/g, 'ts')
    .replace(/ph/g, 'f')
    .replace(/ck|q/g, 'k')
    .replace(/x/g, 'ks')
    .replace(/w/g, 'v')
    .replace(/c(?!h)/g, 'k');
}

/** Согласные слова: первая буква и дальше без гласных, «h» и повторов — «strugatskie» → «strgtsk». */
export function skeleton(word: string): string {
  const w = latin(word).replace(/[^a-z]/g, '');
  if (!w) return '';
  let out = w[0];
  for (const ch of w.slice(1)) {
    if ('aeiouyh'.includes(ch) || out.endsWith(ch)) continue;
    out += ch;
  }
  return out;
}

const words = (s: string) => s.split(/[^\p{L}\p{N}]+/u).filter(Boolean);

/** Текст, заранее подготовленный для поиска. */
export type Haystack = { plain: string; lat: string; skeletons: string[] };

export function haystack(text: string): Haystack {
  const plain = text.toLocaleLowerCase('ru').replace(/ё/g, 'е');
  return { plain, lat: latin(plain), skeletons: words(plain).map(skeleton).filter((s) => s.length >= 3) };
}

/** Каждое слово запроса нашлось — как есть, латиницей или по согласным (для слов от четырёх букв). */
export function textMatches(hay: Haystack, query: string): boolean {
  const q = query.toLocaleLowerCase('ru').replace(/ё/g, 'е');
  return words(q).every((w) => {
    if (hay.plain.includes(w) || hay.lat.includes(latin(w))) return true;
    const sk = w.length >= 4 ? skeleton(w) : '';
    return sk.length >= 3 && hay.skeletons.some((s) => s.startsWith(sk));
  });
}

/** Одно и то же название, как бы ни было набрано: «Мастер и Маргарита» = «Master i Margarita». */
export function sameTitle(a: string, b: string): boolean {
  const key = (s: string) => latin(s.replace(/[^\p{L}\p{N}]+/gu, ' ').trim());
  return key(a) === key(b);
}
