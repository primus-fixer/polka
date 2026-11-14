import { useState } from 'react';
import type { Filter, Nav } from '../App';
import { api, message, type Goal, type Title } from '../api';
import { dayShort, monthTitle, plural } from '../format';
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
};

export default function Home({ titles, nav, wide, view, setView, filter, setFilter, goal, year }: Props) {
  const [busy, setBusy] = useState<string | null>(null);
  const pass = passes(filter);
  const now = titles.filter((t) => t.status === 'doing' && pass(t)).sort((a, b) => (b.started ?? '').localeCompare(a.started ?? ''));
  const finished = titles
    .filter((t) => (t.status === 'done' || (view === 'feed' && t.status === 'dropped')) && pass(t))
    .sort((a, b) => (b.finished ?? b.added).localeCompare(a.finished ?? a.added));
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
  for (const t of finished) {
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

  return (
    <div className="stack">
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

      {view === 'shelf' && goal.books && filter !== 'screen' ? (
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
            <span className="small muted3">сначала новые</span>
          </div>
          <div className="shelf" style={{ gridTemplateColumns: `repeat(${wide ? 6 : 3}, minmax(0, 1fr))` }}>
            {finished.map((t) => (
              <button key={t.id} type="button" className="shelf-item" onClick={() => open(t)}>
                <Cover t={t} badge />
                <span className="lines">
                  <span className="title">{t.title}</span>
                  <span className="sub small">
                    {kindLabel(t)}
                    {t.finished ? ` · ${dayShort(t.finished)}` : ''}
                  </span>
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
              <span className="small muted">{summary(g.rows)}</span>
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

      {!empty && finished.length === 0 && now.length === 0 && <p className="muted">Здесь пока пусто — всё в списке «Хочу».</p>}
    </div>
  );
}
