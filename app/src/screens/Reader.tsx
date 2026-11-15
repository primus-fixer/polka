// Читалка: глава колонками, листание касанием, свайпом, колесом и клавишами; оглавление,
// поиск, сноски, вид страницы (размер, шрифт, интервал, поля, тема) и место чтения.
import { useCallback, useEffect, useMemo, useRef, useState, type CSSProperties, type FormEvent, type MouseEvent as ReactMouseEvent, type TouchEvent as ReactTouchEvent, type WheelEvent as ReactWheelEvent } from 'react';
import type { Nav, ReaderAt } from '../App';
import { api, bookImageSrc, message, type BookHit, type BookNode, type BookView, type Mark, type Reading, type Title } from '../api';
import { dayMonth, plural } from '../format';
import { addReadingFonts, readingFontsReady } from '../reader/fonts';
import { Pager } from '../reader/pager';
import { build, findNote } from '../reader/render';
import { FONTS, LEADINGS, MARGINS, SIZE_MAX, SIZE_MIN, THEMES, readSettings, readSpeed, resolveTheme, saveSettings, saveSpeed, type ReaderFont, type ReaderSettings } from '../reader/settings';
import { store, stored } from '../storage';
import { MarkList, Sheet, THEME_ORDER, highlights, pct, timeLeft, useAndroidBack, useWakeLock } from '../reader/ui';
import { Icon, RatingPicker, Seg } from '../ui';
import QuotePicture from './QuotePicture';
import { setTitleBarColors } from '../TitleBar';

type Spot = { chapter: number; offset: number };
type Target = { offset?: number; anchor?: string; end?: boolean; fraction?: number; hit?: { offset: number; len: number } };
type Panel = null | 'toc' | 'look' | 'search' | 'end';
type Tab = 'toc' | 'bookmarks' | 'quotes';

const posKey = (hash: string) => `polka.pos.${hash}`;

/** Место чтения на этом устройстве (сразу после каждого листания). */
function localSpot(hash: string): (Spot & { at: number }) | null {
  try {
    const v = JSON.parse(stored(posKey(hash), 'null'));
    if (v && Number.isInteger(v.chapter) && Number.isInteger(v.offset) && typeof v.at === 'number') return v;
  } catch {
    /* нет места — с начала */
  }
  return null;
}

/** Глава и доля внутри неё по проценту книги. */
function atPercent(v: BookView, percent: number): { chapter: number; fraction: number } {
  const want = (Math.max(0, Math.min(100, percent)) / 100) * v.total;
  let acc = 0;
  for (let i = 0; i < v.chapters.length; i++) {
    const size = v.chapters[i].size;
    if (size > 0 && acc + size >= want) return { chapter: i, fraction: Math.max(0, (want - acc) / size) };
    acc += size;
  }
  return { chapter: Math.max(0, v.chapters.length - 1), fraction: 1 };
}

export default function Reader({ hash, at, nav, titles, onClose }: { hash: string; at?: ReaderAt; nav: Nav; titles: Title[]; onClose: (r: Reading | null) => void }) {
  const [view, setView] = useState<BookView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [ready, setReady] = useState(false);
  const [look, setLook] = useState<ReaderSettings>(readSettings);
  const [chapter, setChapter] = useState(0);
  const [page, setPage] = useState(0);
  const [pages, setPages] = useState(1);
  const [spot, setSpot] = useState({ offset: 0, length: 1 });
  const [bars, setBars] = useState(false);
  const [panel, setPanel] = useState<Panel>(null);
  const [note, setNote] = useState<{ nodes: BookNode[]; to: string } | null>(null);
  const [back, setBack] = useState<Spot[]>([]);
  const [query, setQuery] = useState('');
  const [hits, setHits] = useState<BookHit[] | null>(null);
  const [searching, setSearching] = useState(false);
  const [scrub, setScrub] = useState<number | null>(null);
  const [wide, setWide] = useState(() => window.innerWidth >= 880);
  const [marks, setMarks] = useState<Mark[]>([]);
  const [tab, setTab] = useState<Tab>('toc');
  const [here, setHere] = useState<Mark | null>(null);
  const [selBar, setSelBar] = useState<{ x: number; y: number } | null>(null);
  const [quotePic, setQuotePic] = useState<Mark | null>(null);
  const [confirmId, setConfirmId] = useState<string | null>(null);

  const paneRef = useRef<HTMLDivElement>(null);
  const flowRef = useRef<HTMLDivElement>(null);
  const noteRef = useRef<HTMLDivElement>(null);
  const tocRef = useRef<HTMLOListElement>(null);
  const pager = useRef<Pager | null>(null);
  const bookRef = useRef<BookView | null>(null);
  const cache = useRef(new Map<number, BookNode[]>());
  const cur = useRef({ chapter: 0, offset: 0, page: 0 });
  const lookRef = useRef(look);
  const navRef = useRef(nav);
  navRef.current = nav;
  const busy = useRef(false);
  // Время чтения (для «Итогов») и скорость (для «осталось около…»).
  const session = useRef({ base: 0, years: {} as Record<string, number>, active: 0, last: Date.now(), saved: 0, savedSpot: '', chars: 0, ms: 0 });
  const marksRef = useRef(marks);
  marksRef.current = marks;
  // Выделение для цитаты: запоминаем, пока показана кнопка (касание может его снять).
  const selRange = useRef<{ range: Range; text: string } | null>(null);
  const hadSelection = useRef(false);
  const touch = useRef<{ x: number; y: number; moved: boolean } | null>(null);
  const skipClick = useRef(false);
  const wheelAt = useRef(0);

  const t = titles.find((x) => x.id === view?.title?.id) ?? view?.title ?? null;
  const starts = useMemo(() => {
    const out: number[] = [];
    let acc = 0;
    for (const c of view?.chapters ?? []) {
      out.push(acc);
      acc += c.size;
    }
    return out;
  }, [view]);
  const startsRef = useRef(starts);
  startsRef.current = starts;

  /** Процент книги по месту в главе. */
  const percentAt = useCallback((ch: number, offset: number, length: number) => {
    const v = bookRef.current;
    if (!v || v.total <= 0 || !v.chapters[ch]) return 0;
    const inside = length > 0 ? Math.min(1, offset / length) : 0;
    return Math.min(100, ((startsRef.current[ch] + inside * v.chapters[ch].size) / v.total) * 100);
  }, []);

  const clearHit = () => highlights()?.delete('polka-hit');
  const markHit = (offset: number, len: number) => {
    const reg = highlights();
    const range = pager.current?.rangeAt(offset, len);
    if (!reg || !range) return;
    const H = (window as unknown as { Highlight: new (r: Range) => unknown }).Highlight;
    reg.set('polka-hit', new H(range));
  };
  /** Цитаты этой главы — подчёркнуты в тексте. */
  const paintQuotes = useCallback(() => {
    const reg = highlights();
    const p = pager.current;
    if (!reg || !p) return;
    const ranges = marksRef.current
      .filter((m) => m.kind === 'quote' && m.book === hash && m.chapter === cur.current.chapter)
      .map((m) => p.rangeAt(m.offset, m.end - m.offset))
      .filter((r): r is Range => !!r);
    const H = (window as unknown as { Highlight: new (...r: Range[]) => unknown }).Highlight;
    if (ranges.length) reg.set('polka-quote', new H(...ranges));
    else reg.delete('polka-quote');
  }, [hash]);

  /** Место — в журнал: не чаще раза в две минуты, при выходе и в конце книги — сразу. */
  const saveReading = useCallback(
    (force: boolean): Reading | null => {
      const v = bookRef.current;
      const p = pager.current;
      const s = session.current;
      if (!v?.title || !p) return null;
      const now = Date.now();
      const sessionMin = Math.round(s.active / 60_000);
      const minutes = s.base + sessionMin;
      const year = String(new Date().getFullYear());
      // То же место и то же время чтения — журнал не трогаем.
      const where = `${cur.current.chapter}:${cur.current.offset}:${minutes}`;
      if (where === s.savedSpot) return null;
      if (!force && now - s.saved < 120_000) return null;
      s.saved = now;
      s.savedSpot = where;
      const r: Reading = {
        title: v.title.id,
        book: hash,
        chapter: cur.current.chapter,
        offset: cur.current.offset,
        percent: Math.round(percentAt(cur.current.chapter, cur.current.offset, p.length) * 10) / 10,
        at: Math.floor(now / 1000),
        minutes,
        years: { ...s.years, [year]: (s.years[year] ?? 0) + sessionMin },
      };
      api.readingSave(r).catch(() => undefined);
      return r;
    },
    [hash, percentAt],
  );

  /** Пришли на страницу: запомнить место, посчитать время и скорость. */
  const arrived = useCallback(
    (ch: number, pg: number) => {
      const p = pager.current;
      if (!p) return;
      const offset = p.offsetOfPage(pg);
      const prev = cur.current;
      const s = session.current;
      const now = Date.now();
      const dt = now - s.last;
      s.last = now;
      s.active += Math.min(dt, 3 * 60_000);
      if (ch === prev.chapter && offset > prev.offset && offset - prev.offset < 6000 && dt > 1500 && dt < 3 * 60_000) {
        s.chars += offset - prev.offset;
        s.ms += dt;
        if (s.ms > 90_000) {
          saveSpeed(readSpeed() * 0.7 + (s.chars / (s.ms / 60_000)) * 0.3);
          s.chars = 0;
          s.ms = 0;
        }
      }
      cur.current = { chapter: ch, offset, page: pg };
      setChapter(ch);
      setPage(pg);
      setPages(p.pages);
      setSpot({ offset, length: p.length });
      store(posKey(hash), JSON.stringify({ chapter: ch, offset, at: Math.floor(now / 1000) }));
      saveReading(false);
    },
    [hash, saveReading],
  );

  const nodesOf = useCallback(
    async (i: number): Promise<BookNode[]> => {
      const c = cache.current;
      const have = c.get(i);
      if (have) return have;
      const nodes = await api.bookChapter(hash, i);
      c.set(i, nodes);
      if (c.size > 8) {
        const first = c.keys().next().value;
        if (first !== undefined && first !== i) c.delete(first);
      }
      return nodes;
    },
    [hash],
  );

  /** Показать главу и встать на место. */
  const show = useCallback(
    async (i: number, target: Target) => {
      const v = bookRef.current;
      const p = pager.current;
      const flow = flowRef.current;
      if (!v || !p || !flow) return;
      const nodes = await nodesOf(i);
      clearHit();
      flow.replaceChildren(build(nodes, (src) => bookImageSrc(hash, src)));
      const end = document.createElement('div');
      end.className = 'rd-end';
      flow.appendChild(end);
      p.index();
      p.layout(lookRef.current.spread);
      let pg = 0;
      if (target.end) pg = p.pages - 1;
      else if (target.hit) pg = p.pageOfOffset(target.hit.offset);
      else if (target.anchor) pg = p.pageOfAnchor(target.anchor) ?? 0;
      else if (target.offset !== undefined) pg = p.pageOfOffset(target.offset);
      else if (target.fraction !== undefined) pg = p.pageOfOffset(Math.floor(target.fraction * p.length));
      p.go(pg);
      if (target.hit) markHit(target.hit.offset, target.hit.len);
      arrived(i, pg);
      paintQuotes();
      if (i + 1 < v.chapters.length) nodesOf(i + 1).catch(() => undefined);
    },
    [hash, nodesOf, arrived, paintQuotes],
  );

  const guard = useCallback(async (f: () => Promise<void>) => {
    if (busy.current) return;
    busy.current = true;
    try {
      await f();
    } catch (e) {
      navRef.current.toast(message(e));
    } finally {
      busy.current = false;
    }
  }, []);

  const turn = useCallback(
    async (dir: 1 | -1) => {
      const p = pager.current;
      const v = bookRef.current;
      if (!p || !v || busy.current) return;
      const { chapter: ch, page: pg } = cur.current;
      clearHit();
      if (dir > 0) {
        if (pg + 1 < p.pages) {
          p.go(pg + 1, true);
          arrived(ch, pg + 1);
        } else if (ch + 1 < v.chapters.length) {
          await guard(() => show(ch + 1, { offset: 0 }));
        } else {
          p.go(pg, true);
          saveReading(true);
          setPanel('end');
        }
      } else if (pg > 0) {
        p.go(pg - 1, true);
        arrived(ch, pg - 1);
      } else if (ch > 0) {
        await guard(() => show(ch - 1, { end: true }));
      } else {
        p.go(0, true);
      }
    },
    [arrived, guard, show, saveReading],
  );

  /** Переход по оглавлению, ссылке, поиску или ползунку. */
  const jump = useCallback(
    async (ch: number, target: Target, remember = true) => {
      const p = pager.current;
      if (!p) return;
      if (remember) setBack((b) => [...b.slice(-9), { chapter: cur.current.chapter, offset: cur.current.offset }]);
      setPanel(null);
      setNote(null);
      setBars(false);
      if (ch === cur.current.chapter && !target.end && target.fraction === undefined) {
        let pg: number | null = null;
        if (target.hit) pg = p.pageOfOffset(target.hit.offset);
        else if (target.anchor) pg = p.pageOfAnchor(target.anchor);
        else if (target.offset !== undefined) pg = p.pageOfOffset(target.offset);
        else pg = 0;
        if (pg !== null) {
          clearHit();
          p.go(pg);
          if (target.hit) markHit(target.hit.offset, target.hit.len);
          arrived(ch, pg);
          return;
        }
      }
      await guard(() => show(ch, target));
    },
    [arrived, guard, show],
  );

  const relayout = useCallback(() => {
    const p = pager.current;
    if (!p || !bookRef.current) return;
    p.layout(lookRef.current.spread);
    const pg = p.pageOfOffset(cur.current.offset);
    p.go(pg);
    cur.current.page = pg;
    setPage(pg);
    setPages(p.pages);
    setSpot({ offset: cur.current.offset, length: p.length });
  }, []);
  const relayoutSoon = useMemo(() => {
    let frame = 0;
    return () => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(relayout);
    };
  }, [relayout]);

  const close = useCallback(() => onClose(saveReading(true)), [onClose, saveReading]);

  // Открыть книгу.
  useEffect(() => {
    let alive = true;
    addReadingFonts();
    api
      .bookOpen(hash)
      .then((v) => {
        if (!alive) return;
        bookRef.current = v;
        if (v.title) navRef.current.update(v.title);
        setView(v);
      })
      .catch((e) => alive && setError(message(e)));
    return () => {
      alive = false;
    };
  }, [hash]);

  // Встать на место: своё на этом устройстве или то, что пришло с другого (оно новее).
  useEffect(() => {
    if (!view || !paneRef.current || !flowRef.current) return;
    pager.current = new Pager(paneRef.current, flowRef.current);
    let alive = true;
    (async () => {
      await readingFontsReady(FONTS[lookRef.current.font].family, lookRef.current.size);
      const local = localSpot(hash);
      let synced: Reading | undefined;
      if (view.title) {
        try {
          synced = (await api.readings()).find((r) => r.title === view.title?.id);
        } catch {
          /* без журнала — по своему месту */
        }
      }
      if (synced) {
        session.current.base = synced.minutes;
        session.current.years = { ...(synced.years ?? {}) };
        // Место из журнала уже записано — пока не перелистнули, писать его снова незачем.
        if (synced.book === hash) session.current.savedSpot = `${synced.chapter}:${synced.offset}:${synced.minutes}`;
        session.current.saved = Date.now();
      }
      const last = view.chapters.length - 1;
      const clamp = (c: number) => Math.max(0, Math.min(last, c));
      let start: { chapter: number; target: Target; note?: string } = { chapter: 0, target: { offset: 0 } };
      if (synced && (!local || synced.at > local.at + 30)) {
        const elsewhere = synced.percent >= 0.5 ? 'Продолжаем с места, где читали на другом устройстве' : undefined;
        if (synced.book === hash) start = { chapter: clamp(synced.chapter), target: { offset: synced.offset }, note: elsewhere };
        else {
          const at = atPercent(view, synced.percent);
          start = { chapter: at.chapter, target: { fraction: at.fraction }, note: `Продолжаем примерно с ${Math.round(synced.percent)} % — как на другом устройстве` };
        }
      } else if (local) {
        start = { chapter: clamp(local.chapter), target: { offset: local.offset } };
      }
      // Открыли цитату из карточки — сразу к ней.
      if (at) start = { chapter: clamp(at.chapter), target: { hit: { offset: at.offset, len: at.len } } };
      if (!alive) return;
      await guard(() => show(start.chapter, start.target));
      if (!alive) return;
      setReady(true);
      if (start.note) navRef.current.toast(start.note);
    })().catch((e) => alive && setError(message(e)));
    return () => {
      alive = false;
    };
    // Место для начала берётся один раз — при открытии книги.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [view, hash, guard, show]);

  // Вид страницы: запомнить и разложить заново на том же месте.
  useEffect(() => {
    lookRef.current = look;
    saveSettings(look);
    if (!ready) return;
    let alive = true;
    readingFontsReady(FONTS[look.font].family, look.size).then(() => alive && relayout());
    return () => {
      alive = false;
    };
  }, [look, ready, relayout]);

  // Размер окна, картинки и шрифты меняют раскладку.
  useEffect(() => {
    const pane = paneRef.current;
    const flow = flowRef.current;
    if (!pane || !flow || !ready) return;
    const ro = new ResizeObserver(() => {
      setWide(window.innerWidth >= 880);
      relayoutSoon();
    });
    ro.observe(pane);
    const onLoad = (e: Event) => e.target instanceof HTMLImageElement && relayoutSoon();
    flow.addEventListener('load', onLoad, true);
    flow.addEventListener('error', onLoad, true);
    document.fonts?.addEventListener?.('loadingdone', relayoutSoon);
    return () => {
      ro.disconnect();
      flow.removeEventListener('load', onLoad, true);
      flow.removeEventListener('error', onLoad, true);
      document.fonts?.removeEventListener?.('loadingdone', relayoutSoon);
    };
  }, [ready, relayoutSoon]);

  // Ушли из приложения — место в журнал; экран не гаснет, пока читаете.
  useWakeLock(() => saveReading(true));

  // Клавиши: стрелки, PageUp/PageDown, пробел; Escape закрывает панель или книгу.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement) {
        if (e.key === 'Escape') setPanel(null);
        return;
      }
      if (e.key === 'Escape') {
        if (note) setNote(null);
        else if (panel) setPanel(null);
        else if (bars) setBars(false);
        else close();
        return;
      }
      if (panel || note) return;
      if (e.key === 'ArrowRight' || e.key === 'PageDown' || (e.key === ' ' && !e.shiftKey)) {
        e.preventDefault();
        turn(1);
      } else if (e.key === 'ArrowLeft' || e.key === 'PageUp' || (e.key === ' ' && e.shiftKey)) {
        e.preventDefault();
        turn(-1);
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [turn, panel, note, bars, close]);

  // Закладки и цитаты этой записи.
  useEffect(() => {
    if (!view?.title) return;
    api
      .marks(view.title.id)
      .then(setMarks)
      .catch(() => undefined);
  }, [view]);
  useEffect(() => {
    if (ready) paintQuotes();
  }, [marks, ready, paintQuotes]);

  // Есть ли закладка на этой странице.
  useEffect(() => {
    const p = pager.current;
    if (!p || !ready) return setHere(null);
    const start = p.offsetOfPage(page);
    const next = page + 1 < pages ? p.offsetOfPage(page + 1) : Number.MAX_SAFE_INTEGER;
    setHere(marks.find((m) => m.kind === 'bookmark' && m.book === hash && m.chapter === chapter && m.offset >= start && m.offset < next) ?? null);
  }, [marks, chapter, page, pages, ready, hash, spot]);

  // Выделили текст — кнопка «В цитаты» под выделением.
  useEffect(() => {
    if (!ready) return;
    let timer = 0;
    const onSelection = () => {
      window.clearTimeout(timer);
      timer = window.setTimeout(() => {
        const sel = window.getSelection();
        const flow = flowRef.current;
        if (!sel || sel.isCollapsed || !sel.rangeCount || !flow || !flow.contains(sel.getRangeAt(0).commonAncestorContainer)) {
          setSelBar(null);
          return;
        }
        const range = sel.getRangeAt(0);
        selRange.current = { range: range.cloneRange(), text: sel.toString() };
        const rects = range.getClientRects();
        const first = rects[0] ?? range.getBoundingClientRect();
        const last = rects[rects.length - 1] ?? first;
        const below = last.bottom + 12;
        const y = below + 52 < window.innerHeight ? below : Math.max(8, first.top - 60);
        const x = Math.min(window.innerWidth - 120, Math.max(120, (first.left + last.right) / 2));
        setSelBar({ x, y });
      }, 180);
    };
    document.addEventListener('selectionchange', onSelection);
    return () => {
      document.removeEventListener('selectionchange', onSelection);
      window.clearTimeout(timer);
    };
  }, [ready]);

  // Android: «Назад» закрывает сноску, панель или книгу, а не всю «Полку».
  useAndroidBack(() => {
    if (note) setNote(null);
    else if (panel) setPanel(null);
    else if (bars) setBars(false);
    else close();
  });

  // Сноска во всплывающем окне.
  useEffect(() => {
    const el = noteRef.current;
    if (el && note) el.replaceChildren(build(note.nodes, (src) => bookImageSrc(hash, src)));
  }, [note, hash]);

  useEffect(() => {
    if (panel === 'toc') tocRef.current?.querySelector('.on')?.scrollIntoView({ block: 'center' });
  }, [panel]);

  const follow = async (el: HTMLElement) => {
    const ext = el.dataset.ext;
    if (ext) {
      api.openLink(ext).catch((e) => nav.toast(message(e)));
      return;
    }
    const to = el.dataset.to ?? '';
    const [chs, anchor] = to.split('#');
    const ch = Number(chs);
    const v = bookRef.current;
    if (!v || !Number.isInteger(ch) || ch < 0 || ch >= v.chapters.length) return;
    const short = (el.textContent ?? '').trim().length <= 4;
    if (anchor && (el.dataset.note === '1' || short)) {
      try {
        const found = findNote(await nodesOf(ch), anchor);
        if (found) {
          setNote({ nodes: found, to });
          return;
        }
      } catch {
        /* не нашли — просто переходим */
      }
    }
    jump(ch, { anchor });
  };

  const onPaneClick = (e: ReactMouseEvent<HTMLDivElement>) => {
    if (skipClick.current) {
      skipClick.current = false;
      return;
    }
    const sel = window.getSelection();
    if (sel && !sel.isCollapsed) return;
    // Касание сняло выделение — страницу не листаем.
    if (hadSelection.current) {
      hadSelection.current = false;
      setSelBar(null);
      return;
    }
    const link = (e.target as HTMLElement).closest('[data-to],[data-ext]') as HTMLElement | null;
    if (link && flowRef.current?.contains(link)) {
      follow(link);
      return;
    }
    if (bars) {
      setBars(false);
      return;
    }
    const r = e.currentTarget.getBoundingClientRect();
    const x = (e.clientX - r.left) / r.width;
    if (x < 0.3) turn(-1);
    else if (x > 0.7) turn(1);
    else setBars(true);
  };

  const onPointerDown = () => {
    const sel = window.getSelection();
    hadSelection.current = !!sel && !sel.isCollapsed;
  };
  const onTouchStart = (e: ReactTouchEvent) => {
    const sel = window.getSelection();
    if (e.touches.length !== 1 || panel || note || (sel && !sel.isCollapsed)) return;
    touch.current = { x: e.touches[0].clientX, y: e.touches[0].clientY, moved: false };
  };
  const onTouchMove = (e: ReactTouchEvent) => {
    const d = touch.current;
    if (!d) return;
    const dx = e.touches[0].clientX - d.x;
    const dy = e.touches[0].clientY - d.y;
    if (!d.moved && Math.abs(dx) > 12 && Math.abs(dx) > Math.abs(dy) * 1.2) d.moved = true;
    if (d.moved) pager.current?.drag(dx);
  };
  const onTouchEnd = (e: ReactTouchEvent) => {
    const d = touch.current;
    touch.current = null;
    if (!d?.moved) return;
    skipClick.current = true;
    setTimeout(() => (skipClick.current = false), 400);
    const dx = e.changedTouches[0].clientX - d.x;
    if (Math.abs(dx) > 50) turn(dx < 0 ? 1 : -1);
    else pager.current?.go(cur.current.page, true);
  };
  const onWheel = (e: ReactWheelEvent) => {
    if (panel || note) return;
    const d = Math.abs(e.deltaY) >= Math.abs(e.deltaX) ? e.deltaY : e.deltaX;
    const now = Date.now();
    if (Math.abs(d) < 6 || now - wheelAt.current < 320) return;
    wheelAt.current = now;
    turn(d > 0 ? 1 : -1);
  };

  const toggleBookmark = async () => {
    const p = pager.current;
    const v = bookRef.current;
    if (!p || !v?.title) return;
    try {
      if (here) {
        await api.markDelete(here.id);
        setMarks((l) => l.filter((m) => m.id !== here.id));
        nav.toast('Закладка убрана');
        return;
      }
      const { chapter: ch, offset } = cur.current;
      const m = await api.markSave({ id: '', title: v.title.id, book: hash, kind: 'bookmark', chapter: ch, offset, end: 0, text: p.textFrom(offset, 140), note: '', percent: Math.round(percentAt(ch, offset, p.length) * 10) / 10, created: 0 });
      setMarks((l) => [...l, m]);
      nav.toast('Закладка поставлена');
    } catch (e) {
      nav.toast(message(e));
    }
  };

  /** Где выделение в главе: знаки от начала текста главы. */
  const selectionSpot = () => {
    const sel = selRange.current;
    const flow = flowRef.current;
    if (!sel || !flow || !flow.contains(sel.range.commonAncestorContainer)) return null;
    const pre = document.createRange();
    pre.selectNodeContents(flow);
    pre.setEnd(sel.range.startContainer, sel.range.startOffset);
    const offset = pre.toString().length;
    const end = offset + sel.range.toString().length;
    const text = sel.text.replace(/[ \t\u00a0]+/g, ' ').replace(/\s*\n\s*/g, '\n').trim();
    return end > offset && text ? { offset, end, text } : null;
  };
  const dropSelection = () => {
    window.getSelection()?.removeAllRanges();
    selRange.current = null;
    setSelBar(null);
  };
  const saveQuote = async () => {
    const place = selectionSpot();
    const v = bookRef.current;
    const p = pager.current;
    if (!place || !v?.title || !p) return dropSelection();
    try {
      const m = await api.markSave({ id: '', title: v.title.id, book: hash, kind: 'quote', chapter: cur.current.chapter, offset: place.offset, end: place.end, text: place.text, note: '', percent: Math.round(percentAt(cur.current.chapter, place.offset, p.length) * 10) / 10, created: 0 });
      setMarks((l) => [...l, m]);
      dropSelection();
      nav.toast('Цитата сохранена — она и в карточке книги');
    } catch (e) {
      nav.toast(message(e));
    }
  };
  const searchSelection = () => {
    const place = selectionSpot();
    dropSelection();
    if (!place) return;
    const q = place.text.replace(/\s+/g, ' ').slice(0, 200);
    setQuery(q);
    setPanel('search');
    find(q);
  };
  const removeMark = async (m: Mark) => {
    if (confirmId !== m.id) return setConfirmId(m.id);
    setConfirmId(null);
    try {
      await api.markDelete(m.id);
      setMarks((l) => l.filter((x) => x.id !== m.id));
    } catch (e) {
      nav.toast(message(e));
    }
  };

  const find = async (q: string) => {
    if (q.trim().length < 2) return;
    setSearching(true);
    try {
      setHits(await api.bookSearch(hash, q.trim()));
    } catch (err) {
      nav.toast(message(err));
    } finally {
      setSearching(false);
    }
  };
  const runSearch = async (e: FormEvent) => {
    e.preventDefault();
    const q = query.trim();
    if (q.length < 2) return;
    await find(q);
  };

  const finish = async () => {
    if (!t) return;
    try {
      nav.update(await api.setStatus(t.id, 'done'));
      nav.toast(`«${t.title}» — прочитано`);
    } catch (e) {
      nav.toast(message(e));
    }
  };
  const rate = async (rating: number | null) => {
    if (!t) return;
    try {
      nav.update(await api.save({ ...t, rating: rating ?? undefined }));
    } catch (e) {
      nav.toast(message(e));
    }
  };
  const commitScrub = () => {
    if (scrub === null || !view) return;
    const at = atPercent(view, scrub);
    setScrub(null);
    jump(at.chapter, { fraction: at.fraction });
  };
  const set = (patch: Partial<ReaderSettings>) => setLook((l) => ({ ...l, ...patch }));

  const byPlace = (a: Mark, b: Mark) => a.chapter - b.chapter || a.offset - b.offset;
  const bookmarks = marks.filter((m) => m.kind === 'bookmark' && m.book === hash).sort(byPlace);
  const quotes = marks.filter((m) => m.kind === 'quote' && m.book === hash).sort(byPlace);
  const theme = THEMES[resolveTheme(look.theme)];
  // Полоса окна на компьютере — цвета темы читалки, после читалки — снова цвета «Полки».
  useEffect(() => setTitleBarColors(theme.bg, theme.ink), [theme.bg, theme.ink]);
  useEffect(() => () => setTitleBarColors(null), []);
  const style = {
    '--rd-bg': theme.bg,
    '--rd-ink': theme.ink,
    '--rd-muted': theme.muted,
    '--rd-link': theme.link,
    '--rd-line': theme.line,
    '--rd-hit': theme.hit,
    '--rd-bar': theme.bar,
    '--rd-size': `${look.size}px`,
    '--rd-leading': String(look.leading),
    '--rd-font': FONTS[look.font].stack,
    '--rd-mshare': String(MARGINS[look.margins].share),
    '--rd-mmin': `${MARGINS[look.margins].min}px`,
    '--rd-align': look.justify ? 'justify' : 'left',
  } as CSSProperties;

  const percent = view ? percentAt(chapter, spot.offset, spot.length) : 0;
  const shown = scrub ?? percent;
  const doneChars = view ? (percent / 100) * view.total : 0;
  const left = view ? timeLeft(Math.max(0, view.total - doneChars) / readSpeed()) : '';
  const tocAt = useMemo(() => {
    if (!view) return undefined;
    const ch = scrub !== null ? atPercent(view, scrub).chapter : chapter;
    // В своей главе — последний раздел, который уже начался на этой странице или раньше.
    let best = [...view.toc].reverse().find((x) => x.chapter < ch);
    for (const it of view.toc) {
      if (it.chapter !== ch) continue;
      const pg = scrub === null && it.anchor ? pager.current?.pageOfAnchor(it.anchor) : 0;
      if (pg === null || pg === undefined || pg <= page) best = it;
    }
    return best;
  }, [view, chapter, page, pages, scrub]);
  const chapterTitle = tocAt?.title || view?.chapters[chapter]?.title || '';

  return (
    <div className={`reader theme-${resolveTheme(look.theme)}`} style={style}>
      <div className="rd-page" onPointerDown={onPointerDown} onClick={onPaneClick} onTouchStart={onTouchStart} onTouchMove={onTouchMove} onTouchEnd={onTouchEnd} onWheel={onWheel}>
        <div className="rd-view" ref={paneRef}>
          <div ref={flowRef} className="rd-flow" lang={view?.meta.lang || 'ru'} />
        </div>
      </div>
      {here && !bars && (
        <svg className="rd-ribbon" viewBox="0 0 24 24" aria-label="Закладка на этой странице" role="img">
          <path d="M19 21l-7-4-7 4V5a2 2 0 0 1 2-2h10a2 2 0 0 1 2 2Z" />
        </svg>
      )}
      <div className="rd-status" aria-hidden={bars}>
        <span className="rd-status-title">{chapterTitle}</span>
        <span className="num">{ready ? `${page + 1} / ${pages} · ${pct(percent)}` : ''}</span>
      </div>

      {!ready && !error && <div className="rd-wait">Открываю книгу…</div>}
      {error && (
        <div className="rd-wait">
          <p>{error}</p>
          <button type="button" className="btn line" onClick={() => onClose(null)}>
            Закрыть
          </button>
        </div>
      )}

      {bars && view && (
        <div className="rd-top">
          <button type="button" className="icon-btn" aria-label="Закрыть книгу" onClick={close}>
            <Icon name="left" size={24} stroke={2} />
          </button>
          <div className="rd-top-title">
            <b>{t?.title || view.meta.title}</b>
            <span>{(t?.creators.length ? t.creators : view.meta.authors).join(', ')}</span>
          </div>
          <button
            type="button"
            className="icon-btn"
            aria-label="Оглавление, закладки и цитаты"
            onClick={() => {
              setTab('toc');
              setPanel('toc');
            }}
          >
            <Icon name="toc" />
          </button>
          <button type="button" className="icon-btn" aria-label="Поиск по книге" onClick={() => setPanel('search')}>
            <Icon name="search" />
          </button>
          <button type="button" className="icon-btn" aria-label="Вид страницы" onClick={() => setPanel('look')}>
            <Icon name="type" />
          </button>
          {t && (
            <button type="button" className={`icon-btn rd-mark-btn${here ? ' on' : ''}`} aria-label={here ? 'Убрать закладку' : 'Закладка на этой странице'} aria-pressed={!!here} onClick={toggleBookmark}>
              <Icon name="want" />
            </button>
          )}
        </div>
      )}
      {bars && view && (
        <div className="rd-bottom">
          <p className="rd-chapter">{chapterTitle}</p>
          <input
            type="range"
            className="rd-slider"
            min={0}
            max={1000}
            value={Math.round(shown * 10)}
            aria-label="Место в книге"
            style={{ '--rd-fill': `${shown}%` } as CSSProperties}
            onChange={(e) => setScrub(Number(e.target.value) / 10)}
            onPointerUp={commitScrub}
            onKeyUp={commitScrub}
            onBlur={commitScrub}
          />
          <div className="rd-meta num">
            <span>{scrub !== null ? pct(scrub) : `Стр. ${page + 1} из ${pages}`}</span>
            <span>{pct(shown)}</span>
            <span>Осталось ≈ {left}</span>
          </div>
        </div>
      )}
      {back.length > 0 && !panel && (
        <button
          type="button"
          className={`rd-back${bars ? ' up' : ''}`}
          onClick={() => {
            const last = back[back.length - 1];
            setBack(back.slice(0, -1));
            jump(last.chapter, { offset: last.offset }, false);
          }}
        >
          <Icon name="left" size={18} stroke={2} />
          <span>Назад</span>
        </button>
      )}

      {selBar && t && (
        <div className="rd-selbar" style={{ left: selBar.x, top: selBar.y }} onPointerDown={(e) => e.preventDefault()}>
          <button type="button" onClick={saveQuote}>
            <Icon name="quote" size={16} />
            <span>В цитаты</span>
          </button>
          <button type="button" onClick={searchSelection}>
            <Icon name="search" size={16} />
            <span>Искать</span>
          </button>
        </div>
      )}
      {quotePic && t && <QuotePicture t={t} mark={quotePic} nav={nav} onClose={() => setQuotePic(null)} />}

      {note && (
        <Sheet title="Сноска" onClose={() => setNote(null)}>
          <div
            className="rd-note rd-flow-like"
            ref={noteRef}
            lang={view?.meta.lang || 'ru'}
            onClick={(e) => {
              const link = (e.target as HTMLElement).closest('[data-to],[data-ext]') as HTMLElement | null;
              if (link) follow(link);
            }}
          />
          <button
            type="button"
            className="btn line"
            onClick={() => {
              const [ch, anchor] = note.to.split('#');
              jump(Number(ch), { anchor });
            }}
          >
            Перейти к сноске
          </button>
        </Sheet>
      )}

      {panel === 'toc' && view && (
        <Sheet title={tab === 'toc' ? 'Оглавление' : tab === 'bookmarks' ? 'Закладки' : 'Цитаты'} side onClose={() => setPanel(null)}>
          {t && (
            <Seg
              cols
              small
              label="Раздел"
              value={tab}
              options={[
                ['toc', 'Оглавление'],
                ['bookmarks', `Закладки${bookmarks.length ? ` ${bookmarks.length}` : ''}`],
                ['quotes', `Цитаты${quotes.length ? ` ${quotes.length}` : ''}`],
              ]}
              onChange={(v) => {
                setTab(v);
                setConfirmId(null);
              }}
            />
          )}
          {tab !== 'toc' ? (
            <MarkList
              list={tab === 'bookmarks' ? bookmarks : quotes}
              empty={tab === 'bookmarks' ? 'Закладок пока нет. Поставить — значок закладки вверху, когда открыты панели.' : 'Цитат пока нет. Выделите текст в книге и нажмите «В цитаты».'}
              where={(m) => `${view.chapters[m.chapter]?.title || `Глава ${m.chapter + 1}`} · ${pct(m.percent)}`}
              confirmId={confirmId}
              onOpen={(m) => jump(m.chapter, m.kind === 'quote' ? { hit: { offset: m.offset, len: m.end - m.offset } } : { offset: m.offset })}
              onShare={(m) => setQuotePic(m)}
              onRemove={removeMark}
            />
          ) : view.toc.length ? (
            <ol className="rd-toc" ref={tocRef}>
              {view.toc.map((it, i) => (
                <li key={i}>
                  <button type="button" className={it === tocAt ? 'on' : ''} style={{ paddingLeft: 12 + Math.min(it.depth, 4) * 16 }} onClick={() => jump(it.chapter, { anchor: it.anchor })}>
                    {it.title}
                  </button>
                </li>
              ))}
            </ol>
          ) : (
            <p className="muted">В книге нет оглавления.</p>
          )}
        </Sheet>
      )}

      {panel === 'search' && (
        <Sheet title="Поиск по книге" side onClose={() => setPanel(null)}>
          <form className="rd-search" onSubmit={runSearch}>
            <input type="search" value={query} autoFocus placeholder="Слово или фраза" aria-label="Что найти" onChange={(e) => setQuery(e.target.value)} maxLength={200} />
            <button type="submit" className="btn primary small" disabled={searching || query.trim().length < 2}>
              Найти
            </button>
          </form>
          {searching && <p className="muted">Ищу…</p>}
          {hits && !searching && (
            <p className="small muted">{hits.length ? `${plural(hits.length, 'место', 'места', 'мест')}${hits.length >= 300 ? ' (первые)' : ''}` : 'Ничего не нашлось'}</p>
          )}
          {hits && !searching && hits.length > 0 && (
            <ul className="rd-hits">
              {hits.map((h, i) => (
                <li key={i}>
                  <button type="button" onClick={() => jump(h.chapter, { hit: { offset: h.offset, len: h.len } })}>
                    <span className="small muted">{view?.chapters[h.chapter]?.title || `Глава ${h.chapter + 1}`}</span>
                    <span>
                      …{h.before}
                      <mark>{h.found}</mark>
                      {h.after}…
                    </span>
                  </button>
                </li>
              ))}
            </ul>
          )}
        </Sheet>
      )}

      {panel === 'look' && (
        <Sheet title="Вид страницы" onClose={() => setPanel(null)}>
          <div className="rd-look">
            <div className="rd-size">
              <button type="button" className="btn soft" aria-label="Мельче" disabled={look.size <= SIZE_MIN} onClick={() => set({ size: look.size - 1 })}>
                <span className="rd-a small">А</span>
              </button>
              <span className="num">{look.size}</span>
              <button type="button" className="btn soft" aria-label="Крупнее" disabled={look.size >= SIZE_MAX} onClick={() => set({ size: look.size + 1 })}>
                <span className="rd-a big">А</span>
              </button>
            </div>
            <Seg cols small label="Шрифт" value={look.font} options={[['serif', FONTS.serif.label], ['sans', FONTS.sans.label]] as [ReaderFont, string][]} onChange={(font) => set({ font })} />
            <Seg cols small label="Интервал" value={String(look.leading)} options={LEADINGS.map(([v, l]) => [String(v), l] as [string, string])} onChange={(v) => set({ leading: Number(v) })} />
            <Seg cols small label="Поля" value={look.margins} options={(Object.keys(MARGINS) as (keyof typeof MARGINS)[]).map((k) => [k, MARGINS[k].label] as [typeof k, string])} onChange={(margins) => set({ margins })} />
            <div className="rd-themes" role="group" aria-label="Тема">
              {THEME_ORDER.map(([id, label]) => {
                const th = THEMES[resolveTheme(id)];
                return (
                  <button key={id} type="button" className={look.theme === id ? 'on' : ''} aria-pressed={look.theme === id} style={{ background: th.bg, color: th.ink }} onClick={() => set({ theme: id })}>
                    {label}
                  </button>
                );
              })}
            </div>
            <Seg cols small label="Выравнивание" value={look.justify ? 'justify' : 'left'} options={[['justify', 'По ширине'], ['left', 'По левому краю']] as [string, string][]} onChange={(v) => set({ justify: v === 'justify' })} />
            {wide && (
              <label className="rd-check">
                <input type="checkbox" checked={look.spread} onChange={(e) => set({ spread: e.target.checked })} />
                <span>Две страницы рядом</span>
              </label>
            )}
          </div>
        </Sheet>
      )}

      {panel === 'end' && view && (
        <Sheet title="Конец книги" onClose={() => setPanel(null)}>
          <p>
            Вы дочитали «{t?.title || view.meta.title}».
          </p>
          {t && t.status !== 'done' && (
            <button type="button" className="btn primary" onClick={finish}>
              <Icon name="check" stroke={2.25} />
              <span>Отметить «Прочитано»</span>
            </button>
          )}
          {t && t.status === 'done' && <p className="muted">Отмечено «Прочитано»{t.finished ? ` ${dayMonth(t.finished)}` : ''}.</p>}
          {t && (
            <div className="stack-s">
              <h3 className="label">Оценка</h3>
              <RatingPicker value={t.rating} onChange={rate} />
            </div>
          )}
          <button type="button" className="btn line" onClick={close}>
            Закрыть книгу
          </button>
        </Sheet>
      )}
    </div>
  );
}
