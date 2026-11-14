import { useState } from 'react';
import type { Filter, Nav } from '../App';
import { api, message, type Title } from '../api';
import { dayShort, plural } from '../format';
import { Cover, Seg } from '../ui';
import { FILTERS, passes, subLine } from './Home';

export default function Want({ titles, nav, filter, setFilter }: { titles: Title[]; nav: Nav; filter: Filter; setFilter: (f: Filter) => void }) {
  const [busy, setBusy] = useState<string | null>(null);
  const all = titles.filter((t) => t.status === 'want').sort((a, b) => b.added.localeCompare(a.added));
  const list = all.filter(passes(filter));
  const books = all.filter((t) => t.kind === 'book').length;
  const screen = all.length - books;

  const start = async (t: Title) => {
    setBusy(t.id);
    try {
      nav.update(await api.setStatus(t.id, 'doing'));
      nav.toast(`«${t.title}» — в процессе`);
    } catch (e) {
      nav.toast(message(e));
    } finally {
      setBusy(null);
    }
  };

  return (
    <div className="stack">
      <div>
        <h1 className="screen-title">Хочу</h1>
        <p className="small muted">
          {all.length === 0
            ? 'Список пуст'
            : [books && plural(books, 'книга', 'книги', 'книг'), screen && plural(screen, 'фильм или сериал', 'фильма или сериала', 'фильмов и сериалов')].filter(Boolean).join(' · ')}
        </p>
      </div>
      <Seg small label="Что показывать" value={filter} options={FILTERS} onChange={setFilter} />
      <div>
        {list.map((t) => (
          <div key={t.id} className="now-row">
            <button type="button" className="now-open" onClick={() => nav.go({ name: 'card', id: t.id })}>
              <Cover t={t} size="mini" />
              <span className="lines">
                <span className="title">{t.title}</span>
                <span className="sub">{subLine(t)}</span>
                <span className="small muted3">добавлено {dayShort(t.added)}</span>
              </span>
            </button>
            <button type="button" className="btn line small" style={{ color: t.kind === 'book' ? 'var(--own)' : 'var(--work)' }} disabled={busy === t.id} onClick={() => start(t)}>
              Начать
            </button>
          </div>
        ))}
        {list.length === 0 && all.length > 0 && <p className="muted">В этом разделе пусто.</p>}
      </div>
    </div>
  );
}
