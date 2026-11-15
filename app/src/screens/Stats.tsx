import { useEffect, useState } from 'react';
import type { Library, Nav } from '../App';
import { api, message, type Title, type YearStats } from '../api';
import { daysLeftInYear, decimal, MONTH_LETTERS, plural, thousands, todayIso, yearOf } from '../format';
import { Bar, Cover, Icon } from '../ui';
import { bookHours, comparisons } from '../fun';
import { kindLabel } from './Home';
import YearPicture from './YearPicture';

export default function Stats({ titles, nav, library }: { titles: Title[]; nav: Nav; library: Library }) {
  const thisYear = yearOf(todayIso());
  const [year, setYear] = useState(thisYear);
  const [s, setS] = useState<YearStats | null>(null);
  const [picture, setPicture] = useState(false);
  const firstYear = Math.min(thisYear, ...titles.map((t) => yearOf(t.finished ?? t.added)));

  useEffect(() => {
    api.stats(year).then(setS).catch((e) => nav.toast(message(e)));
  }, [year, titles, nav]);

  if (!s) return <p className="muted">Считаю итоги…</p>;

  const goal = s.goal.books ?? null;
  const left = s.books_left ?? 0;
  const days = year === thisYear ? daysLeftInYear(todayIso()) : 0;
  const pace = goal && left > 0 && days > 0 ? `Осталось ${plural(left, 'книга', 'книги', 'книг')} — примерно по одной за ${plural(Math.max(1, Math.round(days / left)), 'день', 'дня', 'дней')}` : goal && left === 0 ? 'Цель на год выполнена' : '';
  const max = Math.max(1, ...s.months.map((m) => Math.max(m.books, m.screen)));
  const screenHours = Math.round(s.screen_minutes / 60);
  const figures: [string, string][] = [
    ['Страниц', thousands(s.pages)],
    ['Фильмов', String(s.films)],
    ['Сериалов', String(s.series)],
    ['Серий', String(s.episodes)],
    ['Часов у экрана', String(screenHours)],
    ['Средняя оценка', s.avg_rating ? decimal(s.avg_rating) : '—'],
  ];
  if (s.audio_minutes) figures.splice(1, 0, ['Часов аудиокниг', String(Math.round(s.audio_minutes / 60))]);
  // Время в читалке «Полки» за этот год — со всех устройств.
  const readMinutes = library.readings.reduce((a, r) => a + (r.years?.[String(year)] ?? 0), 0);
  if (readMinutes > 0) figures.splice(1, 0, ['Часов в читалке', readMinutes < 600 ? decimal(Math.round(readMinutes / 6) / 10) : String(Math.round(readMinutes / 60))]);
  if (s.dropped) figures.push(['Брошено', String(s.dropped)]);
  const best = s.best.map((id) => titles.find((t) => t.id === id)).filter((t): t is Title => !!t);
  // Забавные сравнения: часы за книгами и у экрана — в понятных мерах.
  const fun = comparisons(bookHours(s.pages, s.audio_minutes, readMinutes), s.screen_minutes / 60, year);
  const genreMax = Math.max(1, ...s.genres.map((g) => g.n));

  return (
    <div className="stack">
      <div className="sec-head">
        <h1 className="screen-title">Итоги</h1>
        <div className="year-nav">
          <button type="button" className="icon-btn" aria-label="Предыдущий год" disabled={year <= firstYear} onClick={() => setYear(year - 1)}>
            <Icon name="left" stroke={2} />
          </button>
          <span className="num">{year}</span>
          <button type="button" className="icon-btn" aria-label="Следующий год" disabled={year >= thisYear} onClick={() => setYear(year + 1)}>
            <Icon name="right" stroke={2} />
          </button>
        </div>
      </div>

      <section className="stack-s">
        <h2 className="label big">Книги за {year} год</h2>
        <p className="hero-num num">
          {s.books}
          {goal ? <span className="hero-num-small"> из {goal}</span> : null}
        </p>
        {goal ? <Bar pct={(s.books / goal) * 100} thick /> : null}
        {pace ? <p className="small muted">{pace}</p> : !goal ? <p className="small muted">Цель на год можно задать в настройках.</p> : null}
      </section>

      <button type="button" className="btn line year-pic-btn" disabled={s.books + s.films + s.series === 0} onClick={() => setPicture(true)}>
        <Icon name="image" size={20} />
        <span>{s.books + s.films + s.series === 0 ? `Картинка ${year} года — когда что-нибудь закончите` : `Картинка ${year} года`}</span>
      </button>
      {picture && <YearPicture year={year} stats={s} titles={titles} nav={nav} onClose={() => setPicture(false)} />}

      <div className="figures">
        {figures.map(([label, value]) => (
          <div key={label}>
            <span className="small muted">{label}</span>
            <span className="num figure">{value}</span>
          </div>
        ))}
      </div>

      {fun.length > 0 && (
        <section className="stack-s fun">
          <h2 className="sec">Если сравнить</h2>
          <ul>
            {fun.map((line) => (
              <li key={line}>{line}</li>
            ))}
          </ul>
          <p className="small muted3">Часы за книгами — по полторы минуты на страницу; у экрана — по длительности фильмов и серий.</p>
        </section>
      )}

      <section>
        <div className="sec-head">
          <h2 className="sec">По месяцам</h2>
          <span className="legend small muted">
            <span>
              <i style={{ background: 'var(--serOwn)' }} />
              книги
            </span>
            <span>
              <i style={{ background: 'var(--serWork)' }} />
              кино и сериалы
            </span>
          </span>
        </div>
        <div className="chart" role="img" aria-label={`Книги и кино по месяцам ${year} года`}>
          {s.months.map((m, i) => (
            <div key={i} className="chart-col" title={`${MONTH_LETTERS[i]}: книг ${m.books}, кино ${m.screen}`}>
              <span style={{ height: `${(m.books / max) * 100}%`, background: 'var(--serOwn)' }} />
              <span style={{ height: `${(m.screen / max) * 100}%`, background: 'var(--serWork)' }} />
            </div>
          ))}
        </div>
        <div className="chart-labels" aria-hidden="true">
          {MONTH_LETTERS.map((l, i) => (
            <span key={i}>{l}</span>
          ))}
        </div>
      </section>

      {s.genres.length > 0 && (
        <section>
          <h2 className="sec">Жанры книг</h2>
          {s.genres.map((g) => (
            <div key={g.name} className="genre-row">
              <span>{g.name}</span>
              <Bar pct={(g.n / genreMax) * 100} color="var(--serOwn)" />
              <span className="num">{g.n}</span>
            </div>
          ))}
        </section>
      )}

      {s.authors.length > 0 && (
        <section>
          <h2 className="sec">Чаще всего</h2>
          {s.authors.map((a) => (
            <div key={a.name} className="row">
              <span>{a.name}</span>
              <span className="num muted">{a.n}</span>
            </div>
          ))}
        </section>
      )}

      {best.length > 0 && (
        <section>
          <h2 className="sec">Лучшее за год</h2>
          <div className="best">
            {best.map((t) => (
              <button key={t.id} type="button" className="now-open" onClick={() => nav.go({ name: 'card', id: t.id })}>
                <Cover t={t} size="mini" />
                <span className="lines">
                  <span className="title">{t.title}</span>
                  <span className="sub">
                    {kindLabel(t)} · {t.rating} из 10
                  </span>
                </span>
              </button>
            ))}
          </div>
        </section>
      )}
    </div>
  );
}
