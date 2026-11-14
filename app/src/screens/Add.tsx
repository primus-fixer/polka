import { useEffect, useMemo, useRef, useState } from 'react';
import type { Nav } from '../App';
import { api, isMobileApp, message, photoToDataUrl, scanIsbn, type BookFormat, type Candidate, type CoverOption, type Found, type Kind, type Status, type Title } from '../api';
import { plural, todayIso } from '../format';
import { Cover, Header, Icon, RatingPicker, Seg, TagPicker, palette } from '../ui';
import { FORMATS, STATUS } from './Card';

const coverLabel = (o: CoverOption) => [o.source, o.edition].filter(Boolean).join(' · ');

/** Название без регистра, «ё» и знаков — чтобы узнать уже добавленное. */
const norm = (s: string) => s.toLowerCase().replace(/ё/g, 'е').replace(/[^\p{L}\p{N}]+/gu, ' ').trim();

export function onShelf(titles: Title[], c: Pick<Candidate, 'title' | 'kind'>): Title | undefined {
  const key = norm(c.title);
  return titles.find((t) => norm(t.title) === key && (t.kind === 'book') === (c.kind === 'book'));
}

export default function Add({ titles, nav, replace }: { titles: Title[]; nav: Nav; replace?: Title }) {
  const [kind, setKind] = useState<'book' | 'film'>(replace && replace.kind !== 'book' ? 'film' : 'book');
  const [query, setQuery] = useState(replace ? [replace.title, replace.kind === 'book' ? replace.creators[0] : ''].filter(Boolean).join(' ') : '');
  const [found, setFound] = useState<Found | null>(null);
  const [busy, setBusy] = useState(false);
  const [mode, setMode] = useState<'search' | 'link'>('search');
  const [url, setUrl] = useState('');
  const [pick, setPick] = useState<Candidate | null>(null);
  const [manual, setManual] = useState(false);

  const search = async (q = query, k = kind) => {
    if (q.trim().length < 2) return;
    setBusy(true);
    setFound(null);
    try {
      setFound(await api.search(k, q));
    } catch (e) {
      nav.toast(message(e));
    } finally {
      setBusy(false);
    }
  };

  // Смена обложки: сразу ищем ту же книгу или фильм.
  useEffect(() => {
    if (replace) search();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const choose = async (c: Candidate) => {
    setBusy(true);
    try {
      setPick(await api.details(c));
    } catch (e) {
      nav.toast(message(e));
      setPick(c);
    } finally {
      setBusy(false);
    }
  };

  const scan = async () => {
    try {
      const code = await scanIsbn();
      setQuery(code);
      await search(code, 'book');
    } catch (e) {
      nav.toast(message(e));
    }
  };

  const byLink = async () => {
    setBusy(true);
    try {
      setPick(await api.fromLink(url));
    } catch (e) {
      nav.toast(message(e));
    } finally {
      setBusy(false);
    }
  };

  const startManual = () => {
    setManual(true);
    setPick({ kind: kind === 'book' ? 'book' : 'film', title: query.trim(), creators: [], genres: [], covers: [], sources: [], seasons: [], refs: [] });
  };

  if (pick)
    return (
      <Pick
        candidate={pick}
        manual={manual}
        titles={titles}
        nav={nav}
        replace={replace}
        onBack={() => {
          setPick(null);
          setManual(false);
        }}
      />
    );

  return (
    <div className="stack">
      <Header title={replace ? 'Другая обложка' : 'Добавить'} onBack={nav.back} backLabel="Закрыть" />
      {!replace && (
        <Seg
          cols
          label="Что добавляем"
          value={kind}
          options={[
            ['book', 'Книга'],
            ['film', 'Фильм или сериал'],
          ]}
          onChange={(k) => {
            setKind(k);
            setFound(null);
          }}
        />
      )}

      {mode === 'search' ? (
        <form
          className="search"
          onSubmit={(e) => {
            e.preventDefault();
            search();
          }}
        >
          <Icon name="search" style={{ color: 'var(--ink2)', flex: 'none' }} />
          <input
            type="search"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder={kind === 'book' ? 'Название, автор или ISBN' : 'Название фильма или сериала'}
            aria-label="Поиск"
            enterKeyHint="search"
            autoFocus={!replace && !isMobileApp}
          />
          <button type="submit" className="btn primary small" disabled={busy || query.trim().length < 2}>
            Найти
          </button>
        </form>
      ) : (
        <form
          className="search"
          onSubmit={(e) => {
            e.preventDefault();
            byLink();
          }}
        >
          <Icon name="link" style={{ color: 'var(--ink2)', flex: 'none' }} />
          <input type="url" value={url} onChange={(e) => setUrl(e.target.value)} placeholder="https://… страница книги или фильма" aria-label="Ссылка на страницу" />
          <button type="submit" className="btn primary small" disabled={busy || !url.trim()}>
            Взять
          </button>
        </form>
      )}

      {!replace && (
        <div className="methods">
          {kind === 'book' && isMobileApp && (
            <button type="button" onClick={scan}>
              <Icon name="barcode" />
              <span>Штрихкод</span>
            </button>
          )}
          <button type="button" onClick={() => setMode(mode === 'link' ? 'search' : 'link')}>
            <Icon name={mode === 'link' ? 'search' : 'link'} />
            <span>{mode === 'link' ? 'Поиск' : 'Ссылка'}</span>
          </button>
          <button type="button" onClick={startManual}>
            <Icon name="pencil" />
            <span>Вручную</span>
          </button>
        </div>
      )}

      {busy && <p className="muted">{mode === 'link' ? 'Открываю страницу…' : 'Ищу в каталогах…'}</p>}

      {found && (
        <section>
          <h2 className="sec">{found.results.length ? 'Нашлось' : 'Ничего не нашлось'}</h2>
          {found.results.map((c, i) => (
            <button key={`${c.title}-${i}`} type="button" className="result-row" onClick={() => choose(c)} disabled={busy}>
              <Cover t={{ title: c.title, kind: c.kind, creators: c.creators, year: c.year ?? undefined }} size="mini" src={c.covers[0]?.url ?? null} />
              <span className="lines">
                <span className="title wrap">{c.title}</span>
                <span className="sub">{[c.creators.slice(0, 2).join(', '), c.year, c.kind === 'series' ? 'сериал' : c.kind === 'film' ? 'фильм' : ''].filter(Boolean).join(' · ')}</span>
                <span className="small muted3">
                  {[c.covers.length ? plural(c.covers.length, 'обложка', 'обложки', 'обложек') : 'без обложки', c.sources.join(', ')].join(' · ')}
                </span>
                {onShelf(titles, c) && <span className="small warn-text">уже на полке</span>}
              </span>
              <Icon name="right" size={18} stroke={2} style={{ color: 'var(--ink3)' }} />
            </button>
          ))}
          {found.failed.length > 0 && <p className="small warn-text">{found.failed.map((f) => `${f.source}: ${f.message}`).join(' · ')}</p>}
          <p className="small muted foot-note">
            {kind === 'book'
              ? 'Ищем сразу в Open Library, Google Books и FantLab. Не нашлось — добавьте по ссылке или вручную.'
              : 'Данные, рейтинги и постеры — из ПоискКино по вашему ключу. Не нашлось — добавьте вручную.'}
          </p>
        </section>
      )}
    </div>
  );
}

type PickProps = { candidate: Candidate; manual: boolean; titles: Title[]; nav: Nav; replace?: Title; onBack: () => void };

function Pick({ candidate, manual, titles, nav, replace, onBack }: PickProps) {
  const [c, setC] = useState(candidate);
  const [broken, setBroken] = useState<string[]>([]);
  const options = c.covers.filter((o) => !broken.includes(o.url));
  const [sel, setSel] = useState<string | 'photo' | null>(candidate.covers[0]?.url ?? null);
  const [photo, setPhoto] = useState<string | null>(null);
  const [status, setStatus] = useState<Status>('done');
  const [date, setDate] = useState(todayIso());
  const [rating, setRating] = useState<number | null>(null);
  const [format, setFormat] = useState<BookFormat>('paper');
  const [tags, setTags] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  const file = useRef<HTMLInputElement>(null);
  const knownTags = useMemo(() => [...new Set(titles.flatMap((x) => x.tags))].sort((a, b) => a.localeCompare(b)), [titles]);
  const isBook = c.kind === 'book';
  const chosen = options.find((o) => o.url === sel) ?? null;
  // Выбранная обложка не загрузилась — берём следующую.
  useEffect(() => {
    if (sel && sel !== 'photo' && !options.some((o) => o.url === sel)) setSel(options[0]?.url ?? null);
  }, [sel, options]);

  const takePhoto = async (f: File | undefined) => {
    if (!f) return;
    try {
      setPhoto(await photoToDataUrl(f));
      setSel('photo');
    } catch (e) {
      nav.toast(message(e));
    }
  };

  const save = async () => {
    if (!c.title.trim()) return nav.toast('Нужно название');
    setBusy(true);
    try {
      if (replace) {
        let t: Title | null = null;
        if (sel === 'photo' && photo) t = await api.setCoverPhoto(replace.id, photo);
        else if (chosen) t = await api.setCoverUrl(replace.id, chosen.url, coverLabel(chosen));
        if (t) nav.update(t);
        nav.toast('Обложка заменена');
        nav.back();
        return;
      }
      let t = await api.add(
        c,
        sel !== 'photo' && chosen ? chosen.url : null,
        sel !== 'photo' && chosen ? coverLabel(chosen) : null,
        { status, date, rating: status === 'done' || status === 'dropped' ? rating : null, format: isBook ? format : null, tags },
      );
      if (sel === 'photo' && photo) t = await api.setCoverPhoto(t.id, photo);
      nav.update(t);
      const lost = sel !== 'photo' && chosen && !t.cover;
      nav.toast(lost ? `«${t.title}» — на полке, но обложка не скачалась. Выберите другую в карточке` : `«${t.title}» — на полке`);
      nav.go({ name: 'card', id: t.id });
    } catch (e) {
      nav.toast(message(e));
    } finally {
      setBusy(false);
    }
  };

  const [bg, fg] = palette(c.title || 'Полка');
  const dateLabel = status === 'want' ? 'Добавлено' : status === 'doing' ? 'Начато' : status === 'dropped' ? 'Брошено' : isBook ? 'Прочитано' : 'Просмотрено';

  return (
    <div className="stack pick">
      <Header title={replace ? 'Выбор обложки' : c.title || 'Новая запись'} onBack={onBack} backLabel="Назад к поиску" />

      {!replace && onShelf(titles, c) && <p className="small warn-text">Такая запись уже есть на полке — сохранится ещё одна.</p>}

      {manual ? (
        <div className="edit-box">
          <Seg
            cols
            small
            label="Что это"
            value={c.kind}
            options={[
              ['book', 'Книга'],
              ['film', 'Фильм'],
              ['series', 'Сериал'],
            ]}
            onChange={(k: Kind) => setC({ ...c, kind: k })}
          />
          <label className="field">
            <span>Название</span>
            <input value={c.title} onChange={(e) => setC({ ...c, title: e.target.value })} maxLength={300} required />
          </label>
          <label className="field">
            <span>{isBook ? 'Авторы, через запятую' : c.kind === 'film' ? 'Режиссёр' : 'Создатели'}</span>
            <input value={c.creators.join(', ')} onChange={(e) => setC({ ...c, creators: e.target.value.split(',').map((x) => x.trimStart()) })} />
          </label>
          <label className="field">
            <span>Год</span>
            <input value={c.year ?? ''} onChange={(e) => setC({ ...c, year: e.target.value })} inputMode="numeric" maxLength={9} />
          </label>
        </div>
      ) : (
        <p className="small muted">{[c.creators.join(', '), c.year, c.sources.join(', ')].filter(Boolean).join(' · ')}</p>
      )}

      <section className="stack-s">
        <div className="sec-head">
          <h2 className="sec">{c.kind === 'book' ? 'Обложка' : 'Постер'}</h2>
          <span className="small muted">{options.length ? plural(options.length, 'вариант', 'варианта', 'вариантов') : 'вариантов нет'}</span>
        </div>
        <div className="strip">
          {options.map((o) => (
            <button key={o.url} type="button" className={`pick-tile${sel === o.url ? ' sel' : ''}`} aria-pressed={sel === o.url} aria-label={`Обложка: ${coverLabel(o)}`} onClick={() => setSel(o.url)}>
              <span className="cover pick">
                <img src={o.url} alt="" referrerPolicy="no-referrer" onError={() => setBroken((b) => [...b, o.url])} />
                {sel === o.url && (
                  <span className="tick">
                    <Icon name="check" size={14} stroke={2.5} />
                  </span>
                )}
              </span>
              <span className="cap">{o.edition || o.source}</span>
              <span className="cap muted3">{o.edition ? o.source : ''}</span>
            </button>
          ))}
          {photo && (
            <button type="button" className={`pick-tile${sel === 'photo' ? ' sel' : ''}`} aria-pressed={sel === 'photo'} onClick={() => setSel('photo')}>
              <span className="cover pick">
                <img src={photo} alt="" />
              </span>
              <span className="cap">Своё фото</span>
            </button>
          )}
          {!options.length && !photo && (
            <span className="cover pick ph" style={{ background: bg, color: fg }}>
              <span className="ph-line" />
              <span className="ph-title">{c.title || 'Без названия'}</span>
              <span className="ph-by">обложка из названия</span>
            </span>
          )}
        </div>
        <div className="two">
          <button type="button" className="btn soft" onClick={() => file.current?.click()}>
            <Icon name="camera" />
            <span>Своё фото</span>
          </button>
          <button type="button" className="btn soft" onClick={() => setSel(null)} aria-pressed={sel === null}>
            <span>Без обложки</span>
          </button>
          <input ref={file} type="file" accept="image/*" capture={isMobileApp ? 'environment' : undefined} hidden onChange={(e) => takePhoto(e.target.files?.[0])} />
        </div>
      </section>

      {!replace && (
        <>
          <section className="stack-s">
            <h2 className="label">Статус</h2>
            <Seg cols small label="Статус" value={status} options={STATUS} onChange={setStatus} />
            <label className="field row-field">
              <span>{dateLabel}</span>
              <input type="date" value={date} max={todayIso()} onChange={(e) => setDate(e.target.value || todayIso())} />
            </label>
          </section>

          {(status === 'done' || status === 'dropped') && (
            <section className="stack-s">
              <div className="sec-head">
                <h2 className="label">Оценка</h2>
                <span className="small muted3">от 1 до 10</span>
              </div>
              <RatingPicker value={rating} onChange={setRating} />
            </section>
          )}

          {isBook && (
            <section className="stack-s">
              <h2 className="label">Формат</h2>
              <Seg cols small label="Формат" value={format} options={FORMATS} onChange={setFormat} />
            </section>
          )}

          <section className="stack-s">
            <h2 className="label">Метки</h2>
            <TagPicker value={tags} known={knownTags} onChange={setTags} />
          </section>
        </>
      )}

      <div className="save-bar">
        <button type="button" className="btn primary big" disabled={busy} onClick={save}>
          {busy ? 'Сохраняю…' : replace ? 'Сохранить обложку' : 'Сохранить на полку'}
        </button>
      </div>
    </div>
  );
}
