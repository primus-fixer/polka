import { useCallback, useEffect, useMemo, useState } from 'react';
import { api, message, type Goal, type Title } from './api';
import { todayIso, yearOf } from './format';
import { applyTheme, type ThemeMode } from './theme';
import { Icon } from './ui';
import Add from './screens/Add';
import Card from './screens/Card';
import Home from './screens/Home';
import Settings from './screens/Settings';
import Stats from './screens/Stats';
import Want from './screens/Want';

export type Route =
  | { name: 'home' }
  | { name: 'want' }
  | { name: 'stats' }
  | { name: 'settings' }
  | { name: 'add'; replace?: string }
  | { name: 'card'; id: string };

export type Filter = 'all' | 'book' | 'screen';

export type Nav = {
  go: (r: Route) => void;
  back: () => void;
  /** Запись изменилась (или добавилась) — заменить в списке. */
  update: (t: Title) => void;
  removed: (id: string) => void;
  toast: (text: string) => void;
};

const TABS: { id: 'home' | 'want' | 'stats' | 'settings'; label: string; icon: string }[] = [
  { id: 'home', label: 'Полка', icon: 'shelf' },
  { id: 'want', label: 'Хочу', icon: 'want' },
  { id: 'stats', label: 'Итоги', icon: 'stats' },
  { id: 'settings', label: 'Настройки', icon: 'settings' },
];

function stored(key: string, fallback: string): string {
  try {
    return localStorage.getItem(key) ?? fallback;
  } catch {
    return fallback;
  }
}

function store(key: string, value: string) {
  try {
    localStorage.setItem(key, value);
  } catch {
    /* без хранилища просто не запомним */
  }
}

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
  const [toast, setToast] = useState<string | null>(null);
  const [view, setView] = useState<'shelf' | 'feed'>(() => (stored('polka.view', 'shelf') === 'feed' ? 'feed' : 'shelf'));
  const [filter, setFilter] = useState<Filter>('all');
  const [scheme, setScheme] = useState(() => stored('polka.scheme', 'shield'));
  const [mode, setMode] = useState<ThemeMode>(() => stored('polka.mode', 'system') as ThemeMode);
  const [goal, setGoal] = useState<Goal>({});
  const wide = useWide();
  const year = yearOf(todayIso());

  useEffect(() => {
    applyTheme(scheme, mode);
    store('polka.scheme', scheme);
    store('polka.mode', mode);
  }, [scheme, mode]);

  useEffect(() => store('polka.view', view), [view]);

  const loadGoal = useCallback(() => {
    api.settings(year).then((s) => setGoal(s.goal)).catch(() => undefined);
  }, [year]);

  useEffect(() => {
    api
      .titles()
      .then(setTitles)
      .catch((e) => setToast(message(e)))
      .finally(() => setLoaded(true));
    loadGoal();
  }, [loadGoal]);

  useEffect(() => {
    if (!toast) return;
    const id = window.setTimeout(() => setToast(null), 4500);
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
      toast: setToast,
    }),
    [route, history],
  );

  const goTab = (id: (typeof TABS)[number]['id']) => {
    setHistory([]);
    setRoute({ name: id } as Route);
    document.querySelector('.main')?.scrollTo(0, 0);
  };

  const prevTab = [...history].reverse().find((r) => r.name === 'want' || r.name === 'stats' || r.name === 'home');
  const tab = route.name === 'card' || route.name === 'add' ? (prevTab?.name ?? 'home') : route.name;
  const showNav = !wide && route.name !== 'add';
  const showFab = !wide && (route.name === 'home' || route.name === 'want');

  let body;
  if (!loaded) body = <p className="muted">Открываю полку…</p>;
  else if (route.name === 'home') body = <Home titles={titles} nav={nav} wide={wide} view={view} setView={setView} filter={filter} setFilter={setFilter} goal={goal} year={year} />;
  else if (route.name === 'want') body = <Want titles={titles} nav={nav} filter={filter} setFilter={setFilter} />;
  else if (route.name === 'stats') body = <Stats titles={titles} nav={nav} />;
  else if (route.name === 'settings')
    body = <Settings scheme={scheme} setScheme={setScheme} mode={mode} setMode={setMode} goal={goal} year={year} onGoal={loadGoal} nav={nav} />;
  else if (route.name === 'add') body = <Add titles={titles} nav={nav} replace={titles.find((t) => t.id === route.replace)} />;
  else {
    const t = titles.find((x) => x.id === route.id);
    body = t ? <Card key={t.id} t={t} titles={titles} nav={nav} wide={wide} /> : <p className="muted">Запись удалена.</p>;
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
      {toast && (
        <div className="toast" role="status" onClick={() => setToast(null)}>
          {toast}
        </div>
      )}
    </div>
  );
}
