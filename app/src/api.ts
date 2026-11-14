import { convertFileSrc, invoke } from '@tauri-apps/api/core';
import { demo } from './demo';
import { todayIso } from './format';

// Типы повторяют ответы ядра (polka-core, serde): пустые поля в ответе не приходят.

export type Kind = 'book' | 'film' | 'series';
export type Status = 'want' | 'doing' | 'done' | 'dropped';
export type BookFormat = 'paper' | 'ebook' | 'audio';

export type Seen = { ep: number; on: string };
export type Season = { no: number; episodes: number; seen: Seen[] };
export type SourceRef = { provider: string; id: string };

export type Title = {
  id: string;
  kind: Kind;
  title: string;
  original?: string;
  creators: string[];
  year?: string;
  cover?: string;
  cover_source?: string;
  status: Status;
  added: string;
  started?: string;
  finished?: string;
  rating?: number;
  note?: string;
  tags: string[];
  genres: string[];
  format?: BookFormat;
  isbn?: string;
  publisher?: string;
  pages?: number;
  minutes?: number;
  kp?: number;
  imdb?: number;
  ratings_on?: string;
  about?: string;
  seasons: Season[];
  sources?: SourceRef[];
};

export type CoverOption = { url: string; source: string; edition: string };

export type Candidate = {
  kind: Kind;
  title: string;
  original?: string | null;
  creators: string[];
  year?: string | null;
  publisher?: string | null;
  isbn?: string | null;
  pages?: number | null;
  genres: string[];
  covers: CoverOption[];
  sources: string[];
  minutes?: number | null;
  kp?: number | null;
  imdb?: number | null;
  about?: string | null;
  seasons: [number, number][];
  refs: SourceRef[];
};

export type Found = { results: Candidate[]; failed: { source: string; message: string }[] };

/** Что выбрано при добавлении. */
export type AddForm = {
  status: Status;
  date: string;
  rating: number | null;
  format: BookFormat | null;
  tags: string[];
};

export type Goal = { books?: number | null; screen?: number | null };

export type YearStats = {
  year: number;
  goal: Goal;
  books: number;
  films: number;
  series: number;
  episodes: number;
  pages: number;
  audio_minutes: number;
  screen_minutes: number;
  dropped: number;
  avg_rating: number | null;
  months: { books: number; screen: number }[];
  genres: { name: string; n: number }[];
  authors: { name: string; n: number }[];
  best: string[];
  books_left: number | null;
};

export type Settings = { goal: Goal; has_poiskkino: boolean; has_google: boolean; key_method: string };

/** Запущено внутри приложения (Tauri), а не в браузере. */
export const inApp = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

const real = {
  titles: () => invoke<Title[]>('titles'),
  save: (title: Title) => invoke<Title>('save_title', { title, today: todayIso() }),
  remove: (id: string) => invoke<void>('delete_title', { id }),
  setStatus: (id: string, status: Status) => invoke<Title>('set_status', { id, status, today: todayIso() }),
  markEpisode: (id: string, season: number, ep: number, seen: boolean) => invoke<Title>('mark_episode', { id, season, ep, seen, today: todayIso() }),
  markSeason: (id: string, season: number, seen: boolean) => invoke<Title>('mark_season', { id, season, seen, today: todayIso() }),
  search: (kind: 'book' | 'film', query: string) => invoke<Found>('search', { kind, query }),
  fromLink: (url: string) => invoke<Candidate>('from_link', { url }),
  details: (candidate: Candidate) => invoke<Candidate>('details', { candidate }),
  add: (candidate: Candidate, coverUrl: string | null, coverLabel: string | null, form: AddForm) =>
    invoke<Title>('add_title', { candidate, coverUrl, coverLabel, form, today: todayIso() }),
  setCoverUrl: (id: string, url: string, label: string) => invoke<Title>('set_cover_url', { id, url, label }),
  setCoverPhoto: (id: string, dataUrl: string) => invoke<Title>('set_cover_photo', { id, dataUrl }),
  stats: (year: number) => invoke<YearStats>('stats', { year }),
  settings: (year: number) => invoke<Settings>('settings', { year }),
  setGoal: (year: number, goal: Goal) => invoke<void>('set_goal', { year, goal }),
  setKey: (name: 'poiskkino' | 'google', value: string) => invoke<void>('set_key', { name, value }),
};

export type Api = typeof real;

/** В браузере (`npm run dev`, превью) — демо-данные в памяти. */
export const api: Api = inApp ? real : demo;

/** Адрес снимка обложки из базы: его отдаёт протокол `cover` приложения. */
export function coverSrc(id: string): string | null {
  if (inApp) return convertFileSrc(id, 'cover');
  // Демо в браузере: обложка — сама картинка (data:).
  return id.startsWith('data:') ? id : null;
}

/** Сканирование штрихкода книги (EAN-13) камерой телефона. */
export async function scanIsbn(): Promise<string> {
  const scanner = await import('@tauri-apps/plugin-barcode-scanner');
  let perm = await scanner.checkPermissions();
  if (perm !== 'granted') perm = await scanner.requestPermissions();
  if (perm !== 'granted') throw new Error('Нет доступа к камере. Разрешите его в настройках телефона');
  const res = await scanner.scan({ windowed: false, formats: [scanner.Format.EAN13] });
  return res.content;
}

/** Телефон: в приложении на Android или узкое окно. */
export const isMobileApp = inApp && /android|iphone|ipad/i.test(navigator.userAgent);

export function message(e: unknown): string {
  if (typeof e === 'string') return e;
  if (e instanceof Error) return e.message;
  return String(e);
}

/** Снимок обложки с камеры или из файла: уменьшаем до 1200 точек по длинной стороне, JPEG. */
export async function photoToDataUrl(file: File): Promise<string> {
  const url = URL.createObjectURL(file);
  try {
    const img = await new Promise<HTMLImageElement>((resolve, reject) => {
      const i = new Image();
      i.onload = () => resolve(i);
      i.onerror = () => reject(new Error('Снимок не открылся'));
      i.src = url;
    });
    const scale = Math.min(1, 1200 / Math.max(img.naturalWidth, img.naturalHeight));
    const canvas = document.createElement('canvas');
    canvas.width = Math.round(img.naturalWidth * scale);
    canvas.height = Math.round(img.naturalHeight * scale);
    const ctx = canvas.getContext('2d');
    if (!ctx) throw new Error('Снимок не обработать');
    ctx.drawImage(img, 0, 0, canvas.width, canvas.height);
    return canvas.toDataURL('image/jpeg', 0.86);
  } finally {
    URL.revokeObjectURL(url);
  }
}
