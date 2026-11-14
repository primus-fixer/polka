// Демо-данные для браузера: `npm run dev` и превью без приложения. Логика повторяет ядро в упрощённом виде.
import type { Api, BackupStatus, Candidate, CloudStatus, Found, Goal, LanView, Season, Status, Title, YearStats } from './api';
import { todayIso } from './format';

const base = { creators: [] as string[], tags: [] as string[], genres: [] as string[], seasons: [] as Season[], added: '2026-09-20' };
const T = (p: Partial<Title> & Pick<Title, 'id' | 'kind' | 'title' | 'status'>): Title => ({ ...base, ...p });
const seasons = (counts: number[], seen: number[], on: string): Season[] =>
  counts.map((n, i) => ({ no: i + 1, episodes: n, seen: Array.from({ length: seen[i] ?? 0 }, (_, k) => ({ ep: k + 1, on })) }));

let titles: Title[] = [
  T({ id: 'piknik', kind: 'book', title: 'Пикник на обочине', creators: ['Аркадий Стругацкий', 'Борис Стругацкий'], year: '1972', status: 'done', started: '2026-11-02', finished: '2026-11-12', rating: 9, tags: ['в командировке'], genres: ['Фантастика'], format: 'paper', publisher: 'АСТ', pages: 256, note: 'Перечитал в поездке. Финал по-прежнему держит.', cover_source: 'FantLab · АСТ, 2021' }),
  T({ id: 'inter', kind: 'film', title: 'Интерстеллар', creators: ['Кристофер Нолан'], year: '2014', status: 'done', finished: '2026-11-09', rating: 10, minutes: 169, genres: ['фантастика', 'драма'] }),
  T({ id: 'phm', kind: 'book', title: 'Project Hail Mary', creators: ['Andy Weir'], year: '2021', status: 'done', started: '2026-10-22', finished: '2026-11-06', rating: 9, genres: ['Фантастика'], format: 'ebook', pages: 496 }),
  T({ id: 'brat', kind: 'film', title: 'Брат', creators: ['Алексей Балабанов'], year: '1997', status: 'done', finished: '2026-11-03', rating: 8, tags: ['с семьёй'], minutes: 100 }),
  T({ id: 'pacan', kind: 'series', title: 'Слово пацана', creators: ['Жора Крыжовников'], year: '2023', status: 'done', started: '2026-10-20', finished: '2026-10-29', rating: 8, minutes: 55, seasons: seasons([8], [8], '2026-10-29') }),
  T({ id: 'lavr', kind: 'book', title: 'Лавр', creators: ['Евгений Водолазкин'], year: '2012', status: 'done', started: '2026-10-10', finished: '2026-10-24', rating: 10, genres: ['Проза'], format: 'paper', pages: 440 }),
  T({ id: 'dune2', kind: 'film', title: 'Дюна: Часть вторая', creators: ['Дени Вильнёв'], year: '2024', status: 'done', finished: '2026-10-19', rating: 8, tags: ['с семьёй'], minutes: 166 }),
  T({ id: 'habits', kind: 'book', title: 'Atomic Habits', creators: ['James Clear'], year: '2018', status: 'done', started: '2026-10-05', finished: '2026-10-14', rating: 7, tags: ['по работе'], genres: ['Нон-фикшн'], format: 'audio', minutes: 335 }),
  T({ id: 'azazel', kind: 'book', title: 'Азазель', creators: ['Борис Акунин'], year: '1998', status: 'done', started: '2026-10-01', finished: '2026-10-08', rating: 7, tags: ['в командировке'], genres: ['Детектив'], format: 'ebook', pages: 288 }),
  T({ id: 'stalker', kind: 'film', title: 'Сталкер', creators: ['Андрей Тарковский'], year: '1979', status: 'done', finished: '2026-10-02', rating: 9, minutes: 163 }),
  T({ id: 'goldfinch', kind: 'book', title: 'Щегол', creators: ['Донна Тартт'], year: '2013', status: 'doing', started: '2026-11-04', format: 'paper', pages: 832 }),
  T({ id: 'bb', kind: 'series', title: 'Во все тяжкие', original: 'Breaking Bad', creators: ['Винс Гиллиган'], year: '2008–2013', status: 'doing', started: '2026-10-15', tags: ['с семьёй'], kp: 8.9, imdb: 9.5, ratings_on: '2026-10-15', minutes: 47, genres: ['драма', 'криминал'], about: 'Учитель химии узнаёт о смертельном диагнозе и начинает варить метамфетамин, чтобы обеспечить семью.', seasons: seasons([7, 13, 13, 13, 16], [7, 13, 5], '2026-11-01') }),
  T({ id: 'cot', kind: 'book', title: 'Children of Time', creators: ['Adrian Tchaikovsky'], year: '2015', status: 'doing', started: '2026-11-10', format: 'ebook' }),
  T({ id: 'zul', kind: 'book', title: 'Зулейха открывает глаза', creators: ['Гузель Яхина'], year: '2015', status: 'want', added: '2026-11-11' }),
  T({ id: 'opp', kind: 'film', title: 'Оппенгеймер', creators: ['Кристофер Нолан'], year: '2023', status: 'want', added: '2026-11-10' }),
  T({ id: 'pir', kind: 'book', title: 'Piranesi', creators: ['Susanna Clarke'], year: '2020', status: 'want', added: '2026-11-02' }),
  T({ id: 'chern', kind: 'series', title: 'Чернобыль', creators: ['Юхан Ренк'], year: '2019', status: 'want', added: '2026-10-28', seasons: seasons([5], [], '') }),
];
// Демо с пустой полкой — как при первом запуске: адрес с «?empty».
if (typeof location !== 'undefined' && new URLSearchParams(location.search).has('empty')) titles = [];
let goal: Goal = { books: 24, screen: null };
let lan: LanView = { name: 'Демо', hosts: ['192.168.1.34'], code: null, note: null, peers: [], nearby: [] };
const QR_DEMO = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 29 29"><rect width="29" height="29" fill="#fff"/><path d="M1 1h7v7H1zM21 1h7v7h-7zM1 21h7v7H1z" fill="none" stroke="#15232D" stroke-width="2"/><path d="M11 3h2v2h-2zM14 6h3v2h-3zM10 10h4v3h-4zM16 12h3v4h-3zM12 17h2v5h-2zM20 18h4v2h-4zM17 23h3v3h-3zM23 22h3v4h-3z" fill="#15232D"/></svg>';
let backup: BackupStatus = { desktop: true, password: false, dir: 'C:\\Users\\Вы\\Documents\\Полка — копии', chosen: false, every: 'day', keep: 14, last: null, shared: null, error: null, running: false, cloud: false };
const CLOUD_OFF: CloudStatus = { desktop: true, state: 'off', provider: null, title: null, dav: null, cloud_exists: false, backup_password: false, every: 'auto', running: false, progress: null, last: null, error: null, key_pending: false, notice: null };
let cloud: CloudStatus = { ...CLOUD_OFF };
/** Раздел копий в демо обновляется, когда облако меняет пароль копий. */
const backupListeners = new Set<() => void>();
const backupChanged = () => backupListeners.forEach((l) => l());
let next = 1;
const clone = <X,>(x: X): X => JSON.parse(JSON.stringify(x));
const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));

function find(id: string): Title {
  const t = titles.find((x) => x.id === id);
  if (!t) throw new Error('Запись не найдена');
  return t;
}

function setStatus(t: Title, s: Status, today: string) {
  if (s === 'want') {
    delete t.started;
    delete t.finished;
  } else if (s === 'doing') {
    t.started = t.started ?? today;
    delete t.finished;
  } else if (!t.finished || t.status !== s) {
    t.finished = today;
  }
  t.status = s;
}

function follow(t: Title, today: string) {
  if (t.status === 'dropped') return;
  const total = t.seasons.reduce((a, s) => a + s.episodes, 0);
  const seen = t.seasons.reduce((a, s) => a + s.seen.length, 0);
  if (total > 0 && seen === total) {
    t.status = 'done';
    t.finished = t.finished ?? today;
  } else if (seen > 0) {
    t.status = 'doing';
    t.started = t.started ?? today;
    delete t.finished;
  } else if (t.status === 'done') {
    t.status = t.started ? 'doing' : 'want';
    delete t.finished;
  }
}

/** Обложка-образец для превью: в браузере каталоги не опрашиваются, поэтому варианты рисуются здесь. */
function demoCover(title: string, by: string, bg: string, fg: string, style: 0 | 1 | 2 | 3): string {
  const esc = (x: string) => x.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
  const lines: string[] = [];
  for (const w of title.split(' ')) {
    const last = lines[lines.length - 1];
    if (last && `${last} ${w}`.length <= 11) lines[lines.length - 1] = `${last} ${w}`;
    else lines.push(w);
  }
  const deco = [
    `<rect x="12" y="12" width="176" height="276" fill="none" stroke="${fg}" stroke-opacity=".55" stroke-width="2"/>`,
    `<circle cx="100" cy="222" r="50" fill="${fg}" fill-opacity=".2"/>`,
    `<rect y="226" width="200" height="74" fill="${fg}" fill-opacity=".16"/>`,
    `<path d="M0 244 L200 186 L200 300 L0 300 Z" fill="${fg}" fill-opacity=".14"/>`,
  ][style];
  const top = style === 2 ? 64 : 98;
  const text = lines
    .slice(0, 4)
    .map((l, i) => `<text x="100" y="${top + i * 27}" text-anchor="middle" font-family="Georgia, 'Times New Roman', serif" font-size="23" font-weight="700" fill="${fg}">${esc(l)}</text>`)
    .join('');
  const author = `<text x="100" y="${style === 2 ? 268 : 54}" text-anchor="middle" font-family="Arial, sans-serif" font-size="11" letter-spacing="1.2" fill="${fg}" fill-opacity=".85">${esc(by.toUpperCase())}</text>`;
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 200 300"><rect width="200" height="300" fill="${bg}"/>${deco}${text}${author}</svg>`;
  return `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`;
}

const results: Record<'book' | 'film', Candidate[]> = {
  book: [
    { kind: 'book', title: 'Пикник на обочине', creators: ['Аркадий Стругацкий', 'Борис Стругацкий'], year: '1972', publisher: 'АСТ', genres: ['Фантастика'], pages: 256, covers: [
      { url: demoCover('Пикник на обочине', 'Стругацкие', '#1f3a4d', '#f2c14e', 0), source: 'FantLab', edition: 'АСТ, 2021' }, { url: demoCover('Пикник на обочине', 'Стругацкие', '#7a2e2e', '#f4e9d8', 1), source: 'FantLab', edition: 'АСТ, 2017' }, { url: demoCover('Пикник на обочине', 'А. и Б. Стругацкие', '#e8e1d0', '#2b2b2b', 2), source: 'Google Books', edition: 'АСТ, 2015' }, { url: demoCover('Пикник на обочине', 'Стругацкие', '#2f4f3a', '#e9e4d4', 3), source: 'Open Library', edition: '' }],
      sources: ['FantLab', 'Google Books', 'Open Library'], seasons: [], refs: [] },
    { kind: 'book', title: 'Roadside Picnic', creators: ['Arkady Strugatsky', 'Boris Strugatsky'], year: '1977', genres: [], covers: [{ url: demoCover('Roadside Picnic', 'Strugatsky', '#151515', '#e8b34a', 0), source: 'Open Library', edition: '' }], sources: ['Open Library', 'Google Books'], seasons: [], refs: [] },
  ],
  film: [
    { kind: 'series', title: 'Во все тяжкие', original: 'Breaking Bad', creators: ['Винс Гиллиган'], year: '2008–2013', genres: ['драма', 'криминал'], kp: 8.9, imdb: 9.5, minutes: 47, covers: [{ url: demoCover('Во все тяжкие', 'Breaking Bad', '#2e4a2b', '#f0e6a0', 2), source: 'ПоискКино', edition: 'Постер' }], sources: ['ПоискКино'], seasons: [[1, 7], [2, 13], [3, 13], [4, 13], [5, 16]], refs: [] },
    { kind: 'film', title: 'Путь: Во все тяжкие. Фильм', creators: ['Винс Гиллиган'], year: '2019', genres: [], covers: [{ url: demoCover('Путь', 'El Camino', '#3b3b3b', '#e6d27a', 3), source: 'ПоискКино', edition: 'Постер' }], sources: ['ПоискКино'], seasons: [], refs: [] },
  ],
};

function stats(year: number): YearStats {
  const s: YearStats = { year, goal, books: 0, films: 0, series: 0, episodes: 0, pages: 0, audio_minutes: 0, screen_minutes: 0, dropped: 0, avg_rating: null, months: Array.from({ length: 12 }, () => ({ books: 0, screen: 0 })), genres: [], authors: [], best: [], books_left: null };
  const genres = new Map<string, number>();
  const authors = new Map<string, number>();
  const ratings: number[] = [];
  let bestBook: Title | null = null;
  let bestScreen: Title | null = null;
  for (const t of titles) {
    if (t.kind === 'series') {
      const eps = t.seasons.flatMap((x) => x.seen).filter((x) => x.on.startsWith(String(year))).length;
      s.episodes += eps;
      s.screen_minutes += eps * (t.minutes ?? 0);
    }
    if (!t.finished?.startsWith(String(year))) continue;
    if (t.status === 'dropped') { s.dropped++; continue; }
    if (t.status !== 'done') continue;
    const m = Number(t.finished.slice(5, 7)) - 1;
    if (t.rating) ratings.push(t.rating);
    if (t.kind === 'book') {
      s.books++;
      s.months[m].books++;
      if (t.format === 'audio') s.audio_minutes += t.minutes ?? 0; else s.pages += t.pages ?? 0;
      t.genres.forEach((g) => genres.set(g, (genres.get(g) ?? 0) + 1));
      t.creators.forEach((a) => authors.set(a, (authors.get(a) ?? 0) + 1));
      if ((t.rating ?? 0) > (bestBook?.rating ?? 0)) bestBook = t;
    } else {
      if (t.kind === 'film') { s.films++; s.screen_minutes += t.minutes ?? 0; } else s.series++;
      s.months[m].screen++;
      if ((t.rating ?? 0) > (bestScreen?.rating ?? 0)) bestScreen = t;
    }
  }
  if (ratings.length) s.avg_rating = Math.round((ratings.reduce((a, b) => a + b, 0) / ratings.length) * 10) / 10;
  const top = (m: Map<string, number>, n: number) => [...m].map(([name, n]) => ({ name, n })).sort((a, b) => b.n - a.n || a.name.localeCompare(b.name)).slice(0, n);
  s.genres = top(genres, 6);
  s.authors = top(authors, 5).filter((a) => a.n > 1);
  s.best = [bestBook, bestScreen].filter((t): t is Title => !!t).map((t) => t.id);
  s.books_left = goal.books ? Math.max(0, goal.books - s.books) : null;
  return s;
}

export const demo: Api = {
  titles: async () => clone(titles),
  save: async (t) => {
    const x = clone(t);
    if (!x.id) x.id = `demo${next++}`;
    titles = titles.filter((y) => y.id !== x.id).concat(x);
    return clone(x);
  },
  remove: async (id) => {
    titles = titles.filter((t) => t.id !== id);
  },
  setStatus: async (id, status) => {
    const t = find(id);
    setStatus(t, status, todayIso());
    return clone(t);
  },
  markEpisode: async (id, season, ep, seen) => {
    const t = find(id);
    const s = t.seasons.find((x) => x.no === season);
    if (!s) throw new Error(`Нет сезона ${season}`);
    s.seen = s.seen.filter((x) => x.ep !== ep);
    if (seen) s.seen.push({ ep, on: todayIso() });
    s.seen.sort((a, b) => a.ep - b.ep);
    follow(t, todayIso());
    return clone(t);
  },
  markSeason: async (id, season, seen) => {
    const t = find(id);
    const s = t.seasons.find((x) => x.no === season);
    if (!s) throw new Error(`Нет сезона ${season}`);
    s.seen = seen ? Array.from({ length: s.episodes }, (_, k) => s.seen.find((x) => x.ep === k + 1) ?? { ep: k + 1, on: todayIso() }) : [];
    follow(t, todayIso());
    return clone(t);
  },
  search: async (kind): Promise<Found> => {
    await wait(400);
    return kind === 'film'
      ? { results: clone(results.film), failed: [] }
      : { results: clone(results.book), failed: [{ source: 'Демо', message: 'В браузере каталоги не опрашиваются — показан пример' }] };
  },
  fromLink: async () => {
    throw new Error('В демо добавление по ссылке не работает');
  },
  details: async (c) => clone(c),
  add: async (c, url, label, form) => {
    const t: Title = {
      ...base,
      id: `demo${next++}`,
      cover: url ?? undefined,
      kind: c.kind,
      title: c.title,
      creators: c.creators,
      year: c.year ?? undefined,
      genres: c.genres,
      pages: c.pages ?? undefined,
      minutes: c.minutes ?? undefined,
      kp: c.kp ?? undefined,
      imdb: c.imdb ?? undefined,
      about: c.about ?? undefined,
      status: 'want',
      added: todayIso(),
      tags: form.tags,
      format: form.format ?? undefined,
      cover_source: label ?? undefined,
      seasons: c.seasons.map(([no, episodes]) => ({ no, episodes, seen: [] })),
    };
    setStatus(t, form.status, form.date);
    if (form.rating && (form.status === 'done' || form.status === 'dropped')) t.rating = form.rating;
    titles.push(t);
    return clone(t);
  },
  setCoverUrl: async (id, url, label) => {
    const t = find(id);
    t.cover = url;
    t.cover_source = label || undefined;
    return clone(t);
  },
  setCoverPhoto: async (id, dataUrl) => {
    const t = find(id);
    t.cover = dataUrl;
    t.cover_source = 'своё фото';
    return clone(t);
  },
  stats: async (year) => stats(year),
  settings: async () => ({ goal, has_poiskkino: false, has_google: false, key_method: 'demo' }),
  setGoal: async (_year, g) => {
    goal = g;
  },
  setKey: async () => {},
  setTz: async () => {},
  backupStatus: async () => clone(backup),
  backupPassword: async (password) => {
    if ([...password].length < 8) throw new Error('Пароль копии — не короче 8 символов');
    backup = { ...backup, password: true };
    return clone(backup);
  },
  backupEvery: async (every) => {
    backup = { ...backup, every, keep: every === 'week' ? 8 : 14 };
    return clone(backup);
  },
  backupDir: async () => clone(backup),
  backupPickDir: async () => clone(backup),
  backupNow: async () => {
    await wait(700);
    const at = Math.floor(Date.now() / 1000);
    const last = { at, path: `${backup.dir}\\Полка копия.polka`, auto: false, bytes: 2_400_000, events: titles.length * 3, blobs: 4 };
    backup = { ...backup, last, shared: `${at}|Демо` };
    return clone(last);
  },
  backupUpload: async () => {
    throw new Error('В демо восстановление из копии не работает');
  },
  backupInspect: async () => {
    throw new Error('В демо восстановление из копии не работает');
  },
  backupRestore: async () => {
    throw new Error('В демо восстановление из копии не работает');
  },
  backupDiscard: async () => {},
  onBackup: async (listener) => {
    backupListeners.add(listener);
    return () => {
      backupListeners.delete(listener);
    };
  },
  lanStatus: async () => clone(lan),
  lanScan: async () => {
    await wait(400);
    return clone(lan);
  },
  lanCodeStart: async () => {
    lan = { ...lan, code: { code: '6WVG-HEWK', secs: 300, left: 5, wrong: 0, qr_svg: QR_DEMO, hosts: lan.hosts, port: 47510 } };
    return clone(lan);
  },
  lanCodeStop: async () => {
    lan = { ...lan, code: null };
    return clone(lan);
  },
  lanPair: async () => {
    throw new Error('В демо обмен по Wi-Fi не работает');
  },
  lanPairQr: async () => {
    throw new Error('В демо обмен по Wi-Fi не работает');
  },
  lanSyncNow: async () => {
    throw new Error('В демо обмен по Wi-Fi не работает');
  },
  lanUnpair: async () => clone(lan),
  onLan: async () => () => undefined,
  coverBlob: async (id) => (await fetch(id)).blob(),
  saveImage: async (name, data) => {
    const a = document.createElement('a');
    a.href = `data:image/png;base64,${data}`;
    a.download = name;
    a.click();
    return name;
  },
  // Облако в демо — понарошку: вход и обмен только изображаются, файлы никуда не уходят.
  cloudStatus: async () => clone({ ...cloud, backup_password: backup.password, notice: null }),
  cloudDavLogin: async (kind, url, user, password) => {
    await wait(600);
    if (!user.trim()) throw new Error(kind === 'yadisk' ? 'Введите логин Яндекса' : 'Введите логин');
    if (!password) throw new Error(kind === 'yadisk' ? 'Введите пароль приложения' : 'Введите пароль');
    if (kind === 'webdav' && !/^https:\/\/\S+/i.test(url ?? '')) throw new Error('Адрес сервера начинается с https:// — без шифрования пароль ушёл бы по сети открытым');
    const host = kind === 'yadisk' ? 'webdav.yandex.ru' : (url ?? '').replace(/^https:\/\//i, '').split('/')[0];
    cloud = { ...cloud, state: 'password', provider: kind, title: kind === 'yadisk' ? 'Яндекс Диск' : 'Сервер WebDAV', dav: { url: url ?? 'https://webdav.yandex.ru', host, user: user.trim() }, error: null };
    return clone({ ...cloud, backup_password: backup.password, notice: null });
  },
  cloudAppPasswords: async () => {},
  cloudPassword: async (password) => {
    await wait(700);
    if ([...password].length < 8) throw new Error('Пароль копии — не короче 8 символов');
    const notice = backup.password ? null : 'Этот пароль теперь и пароль резервных копий на этом компьютере.';
    backup = { ...backup, password: true, cloud: true };
    backupChanged();
    const at = Math.floor(Date.now() / 1000);
    cloud = { ...cloud, state: 'ready', cloud_exists: true, last: { at, received: 0, sent: titles.length * 2, files_received: 0, files_sent: 4, files_waiting: 0, problems: [] } };
    return clone({ ...cloud, backup_password: true, notice });
  },
  cloudSyncNow: async () => {
    await wait(500);
    cloud = { ...cloud, last: { at: Math.floor(Date.now() / 1000), received: 0, sent: 0, files_received: 0, files_sent: 0, files_waiting: 0, problems: [] } };
    return clone({ ...cloud, backup_password: backup.password, notice: null });
  },
  cloudEvery: async (every) => {
    cloud = { ...cloud, every };
    return clone({ ...cloud, backup_password: backup.password, notice: null });
  },
  cloudDisconnect: async (wipe) => {
    await wait(400);
    const ya = cloud.provider === 'yadisk';
    cloud = { ...CLOUD_OFF };
    backup = { ...backup, cloud: false };
    backupChanged();
    let notice = wipe ? 'Облако отключено, данные «Полки» из него удалены.' : 'Облако отключено. Данные «Полки» в нём остались — его можно подключить снова.';
    if (ya) notice += ' Пароль приложения больше не нужен «Полке» — его можно удалить в Яндекс ID.';
    return clone({ ...cloud, backup_password: backup.password, notice });
  },
  onCloud: async () => () => undefined,
};
