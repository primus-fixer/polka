import { lazy, Suspense, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { api, LOCKED, message, type ClaudeEvent, type Goal, type Imported, type LocalBook, type LockStatus, type Reading, type RemoteBook, type Title } from './api';
import { todayIso, yearOf } from './format';
import { takeBook, takeOpened, takeShared } from './share';
import { pushWidget } from './widget';
import { Icon } from './ui';
import { LockScreen, PinFlow, applyTheme, store, stored, type PinMode, type ThemeMode } from '@primus-fixer/kit/ui';
import Add from './screens/Add';
import Card from './screens/Card';
import Home, { readSort, type ShelfSort } from './screens/Home';
import Settings from './screens/Settings';
import Stats from './screens/Stats';
import Want from './screens/Want';

// Читалка — отдельным куском: её код и шрифт грузятся, только когда открывают книгу.
const Reader = lazy(() => import('./screens/Reader'));
const PdfReader = lazy(() => import('./screens/PdfReader'));
/** Открыть книгу сразу на месте (цитата из карточки): глава, знак, длина подсветки. */
export type ReaderAt = { chapter: number; offset: number; len: number };

/** Название «Полки» на экране входа. */
const LOCK_NAMES = { name: 'Полка', nameAcc: 'Полку' };

export type Route =
  | { name: 'home' }
  | { name: 'want' }
  | { name: 'stats' }
  | { name: 'settings' }
  | { name: 'add'; replace?: string; shared?: string }
  | { name: 'card'; id: string }
  | { name: 'reader'; hash: string; at?: ReaderAt; pdf?: boolean };

export type Filter = 'all' | 'book' | 'screen';

export type Nav = {
  go: (r: Route) => void;
  back: () => void;
  /** Запись изменилась (или добавилась) — заменить в списке. */
  update: (t: Title) => void;
  removed: (id: string) => void;
  /** Вся полка заново — после восстановления из копии. */
  replaceAll: (list: Title[]) => void;
  toast: (text: string) => void;
  /** Открыть книгу в читалке (по хешу файла); `at` — сразу на этом месте. */
  read: (hash: string, at?: ReaderAt) => void;
  /** Файл книги — на полку и в читалку; `title` — привязать к этой записи. */
  importBook: (file: File, title?: string) => void;
};

/** Файлы книг здесь и на других устройствах, места чтения; `filesSync` — включён ли обмен файлами. */
export type Library = { files: LocalBook[]; readings: Reading[]; elsewhere: RemoteBook[]; filesSync: boolean };

const TABS: { id: 'home' | 'want' | 'stats' | 'settings'; label: string; icon: string }[] = [
  { id: 'home', label: 'Полка', icon: 'shelf' },
  { id: 'want', label: 'Хочу', icon: 'want' },
  { id: 'stats', label: 'Итоги', icon: 'stats' },
  { id: 'settings', label: 'Настройки', icon: 'settings' },
];

function useWide(): boolean {
  const query = '(min-width: 900px)';
  const [wide, setWide] = useState(() => window.matchMedia(query).matches);
  useEffect(() => {
    const mq = window.matchMedia(query);
    const on = () => setWide(mq.matches);
    mq.addEventListener('change', on);
    return () => mq.removeEventListener('change', on);
  }, []);
  return wide;
}

export default function App() {
  const [titles, setTitles] = useState<Title[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [route, setRoute] = useState<Route>({ name: 'home' });
  const [history, setHistory] = useState<Route[]>([]);
  const [toast, showToast] = useState<string | null>(null);
  // Claude записал что-то через коннектор: сообщение с кнопкой «Вернуть».
  const [claudeNote, setClaudeNote] = useState<ClaudeEvent | null>(null);
  // Вход по PIN-коду: состояние, экран входа поверх всего, экраны «задать/сменить PIN».
  const [lock, setLock] = useState<LockStatus | null>(null);
  const [locked, setLocked] = useState(false);
  const lockedRef = useRef(false);
  const [lockKey, setLockKey] = useState(0);
  const [pinFlow, setPinFlow] = useState<PinMode | null>(null);
  /** Экран входа — только если его ещё нет на экране (он сам спросит отпечаток). */
  const showLock = useCallback(() => {
    if (lockedRef.current) return;
    lockedRef.current = true;
    setLockKey((k) => k + 1);
    setLocked(true);
  }, []);
  /** «Полка» заблокирована: экран входа с актуальным состоянием. */
  const goLocked = useCallback(() => {
    api.lockStatus().then(
      (st) => {
        setLock(st);
        if (st.locked) showLock();
      },
      () => showLock(),
    );
  }, [showLock]);
  // Ответ «locked» от любой команды — не сообщение, а экран входа.
  const setToast = useCallback((text: string | null) => (text === LOCKED ? goLocked() : showToast(text)), [goLocked]);
  const [view, setView] = useState<'shelf' | 'feed'>(() => (stored('polka.view', 'shelf') === 'feed' ? 'feed' : 'shelf'));
  const [filter, setFilter] = useState<Filter>('all');
  // Поиск по полке: null — поле убрано. Не запоминается между запусками.
  const [query, setQuery] = useState<string | null>(null);
  // Порядок полки и метка — запоминаются.
  const [sort, setSort] = useState<ShelfSort>(readSort);
  const [scheme, setScheme] = useState(() => stored('polka.scheme', 'shield'));
  const [mode, setMode] = useState<ThemeMode>(() => stored('polka.mode', 'system') as ThemeMode);
  const [goal, setGoal] = useState<Goal>({});
  const [library, setLibrary] = useState<Library>({ files: [], readings: [], elsewhere: [], filesSync: false });
  const libraryRef = useRef(library);
  libraryRef.current = library;
  const loadedRef = useRef(loaded);
  loadedRef.current = loaded;
  const checkRef = useRef<() => void>(() => undefined);
  const wide = useWide();
  const year = yearOf(todayIso());

  useEffect(() => {
    applyTheme(scheme, mode);
    store('polka.scheme', scheme);
    store('polka.mode', mode);
  }, [scheme, mode]);

  useEffect(() => store('polka.view', view), [view]);
  useEffect(() => {
    store('polka.order', sort.order ?? '');
    store('polka.tag', sort.tag ?? '');
  }, [sort]);

  const loadGoal = useCallback(() => {
    api.settings(year).then((s) => setGoal(s.goal)).catch(() => undefined);
  }, [year]);

  const loadLibrary = useCallback(() => {
    Promise.all([api.bookFiles(), api.readings(), api.bookElsewhere().catch(() => []), api.filesSettings().catch(() => null)])
      .then(([files, readings, elsewhere, fs]) => setLibrary({ files, readings, elsewhere, filesSync: !!fs?.sync }))
      .catch(() => undefined);
  }, []);

  const reload = useCallback(() => {
    api.titles().then(setTitles).catch(() => undefined);
    loadGoal();
    loadLibrary();
  }, [loadGoal, loadLibrary]);

  const start = useCallback(() => {
    // Смещение местного времени — для имён резервных копий.
    api.setTz(-new Date().getTimezoneOffset()).catch(() => undefined);
    api
      .titles()
      .then(setTitles)
      .catch((e) => setToast(message(e)))
      .finally(() => setLoaded(true));
    loadGoal();
    loadLibrary();
  }, [loadGoal, loadLibrary, setToast]);
  const startRef = useRef(start);
  startRef.current = start;

  // Сначала — вход: с PIN-кодом данные приходят только после него.
  useEffect(() => {
    api.lockStatus().then(
      (st) => {
        setLock(st);
        if (st.locked) showLock();
        else startRef.current();
      },
      () => startRef.current(),
    );
    // Свернули — «Полка» считает время в фоне; вернулись через 5 минут — экран входа.
    const onVisible = () => {
      const visible = document.visibilityState === 'visible';
      api.appVisible(visible).then(
        (st) => {
          setLock(st);
          if (st.locked && visible) goLocked();
        },
        () => undefined,
      );
    };
    document.addEventListener('visibilitychange', onVisible);
    const unlisten = api.onLocked(goLocked).catch(() => () => undefined);
    return () => {
      document.removeEventListener('visibilitychange', onVisible);
      unlisten.then((f) => f());
    };
  }, [showLock, goLocked]);

  // Книга пришла (файл выбрали, перетащили или открыли «С помощью…»): на полку и сразу читать.
  const routeRef = useRef(route);
  routeRef.current = route;
  const opened = useCallback(
    (r: Imported) => {
      setTitles((ts) => (ts.some((x) => x.id === r.title.id) ? ts.map((x) => (x.id === r.title.id ? r.title : x)) : [...ts, r.title]));
      loadLibrary();
      setToast(r.created ? `«${r.title.title}» — теперь на полке, в «Читаю»` : null);
      setHistory((h) => [...h.slice(-20), routeRef.current]);
      setRoute({ name: 'reader', hash: r.hash, pdf: r.format === 'pdf' });
    },
    [loadLibrary],
  );
  const openedRef = useRef(opened);
  openedRef.current = opened;

  // Компьютер: файл книги перетащили в окно.
  useEffect(() => {
    const off = api.onBook((e) => (e.ok ? openedRef.current(e.ok) : e.error && setToast(e.error))).catch(() => () => undefined);
    return () => {
      off.then((f) => f());
    };
  }, []);

  // Обмен по Wi-Fi и через облако идёт в фоне: пришло новое — перечитать полку.
  useEffect(() => {
    const lan = api.onLan((e) => (e.received > 0 || e.paired) && reload()).catch(() => () => undefined);
    const cloud = api.onCloud((e) => !e.running && e.received > 0 && reload()).catch(() => () => undefined);
    return () => {
      lan.then((f) => f());
      cloud.then((f) => f());
    };
  }, [reload]);

  // Коннектор для приложения Claude: Claude что-то записал — перечитать полку и показать, что именно.
  useEffect(() => {
    const off = api
      .onClaude((e) => {
        reload();
        setClaudeNote(e);
      })
      .catch(() => () => undefined);
    return () => {
      off.then((f) => f());
    };
  }, [reload]);

  useEffect(() => {
    if (!claudeNote) return;
    const id = window.setTimeout(() => setClaudeNote(null), 12000);
    return () => window.clearTimeout(id);
  }, [claudeNote]);

  // «Поделиться» → «Полка» на Android: текст со ссылкой открывает добавление.
  useEffect(() => {
    if (!loaded) return;
    const check = () => {
      // Пока «Полка» заблокирована, присланное ждёт входа.
      if (lockedRef.current) return;
      // Из виджета «Сейчас»: открыть запись (или просто «Полку»).
      const open = takeOpened();
      if (open) {
        setHistory([{ name: 'home' }]);
        setRoute({ name: 'card', id: open });
      }
      // Книгу открыли «С помощью…» или прислали через «Поделиться»: файл уже у приложения.
      const book = takeBook();
      if (book) {
        setToast('Открываю книгу…');
        api
          .bookImportIncoming(book)
          .then((r) => openedRef.current(r))
          .catch((e) => setToast(message(e)));
      }
      const text = takeShared();
      if (!text) return;
      setHistory([{ name: 'home' }]);
      setRoute({ name: 'add', shared: text });
    };
    check();
    checkRef.current = check;
    const onVisible = () => document.visibilityState === 'visible' && check();
    document.addEventListener('visibilitychange', onVisible);
    window.addEventListener('focus', check);
    window.addEventListener('polka-book', check);
    return () => {
      document.removeEventListener('visibilitychange', onVisible);
      window.removeEventListener('focus', check);
      window.removeEventListener('polka-book', check);
    };
  }, [loaded, setToast]);

  const onUnlocked = useCallback(
    (st: LockStatus) => {
      setLock(st);
      lockedRef.current = false;
      setLocked(false);
      if (loadedRef.current) reload();
      else startRef.current();
      // Присланное, пока «Полка» была заблокирована.
      checkRef.current();
      if (st.recovery) setPinFlow('reset');
    },
    [reload],
  );
  const security = useMemo(
    () => ({
      lock,
      onPin: (m: PinMode) => setPinFlow(m),
      onBio: (on: boolean) => {
        api
          .bioSet(on)
          .then((st) => {
            setLock(st);
            if (st.error) setToast(st.error);
            else setToast(on ? 'Вход по отпечатку включён' : 'Вход по отпечатку выключен');
          })
          .catch((e) => setToast(message(e)));
      },
      onLockNow: () => {
        api
          .lockNow()
          .then((st) => {
            setLock(st);
            if (st.locked) showLock();
          })
          .catch((e) => setToast(message(e)));
      },
    }),
    [lock, setToast, showLock],
  );

  // Виджет «Сейчас» на главном экране Android — после изменений полки.
  useEffect(() => {
    if (!loaded) return;
    const id = window.setTimeout(() => pushWidget(titles).catch(() => undefined), 800);
    return () => window.clearTimeout(id);
  }, [titles, loaded]);

  useEffect(() => {
    if (!toast) return;
    const id = window.setTimeout(() => showToast(null), 4500);
    return () => window.clearTimeout(id);
  }, [toast]);

  const nav: Nav = useMemo(
    () => ({
      go: (r) => {
        setHistory([...history.slice(-20), route]);
        setRoute(r);
        document.querySelector('.main')?.scrollTo(0, 0);
      },
      back: () => {
        setRoute(history[history.length - 1] ?? { name: 'home' });
        setHistory(history.slice(0, -1));
      },
      update: (t) => setTitles((ts) => (ts.some((x) => x.id === t.id) ? ts.map((x) => (x.id === t.id ? t : x)) : [...ts, t])),
      removed: (id) => setTitles((ts) => ts.filter((x) => x.id !== id)),
      replaceAll: setTitles,
      toast: setToast,
      read: (hash, at) => {
        setHistory([...history.slice(-20), route]);
        setRoute({ name: 'reader', hash, at, pdf: libraryRef.current.files.some((f) => f.hash === hash && f.format === 'pdf') });
      },
      importBook: (file, title) => {
        setToast(`Открываю «${file.name}»…`);
        api
          .bookImport(file, title ?? null)
          .then((r) => openedRef.current(r))
          .catch((e) => setToast(message(e)));
      },
    }),
    [route, history],
  );

  const goTab = (id: (typeof TABS)[number]['id']) => {
    setHistory([]);
    setRoute({ name: id } as Route);
    document.querySelector('.main')?.scrollTo(0, 0);
  };

  const overlays = (
    <>
      {locked && lock && <LockScreen key={lockKey} status={lock} api={api} names={LOCK_NAMES} onUnlocked={onUnlocked} />}
      {pinFlow && !locked && (
        <PinFlow
          api={api}
          mode={pinFlow}
          onClose={() => setPinFlow(null)}
          onDone={(st, text) => {
            setLock(st);
            setPinFlow(null);
            showToast(text);
          }}
        />
      )}
    </>
  );

  const undoClaude = (id: number) => {
    setClaudeNote(null);
    api
      .claudeUndo(id)
      .then(() => {
        reload();
        setToast('Отменено');
      })
      .catch((e) => setToast(message(e)));
  };
  const toastView = claudeNote ? (
    <div className="toast with-action" role="status">
      <span>{claudeNote.text}</span>
      <button type="button" className="toast-btn" onClick={() => undoClaude(claudeNote.id)}>
        Вернуть
      </button>
    </div>
  ) : toast ? (
    <div className="toast" role="status" onClick={() => showToast(null)}>
      {toast}
    </div>
  ) : null;

  if (route.name === 'reader') {
    const BookReader = route.pdf ? PdfReader : Reader;
    return (
      <>
        {overlays}
        <Suspense fallback={<div className="rd-wait">Открываю книгу…</div>}>
          <BookReader
            key={`${route.hash}-${route.at?.chapter ?? ''}-${route.at?.offset ?? ''}`}
            hash={route.hash}
            at={route.at}
            nav={nav}
            titles={titles}
            onClose={(r) => {
              if (r) setLibrary((l) => ({ ...l, readings: [...l.readings.filter((x) => x.title !== r.title), r] }));
              loadLibrary();
              nav.back();
            }}
          />
        </Suspense>
        {toastView}
      </>
    );
  }

  const prevTab = [...history].reverse().find((r) => r.name === 'want' || r.name === 'stats' || r.name === 'home');
  const tab = route.name === 'card' || route.name === 'add' ? (prevTab?.name ?? 'home') : route.name;
  const showNav = !wide && route.name !== 'add';
  const showFab = !wide && (route.name === 'home' || route.name === 'want');

  let body;
  if (!loaded) body = <p className="muted">Открываю полку…</p>;
  else if (route.name === 'home') body = (
      <Home titles={titles} nav={nav} wide={wide} view={view} setView={setView} filter={filter} setFilter={setFilter} goal={goal} year={year} query={query} setQuery={setQuery} sort={sort} setSort={setSort} library={library} />
    );
  else if (route.name === 'want') body = <Want titles={titles} nav={nav} filter={filter} setFilter={setFilter} />;
  else if (route.name === 'stats') body = <Stats titles={titles} nav={nav} library={library} />;
  else if (route.name === 'settings')
    body = <Settings scheme={scheme} setScheme={setScheme} mode={mode} setMode={setMode} goal={goal} year={year} onGoal={loadGoal} onSynced={reload} nav={nav} security={security} />;
  else if (route.name === 'add') body = <Add key={route.shared ?? 'add'} titles={titles} nav={nav} replace={titles.find((t) => t.id === route.replace)} shared={route.shared} />;
  else {
    const t = titles.find((x) => x.id === route.id);
    body = t ? <Card key={t.id} t={t} titles={titles} nav={nav} wide={wide} library={library} onForget={loadLibrary} /> : <p className="muted">Запись удалена.</p>;
  }

  return (
    <div className={wide ? 'app wide' : 'app phone'}>
      {wide && (
        <aside className="side" aria-label="Разделы">
          <p className="wordmark num">Полка</p>
          <button type="button" className="btn primary side-add" onClick={() => nav.go({ name: 'add' })}>
            <Icon name="plus" stroke={2} />
            <span>Добавить</span>
          </button>
          {TABS.map((t) => (
            <button key={t.id} type="button" className={`navitem${tab === t.id ? ' on' : ''}`} aria-current={tab === t.id ? 'page' : undefined} onClick={() => goTab(t.id)}>
              <Icon name={t.icon} />
              <span>{t.label}</span>
            </button>
          ))}
        </aside>
      )}
      <main className="main">
        <div className={`inner${showFab ? ' with-fab' : ''}`}>{body}</div>
      </main>
      {showFab && (
        <button type="button" className="fab" aria-label="Добавить книгу, фильм или сериал" onClick={() => nav.go({ name: 'add' })}>
          <Icon name="plus" size={26} stroke={2} />
        </button>
      )}
      {showNav && (
        <nav className="bottomnav" aria-label="Разделы">
          {TABS.map((t) => (
            <button key={t.id} type="button" className={tab === t.id ? 'on' : ''} aria-current={tab === t.id ? 'page' : undefined} onClick={() => goTab(t.id)}>
              <Icon name={t.icon} size={22} stroke={tab === t.id ? 2 : 1.75} />
              <span>{t.label}</span>
            </button>
          ))}
        </nav>
      )}
      {toastView}
      {overlays}
    </div>
  );
}
