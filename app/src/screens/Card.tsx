import { useEffect, useMemo, useRef, useState } from 'react';
import type { Library, Nav } from '../App';
import { api, isMobileApp, message, photoToDataUrl, type BookFormat, type Mark, type Status, type Title } from '../api';
import { dayMonth, decimal, plural, span } from '../format';
import { Bar, Cover, Header, Icon, RatingPicker, Row, Seg, TagPicker } from '../ui';
import CardPicture from './CardPicture';
import QuotePicture from './QuotePicture';
import { addReadingFonts } from '../reader/fonts';
import { kindLabel, seriesLine } from './Home';

export const STATUS: [Status, string][] = [
  ['want', 'Хочу'],
  ['doing', 'В процессе'],
  ['done', 'Готово'],
  ['dropped', 'Брошено'],
];

export const FORMATS: [BookFormat, string][] = [
  ['paper', 'Бумажная'],
  ['ebook', 'Электронная'],
  ['audio', 'Аудио'],
];

function byLine(t: Title): string {
  if (t.kind === 'book') return t.creators.join(', ');
  const who = t.creators.length ? (t.kind === 'film' ? `реж. ${t.creators.join(', ')}` : t.creators.join(', ')) : '';
  return [t.year, who].filter(Boolean).join(' · ');
}

function metaLine(t: Title): string {
  const parts: string[] = [];
  if (t.kind === 'book') {
    if (t.year) parts.push(t.year);
    if (t.format !== 'audio' && t.pages) parts.push(plural(t.pages, 'страница', 'страницы', 'страниц'));
    if (t.format === 'audio' && t.minutes) parts.push(`${Math.round(t.minutes / 6) / 10} ч`.replace('.', ','));
  } else if (t.kind === 'series') {
    const eps = t.seasons.reduce((a, s) => a + s.episodes, 0);
    if (t.seasons.length) parts.push(`${plural(t.seasons.length, 'сезон', 'сезона', 'сезонов')}, ${plural(eps, 'серия', 'серии', 'серий')}`);
  } else if (t.minutes) {
    parts.push(`${t.minutes} мин`);
  }
  if (t.genres.length) parts.push(t.genres.slice(0, 3).join(', '));
  return parts.join(' · ');
}

/** Какие файлы предлагать в окне выбора. На Android формат FB2 системе не знаком — там без отбора. */
export const BOOK_ACCEPT = isMobileApp ? undefined : '.epub,.fb2,.zip,.pdf,application/epub+zip,application/x-fictionbook+xml,application/pdf';

/** «45 %», «4,5 %». */
export function readPct(p: number): string {
  return p > 0 && p < 10 ? `${(Math.round(p * 10) / 10).toString().replace('.', ',')} %` : `${Math.round(p)} %`;
}

function readTime(minutes: number): string {
  if (minutes < 60) return plural(Math.max(1, minutes), 'минута', 'минуты', 'минут');
  const h = Math.round(minutes / 6) / 10;
  return `${h.toString().replace('.', ',')} ч`;
}

/** День из секунд Unix по местному времени: «2026-11-14». */
function isoDay(secs: number): string {
  const d = new Date(secs * 1000);
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`;
}

function fileSize(bytes: number): string {
  return bytes >= 1 << 20 ? `${(Math.round(bytes / 104857.6) / 10).toString().replace('.', ',')} МБ` : `${Math.max(1, Math.round(bytes / 1024))} КБ`;
}

export default function Card({ t, titles, nav, wide, library, onForget }: { t: Title; titles: Title[]; nav: Nav; wide: boolean; library: Library; onForget: () => void }) {
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState(t.note ?? '');
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [openSeason, setOpenSeason] = useState<number | null>(() => seriesLine(t).next?.[0] ?? null);
  const [edit, setEdit] = useState(false);
  const [picture, setPicture] = useState(false);
  const [draft, setDraft] = useState({ title: t.title, creators: t.creators.join(', '), year: t.year ?? '' });
  const [newSeason, setNewSeason] = useState('');
  const file = useRef<HTMLInputElement>(null);
  const bookInput = useRef<HTMLInputElement>(null);
  const bookFile = t.kind === 'book' ? library.files.find((f) => f.title === t.id) : undefined;
  // Файла здесь нет, но он есть на другом устройстве.
  const remote = t.kind === 'book' && !bookFile ? library.elsewhere.find((f) => f.title === t.id) : undefined;
  const reading = library.readings.find((r) => r.title === t.id);
  const readable = t.kind === 'book' && t.format !== 'audio';
  const [quotes, setQuotes] = useState<Mark[]>([]);
  const [quotePic, setQuotePic] = useState<Mark | null>(null);
  const [confirmQuote, setConfirmQuote] = useState<string | null>(null);
  useEffect(() => {
    if (t.kind !== 'book') return;
    api
      .marks(t.id)
      .then((list) => {
        const found = list.filter((m) => m.kind === 'quote');
        // Цитаты — шрифтом читалки.
        if (found.length) addReadingFonts();
        setQuotes(found);
      })
      .catch(() => undefined);
  }, [t.id, t.kind]);
  const removeQuote = async (m: Mark) => {
    if (confirmQuote !== m.id) return setConfirmQuote(m.id);
    setConfirmQuote(null);
    try {
      await api.markDelete(m.id);
      setQuotes((l) => l.filter((x) => x.id !== m.id));
    } catch (e) {
      nav.toast(message(e));
    }
  };
  const knownTags = useMemo(() => [...new Set(titles.flatMap((x) => x.tags))].sort((a, b) => a.localeCompare(b)), [titles]);
  const series = t.kind === 'series' ? seriesLine(t) : null;

  const run = async (p: () => Promise<Title>, done?: string) => {
    setBusy(true);
    try {
      nav.update(await p());
      if (done) nav.toast(done);
    } catch (e) {
      nav.toast(message(e));
    } finally {
      setBusy(false);
    }
  };
  const save = (patch: Partial<Title>, done?: string) => run(() => api.save({ ...t, ...patch }), done);

  const photo = async (f: File | undefined) => {
    if (!f) return;
    try {
      const dataUrl = await photoToDataUrl(f);
      await run(() => api.setCoverPhoto(t.id, dataUrl), 'Обложка заменена своим фото');
    } catch (e) {
      nav.toast(message(e));
    }
  };

  const remove = async () => {
    try {
      await api.remove(t.id);
      nav.removed(t.id);
      nav.toast(`«${t.title}» удалено`);
      nav.back();
    } catch (e) {
      nav.toast(message(e));
    }
  };

  const total = t.seasons.reduce((a, s) => a + s.episodes, 0);
  const seen = t.seasons.reduce((a, s) => a + s.seen.length, 0);
  const finishedLabel = t.status === 'dropped' ? 'Брошено' : t.kind === 'book' ? 'Прочитано' : 'Просмотрено';

  return (
    <div className="stack">
      <Header
        onBack={nav.back}
        right={
          <span className="topbar-tools">
            <button type="button" className="icon-btn" aria-label="Поделиться карточкой" onClick={() => setPicture(true)}>
              <Icon name="share" />
            </button>
            <button type="button" className="icon-btn" aria-label="Изменить название и автора" aria-expanded={edit} onClick={() => setEdit(!edit)}>
              <Icon name="pencil" />
            </button>
          </span>
        }
      />
      {picture && <CardPicture t={t} nav={nav} onClose={() => setPicture(false)} />}

      <div className="hero" style={{ gridTemplateColumns: wide ? '200px minmax(0, 1fr)' : '120px minmax(0, 1fr)' }}>
        <Cover t={t} size="hero" />
        <div className="hero-info">
          <span className={`kind-chip ${t.kind === 'book' ? 'own' : 'work'}`}>{kindLabel(t)}</span>
          <h1 className="card-title">{t.title}</h1>
          {t.original && <p className="small muted">{t.original}</p>}
          {byLine(t) && <p className="muted">{byLine(t)}</p>}
          {metaLine(t) && <p className="small muted">{metaLine(t)}</p>}
          {(t.kp || t.imdb) && (
            <p className="num ratings-line">
              {[t.kp && `КП ${decimal(t.kp)}`, t.imdb && `IMDb ${decimal(t.imdb)}`].filter(Boolean).join(' · ')}
              {t.ratings_on && <span className="small muted3">рейтинги на {dayMonth(t.ratings_on)}</span>}
            </p>
          )}
          <div className="hero-actions">
            <button type="button" className="link-btn" onClick={() => nav.go({ name: 'add', replace: t.id })}>
              Сменить обложку
            </button>
            <button type="button" className="link-btn" onClick={() => file.current?.click()}>
              Своё фото
            </button>
            <input ref={file} type="file" accept="image/*" hidden onChange={(e) => photo(e.target.files?.[0])} />
          </div>
        </div>
      </div>

      {readable && (
        <section className="read-box">
          {bookFile ? (
            <button type="button" className="btn primary big" onClick={() => nav.read(bookFile.hash)}>
              <Icon name="read" stroke={2} />
              <span>{reading && reading.percent >= 0.5 && reading.percent < 99.5 ? `Продолжить · ${readPct(reading.percent)}` : 'Читать'}</span>
            </button>
          ) : (
            <button type="button" className="btn line big" onClick={() => bookInput.current?.click()}>
              <Icon name="file" />
              <span>Открыть файл книги</span>
            </button>
          )}
          {reading && reading.percent >= 0.5 && (
            <div className="read-progress">
              <Bar pct={reading.percent} />
              <span className="small muted">
                Прочитано {readPct(reading.percent)}
                {reading.minutes > 0 ? ` · ${readTime(reading.minutes)} за чтением` : ''}
                {!bookFile && !remote ? ' · файл — на другом устройстве' : ''}
              </span>
            </div>
          )}
          {remote && (
            <p className="small muted">
              Файл «{remote.name || remote.format.toUpperCase()}» ({fileSize(remote.size)}) — на другом устройстве.{' '}
              {remote.skipped ? (
                <>
                  Он убран с этого устройства.{' '}
                  <button
                    type="button"
                    className="link-btn small"
                    onClick={async () => {
                      try {
                        await api.bookUnskip(remote.hash);
                        onForget();
                        nav.toast(library.filesSync ? 'Файл придёт при следующем обмене' : 'Файл придёт, когда включите «Файлы книг на всех устройствах» в настройках');
                      } catch (e) {
                        nav.toast(message(e));
                      }
                    }}
                  >
                    Вернуть
                  </button>
                </>
              ) : library.filesSync ? (
                'Придёт сюда при следующем обмене по Wi-Fi или через облако.'
              ) : (
                'Чтобы он пришёл сюда, включите «Файлы книг на всех устройствах» в настройках — здесь и там, где файл.'
              )}
            </p>
          )}
          {!bookFile && !remote && <p className="small muted">EPUB, FB2 или PDF — книга откроется в читалке «Полки», место чтения перейдёт на другие устройства.</p>}
          <input
            ref={bookInput}
            type="file"
            accept={BOOK_ACCEPT}
            hidden
            onChange={(e) => {
              const f = e.target.files?.[0];
              e.target.value = '';
              if (f) nav.importBook(f, t.id);
            }}
          />
        </section>
      )}

      {edit && (
        <form
          className="edit-box"
          onSubmit={(e) => {
            e.preventDefault();
            const creators = draft.creators.split(',').map((x) => x.trim()).filter(Boolean);
            save({ title: draft.title, creators, year: draft.year.trim() || undefined }, 'Сохранено').then(() => setEdit(false));
          }}
        >
          <label className="field">
            <span>Название</span>
            <input value={draft.title} onChange={(e) => setDraft({ ...draft, title: e.target.value })} required maxLength={300} />
          </label>
          <label className="field">
            <span>{t.kind === 'book' ? 'Авторы, через запятую' : t.kind === 'film' ? 'Режиссёр' : 'Создатели'}</span>
            <input value={draft.creators} onChange={(e) => setDraft({ ...draft, creators: e.target.value })} />
          </label>
          <label className="field">
            <span>Год</span>
            <input value={draft.year} onChange={(e) => setDraft({ ...draft, year: e.target.value })} inputMode="numeric" maxLength={9} />
          </label>
          <button type="submit" className="btn primary" disabled={busy}>
            Сохранить
          </button>
        </form>
      )}

      {t.about && <p className="muted">{t.about}</p>}

      <section className="stack-s">
        <h2 className="label">Статус</h2>
        <Seg cols small label="Статус" value={t.status} options={STATUS} onChange={(s) => run(() => api.setStatus(t.id, s))} />
        {t.status !== 'want' && (
          <div className="dates">
            <label className="field">
              <span>Начато</span>
              <input type="date" value={t.started ?? ''} onChange={(e) => save({ started: e.target.value || undefined })} />
            </label>
            {(t.status === 'done' || t.status === 'dropped') && (
              <label className="field">
                <span>{finishedLabel}</span>
                <input type="date" value={t.finished ?? ''} onChange={(e) => save({ finished: e.target.value || undefined })} />
              </label>
            )}
          </div>
        )}
      </section>

      {series && (
        <section className="stack-s">
          <div className="goal-text">
            <span>Просмотрено</span>
            <span className="num ink">
              {seen} из {plural(total, 'серии', 'серий', 'серий')}
            </span>
          </div>
          <Bar pct={series.pct} color="var(--work)" thick />
          {series.next && (
            <button type="button" className="btn work" disabled={busy} onClick={() => run(() => api.markEpisode(t.id, series.next![0], series.next![1], true))}>
              <Icon name="check" stroke={2.25} />
              <span>
                Отметить серию {series.next[1]} сезона {series.next[0]}
              </span>
            </button>
          )}
          <div>
            {[...t.seasons]
              .sort((a, b) => a.no - b.no)
              .map((s) => {
                const open = openSeason === s.no;
                const full = s.seen.length === s.episodes;
                return (
                  <div key={s.no} className="season">
                    <button type="button" className="season-head" aria-expanded={open} onClick={() => setOpenSeason(open ? null : s.no)}>
                      <span className="lines">
                        <span className="title">Сезон {s.no}</span>
                        <span className="sub">{plural(s.episodes, 'серия', 'серии', 'серий')}</span>
                      </span>
                      <span className="season-state" style={{ color: full ? 'var(--pos)' : s.seen.length ? 'var(--work)' : 'var(--ink3)' }}>
                        {full ? 'просмотрен' : s.seen.length ? `${s.seen.length} из ${s.episodes}` : 'не начат'}
                        <Icon name={open ? 'up' : 'down'} size={18} stroke={2} style={{ color: 'var(--ink2)' }} />
                      </span>
                    </button>
                    {open && (
                      <div className="season-body">
                        <div className="eps">
                          {Array.from({ length: s.episodes }, (_, i) => i + 1).map((ep) => {
                            const w = s.seen.find((x) => x.ep === ep);
                            const next = series.next?.[0] === s.no && series.next?.[1] === ep;
                            return (
                              <button
                                key={ep}
                                type="button"
                                className={`ep num${w ? ' seen' : ''}${next ? ' next' : ''}`}
                                aria-pressed={!!w}
                                aria-label={`Серия ${ep}${w ? `, просмотрена ${dayMonth(w.on)}` : next ? ', следующая' : ''}`}
                                disabled={busy}
                                onClick={() => run(() => api.markEpisode(t.id, s.no, ep, !w))}
                              >
                                {ep}
                              </button>
                            );
                          })}
                        </div>
                        <button type="button" className="link-btn" disabled={busy} onClick={() => run(() => api.markSeason(t.id, s.no, !full))}>
                          {full ? 'Снять отметки сезона' : 'Отметить весь сезон'}
                        </button>
                      </div>
                    )}
                  </div>
                );
              })}
          </div>
          <form
            className="add-season"
            onSubmit={(e) => {
              e.preventDefault();
              const n = Number(newSeason);
              if (!Number.isInteger(n) || n < 1 || n > 500) return nav.toast('Число серий — от 1 до 500');
              const no = Math.max(0, ...t.seasons.map((s) => s.no)) + 1;
              save({ seasons: [...t.seasons, { no, episodes: n, seen: [] }] }, `Добавлен сезон ${no}`).then(() => setNewSeason(''));
            }}
          >
            <input value={newSeason} onChange={(e) => setNewSeason(e.target.value)} inputMode="numeric" placeholder="Серий в новом сезоне" aria-label="Серий в новом сезоне" />
            <button type="submit" className="btn line small" disabled={busy || !newSeason}>
              Добавить сезон
            </button>
          </form>
        </section>
      )}

      {(t.status === 'done' || t.status === 'dropped') && (
        <section className="stack-s">
          <div className="sec-head">
            <h2 className="label">Оценка</h2>
            <span className="small muted3">от 1 до 10</span>
          </div>
          <RatingPicker value={t.rating} onChange={(r) => save({ rating: r ?? undefined })} />
        </section>
      )}

      {t.kind === 'book' && (
        <section className="stack-s">
          <h2 className="label">Формат</h2>
          <Seg cols small label="Формат" value={t.format ?? 'paper'} options={FORMATS} onChange={(format) => save({ format })} />
        </section>
      )}

      <section className="stack-s">
        <h2 className="label">Метки</h2>
        <TagPicker value={t.tags} known={knownTags} onChange={(tags) => save({ tags })} />
      </section>

      <section className="stack-s">
        <h2 className="label">Заметка</h2>
        <textarea
          className="note"
          value={note}
          placeholder="Пара строк для себя"
          rows={4}
          maxLength={20000}
          onChange={(e) => setNote(e.target.value)}
          onBlur={() => note !== (t.note ?? '') && save({ note })}
        />
      </section>

      {quotes.length > 0 && (
        <section className="stack-s">
          <div className="sec-head">
            <h2 className="label">Цитаты</h2>
            <span className="small muted3 num">{quotes.length}</span>
          </div>
          <ul className="quotes">
            {quotes.map((m) => (
              <li key={m.id} className="quote-row">
                <p className="quote-text">«{m.text}»</p>
                <div className="quote-meta">
                  <span className="small muted">
                    {readPct(m.percent)} · {dayMonth(isoDay(m.created))}
                  </span>
                  <span className="topbar-tools">
                    {bookFile?.hash === m.book && (
                      <button type="button" className="icon-btn" aria-label="Открыть в книге" onClick={() => nav.read(m.book, { chapter: m.chapter, offset: m.offset, len: m.end - m.offset })}>
                        <Icon name="read" size={18} />
                      </button>
                    )}
                    <button type="button" className="icon-btn" aria-label="Цитата картинкой" onClick={() => setQuotePic(m)}>
                      <Icon name="share" size={18} />
                    </button>
                    <button type="button" className={confirmQuote === m.id ? 'link-btn warn-text small' : 'icon-btn'} aria-label="Удалить цитату" onClick={() => removeQuote(m)}>
                      {confirmQuote === m.id ? 'Удалить?' : <Icon name="trash" size={18} />}
                    </button>
                  </span>
                </div>
              </li>
            ))}
          </ul>
        </section>
      )}
      {quotePic && <QuotePicture t={t} mark={quotePic} nav={nav} onClose={() => setQuotePic(null)} />}

      <div>
        {t.kind === 'book' && t.format !== 'audio' && (
          <Row label="Страниц">
            <input
              className="inline-num num"
              inputMode="numeric"
              defaultValue={t.pages ?? ''}
              aria-label="Страниц"
              onBlur={(e) => {
                const v = Number(e.target.value);
                if (String(v) !== String(t.pages ?? '')) save({ pages: v > 0 ? Math.round(v) : undefined });
              }}
            />
          </Row>
        )}
        {(t.kind !== 'book' || t.format === 'audio') && (
          <Row label={t.kind === 'series' ? 'Минут в серии' : 'Длительность, мин'}>
            <input
              className="inline-num num"
              inputMode="numeric"
              defaultValue={t.minutes ?? ''}
              aria-label="Минут"
              onBlur={(e) => {
                const v = Number(e.target.value);
                if (String(v) !== String(t.minutes ?? '')) save({ minutes: v > 0 ? Math.round(v) : undefined });
              }}
            />
          </Row>
        )}
        {t.status !== 'want' && <Row label="Период">{span(t.started, t.finished) || '—'}</Row>}
        <Row label="Добавлено">{dayMonth(t.added)}</Row>
        {t.publisher && <Row label="Издательство">{t.publisher}</Row>}
        {t.isbn && <Row label="ISBN">{t.isbn}</Row>}
        {t.cover_source && <Row label="Обложка">{t.cover_source}</Row>}
        {bookFile && (
          <Row label="Файл книги">
            <span className="file-row">
              <span className="small">
                {bookFile.name || bookFile.format.toUpperCase()} · {fileSize(bookFile.size)}
              </span>
              <button
                type="button"
                className="link-btn small"
                onClick={async () => {
                  try {
                    await api.bookForget(bookFile.hash);
                    onForget();
                    nav.toast('Файл книги убран с этого устройства (на других он остался); место чтения сохранилось');
                  } catch (e) {
                    nav.toast(message(e));
                  }
                }}
              >
                Убрать с устройства
              </button>
              <button type="button" className="link-btn small" onClick={() => bookInput.current?.click()}>
                Другой файл
              </button>
            </span>
          </Row>
        )}
      </div>

      <div className="danger">
        {confirmDelete ? (
          <>
            <button type="button" className="btn warn" onClick={remove}>
              Точно удалить
            </button>
            <button type="button" className="btn line" onClick={() => setConfirmDelete(false)}>
              Оставить
            </button>
          </>
        ) : (
          <button type="button" className="link-btn warn-text" onClick={() => setConfirmDelete(true)}>
            <Icon name="trash" size={18} />
            <span>Удалить запись</span>
          </button>
        )}
      </div>
    </div>
  );
}
