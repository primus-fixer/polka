/** «Поделиться» → «Полка» на Android: текст из другого приложения — ссылка и, если есть, название. */

/** Сайты кино: ссылка с них — фильм или сериал, остальные — книга. */
const FILM_SITES = /(^|\.)(kinopoisk\.ru|ivi\.ru|okko\.tv|kion\.ru|start\.ru|premier\.one|wink\.ru|more\.tv|kino\.mail\.ru|imdb\.com|letterboxd\.com|netflix\.com)$/i;

/** Обороты, которыми приложения обрамляют ссылку: «Посмотрите … на Кинопоиске». */
const NOISE =
  /(?<!\p{L})(посмотрите|смотрите|смотри|посмотри|читайте|читай|почитай|почитайте|слушайте|послушайте|рекомендую|советую|нашёл|нашел|нашла|на кинопоиске|на литрес[еа]?|в литрес[еа]?|на livelib|на лабиринте|в читай-городе|в приложении)(?!\p{L})/giu;
/** «Слушайте аудиокнигу Щегол» → «Щегол»: слово-вид в начале — не часть названия. */
const LEAD = /^(аудиокнигу|аудиокнига|электронную книгу|книгу|книга|фильм|сериал)\s+/iu;

export type Shared = { url: string | null; title: string; kind: 'book' | 'film' | null };

export function hostOf(url: string): string {
  const rest = url.split('://')[1] ?? '';
  return (rest.split(/[/?#]/)[0] ?? '').split('@').pop()?.split(':')[0]?.toLowerCase() ?? '';
}

export function parseShared(text: string): Shared {
  const m = /https?:\/\/[^\s<>"«»]+/i.exec(text);
  const url = m ? m[0].replace(/[.,;:!?)'"]+$/, '') : null;
  const rest = (m ? text.replace(m[0], ' ') : text).replace(/\s+/g, ' ').trim();
  const quoted = /«([^»]{2,120})»/.exec(rest)?.[1];
  const title = (quoted ?? rest.replace(NOISE, ' '))
    .replace(/[|•]+.*$/, '')
    .replace(/^[\s\-—–:,.()]+|[\s\-—–:,.()]+$/g, '')
    .replace(/\s+/g, ' ')
    .replace(LEAD, '')
    .slice(0, 120)
    .trim();
  const kind = url ? (FILM_SITES.test(hostOf(url)) ? 'film' : 'book') : null;
  return { url: url && url.length > 'https://'.length + 3 ? url : null, title: title.length >= 2 ? title : '', kind };
}

type Bridge = {
  take(): string | null;
  takeOpen?(): string | null;
  setWidget?(json: string): void;
  sendImage?(base64: string, name: string): boolean;
  canSaveImage?(): boolean;
  saveImage?(base64: string, name: string): string | null;
};

function bridge(): Bridge | undefined {
  return (window as unknown as { PolkaShare?: Bridge }).PolkaShare;
}

/** На телефоне картинку можно отправить через «Поделиться». */
export function canShareImage(): boolean {
  return typeof bridge()?.sendImage === 'function';
}

/** Android 10 и новее: картинку можно положить прямо в «Галерею» (Изображения/Полка). */
export function canSaveToGallery(): boolean {
  try {
    return bridge()?.canSaveImage?.() === true;
  } catch {
    return false;
  }
}

/** Кладёт картинку в «Изображения/Полка»; возвращает, куда легла, или null. */
export function saveToGallery(base64: string, name: string): string | null {
  try {
    return bridge()?.saveImage?.(base64, name) ?? null;
  } catch {
    return null;
  }
}

/** Открывает «Поделиться» с картинкой (PNG в base64). */
export function shareImage(base64: string, name: string): boolean {
  try {
    return bridge()?.sendImage?.(base64, name) ?? false;
  } catch {
    return false;
  }
}

/** Запись, которую открыли из виджета: номер; «» — просто «Полка»; null — ничего. */
export function takeOpened(): string | null {
  try {
    return bridge()?.takeOpen?.() ?? null;
  } catch {
    return null;
  }
}

/** Виджет «Сейчас» на главном экране Android: есть ли он в этой сборке. */
export function canWidget(): boolean {
  return typeof bridge()?.setWidget === 'function';
}

export function setWidget(json: string) {
  try {
    bridge()?.setWidget?.(json);
  } catch {
    /* виджет обновится при следующем изменении */
  }
}

/** Текст, который ждёт от «Поделиться» (Android); его можно забрать один раз. */
export function takeShared(): string | null {
  try {
    return bridge()?.take() || null;
  } catch {
    return null;
  }
}
