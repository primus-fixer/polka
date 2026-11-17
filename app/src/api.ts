import { convertFileSrc, invoke } from '@tauri-apps/api/core';
import { lockCommands } from '@primus-fixer/kit/ui';
import { demo, demoBookImage } from './demo';
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

export type BackupEvery = 'day' | 'week' | 'off';
export type BackupLast = { at: number; path: string; auto: boolean; bytes: number; events: number; blobs: number; books?: number };
export type BackupStatus = {
  /** Копии делает это устройство (компьютер). */
  desktop: boolean;
  password: boolean;
  dir: string;
  /** Папку выбрали, а не папка по умолчанию. */
  chosen: boolean;
  every: BackupEvery;
  keep: number;
  last: BackupLast | null;
  /** Последняя копия с любого устройства: «секунды|устройство». */
  shared: string | null;
  error: string | null;
  running: boolean;
  /** Подключено облако: оно зашифровано тем же паролем. */
  cloud: boolean;
};
export type BackupInfo = { created: number; device: string; app: string; titles: number; events: number; blobs: number; books: number; bytes: number };
export type RestoreResult = { events: number; blobs: number; books: number; titles: number; list: Title[] };

// Связь по Wi-Fi и облако — общие экраны и команды из kit (`@primus-fixer/kit/ui`).
export type { CloudEvent, CloudLast, CloudStatus, CloudStep, LanCode, LanEvent, LanNearby, LanPeer, LanView, PairResult } from '@primus-fixer/kit/ui';
import type { CloudEvent, CloudStatus, LanEvent, LanView, PairResult } from '@primus-fixer/kit/ui';
/** Облако по паролю: Яндекс Диск (пароль приложения) или свой сервер WebDAV. */
export type CloudProvider = 'yadisk' | 'webdav';

// ——— Читалка ———

export type FileFormat = 'epub' | 'fb2' | 'pdf';
export type BookMeta = { title: string; authors: string[]; lang?: string; year?: string; publisher?: string; isbn?: string; about?: string; genres: string[] };
/** Глава: название и объём в знаках (по нему считается процент). */
export type BookChapter = { title: string; size: number };
export type TocItem = { title: string; chapter: number; anchor?: string; depth: number };
export type BookView = { hash: string; format: FileFormat; meta: BookMeta; chapters: BookChapter[]; toc: TocItem[]; total: number; title: Title | null };
/** Найденное в книге: место — в единицах UTF-16 от начала главы. */
export type BookHit = { chapter: number; offset: number; len: number; before: string; found: string; after: string };
/** Файл книги на этом устройстве и запись полки, к которой он привязан. */
export type LocalBook = { hash: string; title: string; name: string; format: FileFormat; size: number; added: number };
/** Где остановились (запись журнала — уходит на другие устройства); `years` — минуты чтения по годам. */
export type Reading = { title: string; book: string; chapter: number; offset: number; percent: number; at: number; minutes: number; years?: Record<string, number> };
export type MarkKind = 'bookmark' | 'quote';
/** Закладка или цитата (запись журнала): место в книге, текст, своя заметка. */
export type Mark = { id: string; title: string; book: string; kind: MarkKind; chapter: number; offset: number; end: number; text?: string; note?: string; percent: number; created: number };
export type Imported = { hash: string; title: Title; created: boolean; format: FileFormat };
/** Файл книги на другом устройстве (запись пришла с журналом); `skipped` — убран отсюда, сам не придёт. */
export type RemoteBook = { hash: string; title: string; name: string; format: FileFormat; size: number; skipped: boolean };
/** Файлы книг на этом устройстве: обмен ими, в резервной копии ли они, сколько их здесь. */
export type FilesSettings = { sync: boolean; backup: boolean; count: number; bytes: number };
/** Узел главы: текст или [тег, атрибуты, …дети] — страница строит его сама, без innerHTML. */
export type BookNode = string | [string, Record<string, string>, ...BookNode[]];
export type BookDrop = { ok?: Imported; error?: string };

// ——— Вход ———

/** Ответ команд с данными, пока «Полка» заблокирована, и состояние входа — из kit. */
export { LOCKED, type LockStatus } from '@primus-fixer/kit/ui';

// ——— Коннектор для приложения Claude (компьютер) ———

/** Действие Claude, которое можно отменить. */
export type ClaudeAction = { id: number; at: number; tool: string; text: string; undone: boolean };
export type ClaudeStatus = {
  /** Коннектор есть только на компьютере. */
  desktop: boolean;
  on: boolean;
  write: boolean;
  /** Путь к polka.exe и папка данных — для ручной настройки. */
  command: string;
  dir: string;
  /** Готовый текст для claude_desktop_config.json. */
  config: string;
  /** Новые сверху. */
  actions: ClaudeAction[];
};
/** Claude что-то записал: `text` — «Claude: Добавлено: …». */
export type ClaudeEvent = { id: number; text: string };

/** Файл копии уходит в ядро кусками по 4 МБ: на телефоне его иначе не прочитать. */
const UPLOAD_CHUNK = 4 << 20;

/** Запущено внутри приложения (Tauri), а не в браузере. */
export const inApp = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

const real = {
  // Вход: PIN-код, отпечаток или Windows Hello — команды kit-tauri.
  ...lockCommands(),
  claudeStatus: () => invoke<ClaudeStatus>('claude_status'),
  claudeSet: (on: boolean, write: boolean) => invoke<ClaudeStatus>('claude_set', { on, write }),
  /** Отменить действие Claude («Вернуть»). */
  claudeUndo: (id: number) => invoke<ClaudeStatus>('claude_undo', { id }),
  /** Расширение «Полка для Claude.mcpb» — в «Загрузки»; `open` — сразу открыть: Claude предложит установить. */
  claudeBundle: (open: boolean) => invoke<string>('claude_bundle', { open }),
  onClaude: async (listener: (e: ClaudeEvent) => void): Promise<() => void> => {
    const { listen } = await import('@tauri-apps/api/event');
    return listen<ClaudeEvent>('claude', (ev) => listener(ev.payload));
  },
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
  /** Смещение местного времени — для имён резервных копий. */
  setTz: (minutes: number) => invoke<void>('set_tz', { minutes }),
  backupStatus: () => invoke<BackupStatus>('backup_status'),
  backupPassword: (password: string) => invoke<BackupStatus>('backup_password', { password }),
  backupEvery: (every: BackupEvery) => invoke<BackupStatus>('backup_every', { every }),
  /** Папка копий; `null` — папка по умолчанию. */
  backupDir: (path: string | null) => invoke<BackupStatus>('backup_dir', { path }),
  backupPickDir: () => invoke<BackupStatus>('backup_pick_dir'),
  backupNow: () => invoke<BackupLast>('backup_now'),
  /** Загружает файл копии в ядро и возвращает номер загрузки. */
  backupUpload: async (file: File, progress?: (done: number, total: number) => void): Promise<string> => {
    let id = 'new';
    for (let at = 0; at < file.size || at === 0; at += UPLOAD_CHUNK) {
      const part = new Uint8Array(await file.slice(at, at + UPLOAD_CHUNK).arrayBuffer());
      const r = await invoke<{ id: string; size: number }>('backup_upload', part, { headers: { 'x-upload': id, 'x-offset': String(at) } });
      id = r.id;
      progress?.(r.size, file.size);
      if (file.size === 0) break;
    }
    return id;
  },
  /** Файл книги: загрузить кусками, сохранить у себя, найти или завести запись на полке
   *  (`title` — к какой записи привязать, если файл открыли из её карточки). */
  bookImport: async (file: File, title: string | null, progress?: (done: number, total: number) => void): Promise<Imported> => {
    const upload = await real.backupUpload(file, progress);
    return invoke<Imported>('book_import', { upload, name: file.name, title, today: todayIso() });
  },
  /** Android: книга, открытая «С помощью…» или присланная через «Поделиться». */
  bookImportIncoming: (name: string) => invoke<Imported>('book_import_incoming', { name, today: todayIso() }),
  bookOpen: (hash: string) => invoke<BookView>('book_open', { hash, today: todayIso() }),
  bookChapter: (hash: string, index: number) => invoke<BookNode[]>('book_chapter', { hash, index }),
  bookSearch: (hash: string, query: string) => invoke<BookHit[]>('book_search', { hash, query }),
  bookFiles: () => invoke<LocalBook[]>('book_files'),
  bookForget: (hash: string) => invoke<void>('book_forget', { hash }),
  /** PDF целиком — его страницы рисует pdf.js. */
  bookData: (hash: string) => invoke<ArrayBuffer>('book_data', { hash }),
  /** Обложка PDF (первая страница картинкой) — если у записи её ещё нет. */
  bookCover: (hash: string, dataUrl: string) => invoke<Title | null>('book_cover', { hash, dataUrl }),
  /** Файлы книг, которые есть только на других устройствах. */
  bookElsewhere: () => invoke<RemoteBook[]>('book_elsewhere'),
  /** Убранный файл снова придёт при обмене. */
  bookUnskip: (hash: string) => invoke<void>('book_unskip', { hash }),
  filesSettings: () => invoke<FilesSettings>('files_settings'),
  filesSet: (what: 'sync' | 'backup', on: boolean) => invoke<FilesSettings>('files_set', { what, on }),
  readings: () => invoke<Reading[]>('reading_list'),
  readingSave: (reading: Reading) => invoke<void>('reading_save', { reading }),
  /** Закладки и цитаты: все или одной записи. */
  marks: (title: string | null) => invoke<Mark[]>('marks', { title }),
  /** Новая (без номера) или изменённая закладка или цитата. */
  markSave: (mark: Mark) => invoke<Mark>('mark_save', { mark }),
  markDelete: (id: string) => invoke<void>('mark_delete', { id }),
  /** Внешняя ссылка из книги — в браузере. */
  openLink: (url: string) => invoke<void>('open_link', { url }),
  /** Компьютер: книгу перетащили в окно. */
  onBook: async (listener: (e: BookDrop) => void): Promise<() => void> => {
    const { listen } = await import('@tauri-apps/api/event');
    return listen<BookDrop>('book', (ev) => listener(ev.payload));
  },
  backupInspect: (id: string, password: string) => invoke<BackupInfo>('backup_inspect', { id, password }),
  backupRestore: (id: string, password: string) => invoke<RestoreResult>('backup_restore', { id, password }),
  backupDiscard: (id: string) => invoke<void>('backup_discard', { id }),
  lanStatus: () => invoke<LanView>('lan_status'),
  /** Поискать в сети сейчас (около секунды). */
  lanScan: () => invoke<LanView>('lan_scan'),
  lanCodeStart: () => invoke<LanView>('lan_code_start'),
  lanCodeStop: () => invoke<LanView>('lan_code_stop'),
  /** Связь по коду: с найденным устройством или по адресу. */
  lanPair: (device: string | null, addr: string | null, code: string) => invoke<PairResult>('lan_pair', { device, addr, code }),
  lanPairQr: (uri: string) => invoke<PairResult>('lan_pair_qr', { uri }),
  lanSyncNow: (id: string) => invoke<PairResult>('lan_sync_now', { id }),
  lanUnpair: (id: string) => invoke<LanView>('lan_unpair', { id }),
  /** Обмен по Wi-Fi или связь по коду: `received` — сколько нового пришло. */
  onLan: async (listener: (e: LanEvent) => void): Promise<() => void> => {
    const { listen } = await import('@tauri-apps/api/event');
    return listen<LanEvent>('lan', (ev) => listener(ev.payload));
  },
  /** Обложка из базы — для картинки года (холст рисует её без интернета). */
  coverBlob: async (id: string) => new Blob([await invoke<ArrayBuffer>('cover_bytes', { id })]),
  /** Компьютер: сохранить картинку через окно «Сохранить как»; null — передумали. */
  saveImage: (name: string, data: string) => invoke<string | null>('save_image', { name, data }),
  cloudStatus: () => invoke<CloudStatus>('cloud_status'),
  /** Вход по логину и паролю: Яндекс Диск (пароль приложения) или свой сервер WebDAV. */
  cloudDavLogin: (kind: string, url: string | null, user: string, password: string) => invoke<CloudStatus>('cloud_dav_login', { kind, url, user, password }),
  /** Страница Яндекс ID, где создают пароль приложения. */
  cloudAppPasswords: () => invoke<void>('cloud_app_passwords'),
  cloudPassword: (password: string) => invoke<CloudStatus>('cloud_password', { password }),
  cloudSyncNow: () => invoke<CloudStatus>('cloud_sync_now'),
  cloudEvery: (every: CloudStatus['every']) => invoke<CloudStatus>('cloud_every', { every }),
  cloudDisconnect: (wipe: boolean) => invoke<CloudStatus>('cloud_disconnect', { wipe }),
  /** Обмен через облако идёт или закончился: `received` — сколько нового пришло. */
  onCloud: async (listener: (e: CloudEvent) => void): Promise<() => void> => {
    const { listen } = await import('@tauri-apps/api/event');
    return listen<CloudEvent>('cloud', (ev) => listener(ev.payload));
  },
  /** Автокопия сделана или не получилась — обновить раздел копий. */
  onBackup: async (listener: () => void): Promise<() => void> => {
    const { listen } = await import('@tauri-apps/api/event');
    return listen('backup', listener);
  },
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

/** Адрес картинки из книги: её отдаёт протокол `book` приложения. */
export function bookImageSrc(hash: string, path: string): string {
  if (inApp) return convertFileSrc(`${hash}/${path}`, 'book');
  return demoBookImage(path);
}

/** Сканирование QR-кода связи камерой телефона. */
export async function scanQr(): Promise<string> {
  const scanner = await import('@tauri-apps/plugin-barcode-scanner');
  let perm = await scanner.checkPermissions();
  if (perm !== 'granted') perm = await scanner.requestPermissions();
  if (perm !== 'granted') throw new Error('Нет доступа к камере. Разрешите его в настройках телефона');
  const res = await scanner.scan({ windowed: false, formats: [scanner.Format.QRCode] });
  return res.content;
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
