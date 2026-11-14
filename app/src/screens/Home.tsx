import { useEffect, useRef, useState } from 'react';
import type { Filter, Nav } from '../App';
import { api, message, type Goal, type Title } from '../api';
import { dayShort, monthTitle, plural } from '../format';
import { stored } from '../storage';
import { Bar, Cover, Icon, Seg, shortBy } from '../ui';

export const FILTERS: [Filter, string][] = [
  ['all', 'Всё'],
  ['book', 'Книги'],
  ['screen', 'Кино и сериалы'],
];

export const passes = (f: Filter) => (t: Title) => f === 'all' || (f === 'book' ? t.kind === 'book' : t.kind !== 'book');

const KIND_LABEL = { book: 'книга', film: 'фильм', series: 'сериал' } as const;

export function kindLabel(t: Title): string {
  return KIND_LABEL[t.kind];
}

/** «книга · Стругацкие», «фильм · 2014 · Нолан», «сериал · 2023». */
export function subLine(t: Title): string {
  const who = t.creators[0]?.split(' ').pop() ?? '';
  if (t.kind === 'book') return `книга · ${shortBy(t)}`;
  if (t.kind === 'film') return ['фильм', t.year, who].filter(Boolean).join(' · ');
  return ['сериал', t.year].filter(Boolean).join(' · ');
}

/** Порядок полки: по автору, жанру или году выхода; метка оставляет только записи с ней. */
export type Order = 'author' | 'genre' | 'year';
export type ShelfSort = { order: Order | null; tag: string | null };

export const ORDERS: [Order, string, string][] = [
  ['author', 'Автор', 'по автору'],
  ['genre', 'Жанр', 'по жанру'],
  ['year', 'Год', 'по году выхода'],
];

/** Последний выбор порядка и метки на этом устройстве. */
export function readSort(): ShelfSort {
  const o = stored('polka.order', '');
  return { order: ORDERS.some(([k]) => k === o) ? (o as Order) : null, tag: stored('polka.tag', '') || null };
}

const fold = (s: string) => s.toLocaleLowerCase('ru').replace(/ё/g, 'е');

/** Поиск по полке: каждое слово запроса — в названии, авторе, метках или заметке. */
export function matches(t: Title, query: string): boolean {
  const words = fold(query).split(/\s+/).filter(Boolean);
  if (!words.length) return true;
  const hay = fold([t.title, t.original ?? '', ...t.creators, ...t.tags, t.note ?? ''].join('\n'));
  return words.every((w) => hay.includes(w));
}

const collator = new Intl.Collator('ru', { sensitivity: 'base', numeric: true });
/** Фамилия первого автора: «Аркадий Стругацкий» → «Стругацкий». */
const surname = (t: Title) => (t.creators[0] ?? '').trim().split(/\s+/).pop() ?? '';
const yearNo = (t: Title) => Number(/\d{4}/.exec(t.year ?? '')?.[0] ?? 0);

/** Записи в выбранном порядке; у кого нет автора, жанра или года — в конце. */
export function ordered(list: Title[], order: Order): Title[] {
  const byTitle = (a: Title, b: Title) => collator.compare(a.title, b.title);
  const text = (key: (t: Title) => string) => (a: Title, b: Title) => {
    const [x, y] = [key(a), key(b)];
    if (!x !== !y) return x ? -1 : 1;
    return collator.compare(x, y) || byTitle(a, b);
  };
  const cmp =
    order === 'author'
      ? text(surname)
      : order === 'genre'
        ? text((t) => t.genres[0] ?? '')
        : (a: Title, b: Title) => {
            const [x, y] = [yearNo(a), yearNo(b)];
            if (!x !== !y) return x ? -1 : 1;
            return y - x || byTitle(a, b);
          };
  return [...list].sort(cmp);
}

/** Подпись под обложкой в выбранном порядке: автор, жанр или год. */
function orderLine(t: Title, order: Order): string {
  if (order === 'author') return t.kind === 'book' ? shortBy(t) || 'автор не указан' : t.creators[0] || 'автор не указан';
  if (order === 'genre') return t.genres[0] || 'жанр не указан';
  return t.year || 'год не указан';
}

export function seriesLine(t: Title): { text: string; pct: number; next: [number, number] | null } {
  const total = t.seasons.reduce((a, s) => a + s.episodes, 0);
  const seen = t.seasons.reduce((a, s) => a + s.seen.length, 0);
  let next: [number, number] | null = null;
  for (const s of [...t.seasons].sort((a, b) => a.no - b.no)) {
    for (let ep = 1; ep <= s.episodes && !next; ep++) if (!s.seen.some((x) => x.ep === ep)) next = [s.no, ep];
    if (next) break;
  }
  const text = total === 0 ? 'серии не указаны' : next ? `Сезон ${next[0]} · серия ${next[1]} следующая` : 'все серии просмотрены';
  return { text, pct: total ? Math.round((seen / total) * 100) : 0, next };
}

type Props = {
  titles: Title[];
  nav: Nav;
  wide: boolean;
  view: 'shelf' | 'feed';
  setView: (v: 'shelf' | 'feed') => void;
  filter: Filter;
  setFilter: (f: Filter) => void;
  goal: Goal;
  year: number;
  /** Поиск по полке; null — поле убрано. */
  query: string | null;
  setQuery: (q: string | null) => void;
  sort: ShelfSort;
  setSort: (s: ShelfSort) => void;
};

export default function Home({ titles, nav, wide, view, setView, filter, setFilter, goal, year, query, setQuery, sort, setSort }: Props) {
  const [busy, setBusy] = useState<string | null>(null);
  const [sorting, setSorting] = useState(false);
  const input = useRef<HTMLInputElement>(null);
  const searching = query !== null;
  useEffect(() => {
    if (searching) input.current?.focus();
  }, [searching]);

  // Метки с полки — чаще встречающиеся первыми.
  const tagCount = new Map<string, number>();
  for (const t of titles) if (t.status !== 'want') for (const x of t.tags) tagCount.set(x, (tagCount.get(x) ?? 0) + 1);
  const tags = [...tagCount.keys()].sort((a, b) => (tagCount.get(b) ?? 0) - (tagCount.get(a) ?? 0) || collator.compare(a, b));
  // Запомненной метки на полке уже нет — не отбираем по ней.
  const tag = sort.tag && tagCount.has(sort.tag) ? sort.tag : null;
  const order = sort.order;
  const q = query?.trim() ?? '';
  const narrowed = q !== '' || tag !== null;

  const kindPass = passes(filter);
  const pass = (t: Title) => kindPass(t) && (!tag || t.tags.includes(tag)) && (!q || matches(t, q));
  const now = titles.filter((t) => t.status === 'doing' && pass(t)).sort((a, b) => (b.started ?? '').localeCompare(a.started ?? ''));
  const byDate = titles
    .filter((t) => (t.status === 'done' || (view === 'feed' && t.status === 'dropped')) && pass(t))
    .sort((a, b) => (b.finished ?? b.added).localeCompare(a.finished ?? a.added));
  const finished = order ? ordered(byDate, order) : byDate;
  // Поиск находит и то, что в списке «Хочу».
  const wanted = q ? titles.filter((t) => t.status === 'want' && pass(t)).sort((a, b) => b.added.localeCompare(a.added)) : [];
  const orderLabel = ORDERS.find(([k]) => k === order)?.[2];
  const booksThisYear = titles.filter((t) => t.kind === 'book' && t.status === 'done' && t.finished?.startsWith(String(year))).length;
  const open = (t: Title) => nav.go({ name: 'card', id: t.id });

  const quick = async (t: Title) => {
    setBusy(t.id);
    try {
      if (t.kind === 'series') {
        const next = seriesLine(t).next;
        if (!next) return;
        nav.update(await api.markEpisode(t.id, next[0], next[1], true));
      } else {
        nav.update(await api.setStatus(t.id, 'done'));
        nav.toast(`«${t.title}» — готово. Поставьте оценку`);
        open(t);
      }
    } catch (e) {
      nav.toast(message(e));
    } finally {
      setBusy(null);
    }
  };

  const groups: { key: string; title: string; rows: Title[] }[] = [];
  if (order) groups.push({ key: 'all', title: 'Готово', rows: finished });
  else for (const t of finished) {
    const d = t.finished ?? t.added;
    const key = d.slice(0, 7);
    const g = groups.find((x) => x.key === key);
    if (g) g.rows.push(t);
    else groups.push({ key, title: monthTitle(d, year), rows: [t] });
  }
  const summary = (rows: Title[]) => {
    const b = rows.filter((r) => r.kind === 'book').length;
    const f = rows.filter((r) => r.kind === 'film').length;
    const s = rows.filter((r) => r.kind === 'series').length;
    return [b && plural(b, 'книга', 'книги', 'книг'), f && plural(f, 'фильм', 'фильма', 'фильмов'), s && plural(s, 'сериал', 'сериала', 'сериалов')].filter(Boolean).join(' · ');
  };

  const empty = titles.length === 0;
  const pick = (next: ShelfSort) => setSort(next);

  return (
    <div className="stack">
      {!empty && (
        <div className="shelf-top">
          <h1 className="screen-title">Полка</h1>
          <span className="shelf-tools">
            <button
              type="button"
              className={'tool' + (searching ? ' on' : '')}
              aria-label={searching ? 'Убрать поиск' : 'Поиск по полке'}
              aria-expanded={searching}
              onClick={() => setQuery(searching ? null : '')}
            >
              <Icon name="search" />
            </button>
            <button
              type="button"
              className={'tool' + (sorting || order || tag ? ' on' : '')}
              aria-label="Порядок и метки"
              aria-expanded={sorting}
              onClick={() => setSorting(!sorting)}
            >
              <Icon name="sort" />
            </button>
          </span>
        </div>
      )}

      <div className={'slide' + (searching ? ' open' : '')} inert={!searching}>
        <div>
          <label className="search-field">
            <Icon name="search" />
            <input
              ref={input}
              type="search"
              enterKeyHint="search"
              placeholder="Название, автор, метка или заметка"
              aria-label="Поиск по полке"
              value={query ?? ''}
              onChange={(e) => setQuery(e.target.value)}
              onKeyDown={(e) => e.key === 'Escape' && setQuery(null)}
            />
            {q && (
              <button type="button" className="clear" aria-label="Очистить поиск" onClick={() => (setQuery(''), input.current?.focus())}>
                <Icon name="x" size={18} />
              </button>
            )}
          </label>
        </div>
      </div>

      <div className={'slide' + (sorting ? ' open' : '')} inert={!sorting}>
        <div>
          <div className="chips sort-chips" role="group" aria-label="Порядок полки и метки">
            {ORDERS.map(([k, label]) => (
              <button key={k} type="button" className={'chip' + (order === k ? ' sel' : '')} aria-pressed={order === k} onClick={() => pick({ order: order === k ? null : k, tag })}>
                {label}
              </button>
            ))}
            {tags.length > 0 && <span className="chips-gap" aria-hidden="true" />}
            {tags.map((x) => (
              <button key={x} type="button" className={'chip tag-chip' + (tag === x ? ' sel' : '')} aria-pressed={tag === x} onClick={() => pick({ order, tag: tag === x ? null : x })}>
                {x}
              </button>
            ))}
          </div>
          <p className="small muted3 sort-hint">{tags.length ? 'Автор, жанр и год задают порядок, метка оставляет только записи с ней. Ещё раз — снять.' : 'Автор, жанр и год задают порядок. Метки появятся здесь, когда вы добавите их в записи.'}</p>
        </div>
      </div>

      {!sorting && tag && (
        <p className="small muted only-tag">
          Только с меткой «{tag}» ·{' '}
          <button type="button" className="link-btn" onClick={() => pick({ order, tag: null })}>
            снять
          </button>
        </p>
      )}

      <div className="home-head">
        <Seg small label="Что показывать" value={filter} options={FILTERS} onChange={setFilter} />
        <div className="seg" role="group" aria-label="Вид" style={{ gridAutoFlow: 'column' }}>
          <button type="button" className={`icon-seg${view === 'shelf' ? ' on' : ''}`} aria-pressed={view === 'shelf'} aria-label="Полка обложек" onClick={() => setView('shelf')}>
            <Icon name="grid" />
          </button>
          <button type="button" className={`icon-seg${view === 'feed' ? ' on' : ''}`} aria-pressed={view === 'feed'} aria-label="Лента по датам" onClick={() => setView('feed')}>
            <Icon name="list" />
          </button>
        </div>
      </div>

      {empty && (
        <section className="empty">
          <h1 className="screen-title">Полка пока пустая</h1>
          <p className="muted">Добавьте книгу, фильм или сериал: найдите по названию, отсканируйте штрихкод книги или вставьте ссылку на страницу — обложка подтянется сама.</p>
          <button type="button" className="btn primary" onClick={() => nav.go({ name: 'add' })}>
            <Icon name="plus" stroke={2} />
            <span>Добавить</span>
          </button>
        </section>
      )}

      {now.length > 0 && (
        <section>
          <h2 className="sec">Сейчас</h2>
          <div className="now" style={{ gridTemplateColumns: `repeat(${wide ? 3 : 1}, minmax(0, 1fr))` }}>
            {now.map((t) => {
              const s = t.kind === 'series' ? seriesLine(t) : null;
              const line = s ? s.text : [t.kind === 'book' ? shortBy(t) : t.year, t.started && `с ${dayShort(t.started)}`].filter(Boolean).join(' · ');
              return (
                <div key={t.id} className="now-row">
                  <button type="button" className="now-open" onClick={() => open(t)}>
                    <Cover t={t} size="mini" />
                    <span className="lines">
                      <span className="title">{t.title}</span>
                      <span className="sub">{line}</span>
                      {s && <Bar pct={s.pct} color="var(--work)" />}
                    </span>
                  </button>
                  {(!s || s.next) && (
                    <button
                      type="button"
                      className="btn line small"
                      style={{ color: t.kind === 'book' ? 'var(--own)' : 'var(--work)' }}
                      disabled={busy === t.id}
                      aria-label={s ? `Отметить серию ${s.next?.[1]} сезона ${s.next?.[0]}` : `Отметить как готово: ${t.title}`}
                      onClick={() => quick(t)}
                    >
                      {s ? '+1' : 'Готово'}
                    </button>
                  )}
                </div>
              );
            })}
          </div>
        </section>
      )}

      {view === 'shelf' && goal.books && filter !== 'screen' && !narrowed ? (
        <button type="button" className="goal-line" onClick={() => nav.go({ name: 'stats' })}>
          <span className="goal-text">
            <span>Цель на {year} год</span>
            <span className="num ink">
              {booksThisYear} из {plural(goal.books, 'книги', 'книг', 'книг')}
            </span>
          </span>
          <Bar pct={(booksThisYear / goal.books) * 100} thick />
        </button>
      ) : null}

      {view === 'shelf' && finished.length > 0 && (
        <section>
          <div className="sec-head">
            <h2 className="sec">Готово</h2>
            <span className="small muted3">{orderLabel ?? 'сначала новые'}</span>
          </div>
          <div className="shelf" style={{ gridTemplateColumns: `repeat(${wide ? 6 : 3}, minmax(0, 1fr))` }}>
            {finished.map((t) => (
              <button key={t.id} type="button" className="shelf-item" onClick={() => open(t)}>
                <Cover t={t} badge />
                <span className="lines">
                  <span className="title">{t.title}</span>
                  <span className="sub small">{order ? orderLine(t, order) : `${kindLabel(t)}${t.finished ? ` · ${dayShort(t.finished)}` : ''}`}</span>
                </span>
              </button>
            ))}
          </div>
        </section>
      )}

      {view === 'feed' &&
        groups.map((g) => (
          <section key={g.key}>
            <div className="sec-head">
              <h2 className="sec">{g.title}</h2>
              <span className="small muted">{orderLabel ?? summary(g.rows)}</span>
            </div>
            {g.rows.map((t) => (
              <button key={t.id} type="button" className="feed-row" onClick={() => open(t)}>
                <Cover t={t} size="mini" />
                <span className="lines">
                  <span className="title">{t.title}</span>
                  <span className="sub">{t.status === 'dropped' ? `${subLine(t)} · брошено` : subLine(t)}</span>
                  {t.tags.length > 0 && (
                    <span className="tags">
                      {t.tags.map((x) => (
                        <span key={x} className="tag">
                          {x}
                        </span>
                      ))}
                    </span>
                  )}
                </span>
                <span className="right">
                  {t.rating ? <span className="num rating">{t.rating}</span> : null}
                  <span className="small muted">{t.finished ? dayShort(t.finished) : ''}</span>
                </span>
              </button>
            ))}
          </section>
        ))}

      {wanted.length > 0 && (
        <section>
          <div className="sec-head">
            <h2 className="sec">В «Хочу»</h2>
            <span className="small muted3">{plural(wanted.length, 'запись', 'записи', 'записей')}</span>
          </div>
          {wanted.map((t) => (
            <button key={t.id} type="button" className="feed-row" onClick={() => open(t)}>
              <Cover t={t} size="mini" />
              <span className="lines">
                <span className="title">{t.title}</span>
                <span className="sub">{subLine(t)}</span>
              </span>
              <span className="right">
                <span className="small muted">добавлено {dayShort(t.added)}</span>
              </span>
            </button>
          ))}
        </section>
      )}

      {!empty && finished.length === 0 && now.length === 0 && wanted.length === 0 && narrowed && (
        <div className="stack-s">
          <p className="muted">{q ? `Ничего не нашлось по «${q}»${tag ? ` с меткой «${tag}»` : ''}.` : `Записей с меткой «${tag}» здесь нет.`}</p>
          <div className="key-actions">
            {q && (
              <button type="button" className="btn line small" onClick={() => setQuery('')}>
                Очистить поиск
              </button>
            )}
            {tag && (
              <button type="button" className="btn line small" onClick={() => pick({ order, tag: null })}>
                Снять метку
              </button>
            )}
          </div>
        </div>
      )}
      {!empty && finished.length === 0 && now.length === 0 && !narrowed && <p className="muted">Здесь пока пусто — всё в списке «Хочу».</p>}
    </div>
  );
}
