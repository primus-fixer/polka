// Читалка PDF: страницы рисует pdf.js (PDFViewer) — лентой или по одной, по ширине экрана или целиком;
// масштаб щипком, кнопками и Ctrl+колесом. Оглавление из файла, поиск по тексту, закладки, цитаты
// из выделения, место чтения и время — как у EPUB и FB2. Место в книге: страница (с нуля) и доля её
// высоты в десятитысячных; места цитат и найденного — знаки текста страницы.
import { useCallback, useEffect, useMemo, useRef, useState, type CSSProperties, type FormEvent, type MouseEvent as ReactMouseEvent, type TouchEvent as ReactTouchEvent, type WheelEvent as ReactWheelEvent } from 'react';
import type { PDFDocumentLoadingTask, PDFDocumentProxy } from 'pdfjs-dist/legacy/build/pdf.mjs';
import type { EventBus, PDFLinkService, PDFViewer } from 'pdfjs-dist/legacy/web/pdf_viewer.mjs';
import type { Nav, ReaderAt } from '../App';
import { api, isMobileApp, message, type BookView, type Mark, type Reading, type Title } from '../api';
import { dayMonth, plural } from '../format';
import { coverFromDoc, docOptions, findInPage, loadPdf, normQuery, offsetIn, pageText, pdfError, rangeIn, type PageText, type PdfHit } from '../reader/pdf';
import { readSpeed, resolveTheme, THEMES, type ReaderTheme } from '../reader/settings';
import { MarkList, Sheet, THEME_ORDER, highlight, pct, timeLeft, useAndroidBack, useWakeLock } from '../reader/ui';
import { store, stored } from '../storage';
import { Icon, RatingPicker, Seg } from '../ui';
import QuotePicture from './QuotePicture';

type Panel = null | 'toc' | 'look' | 'search' | 'end';
type Tab = 'toc' | 'bookmarks' | 'quotes';
type Fit = 'auto' | 'page-width' | 'page-fit';
/** Вид PDF на этом устройстве. `zoom` — заданный щипком или кнопками масштаб, иначе `fit`. */
type PdfLook = { theme: ReaderTheme; flow: 'scroll' | 'pages'; fit: Fit; zoom: number | null; spread: boolean };
/** Место: страница (с единицы) и доля её высоты у верхнего края экрана. */
type Spot = { page: number; frac: number };
type OutlineItem = { title: string; depth: number; dest: unknown; url: string | null; page: number | null };

const LOOK_KEY = 'polka.pdf';
const CHARS_PER_PAGE = 1800;
const EMPTY_TEXT: PageText = { text: '', shown: '', norm: '', back: [] };

function readLook(): PdfLook {
  const d: PdfLook = { theme: 'auto', flow: 'scroll', fit: 'auto', zoom: null, spread: false };
  try {
    const v = JSON.parse(stored(LOOK_KEY, '{}')) as Partial<PdfLook>;
    return {
      theme: v.theme === 'auto' || (v.theme && v.theme in THEMES) ? v.theme : d.theme,
      flow: v.flow === 'pages' ? 'pages' : 'scroll',
      fit: v.fit === 'page-width' || v.fit === 'page-fit' ? v.fit : 'auto',
      zoom: typeof v.zoom === 'number' && v.zoom >= 0.25 && v.zoom <= 6 ? v.zoom : null,
      spread: v.spread === true,
    };
  } catch {
    return d;
  }
}

const posKey = (hash: string) => `polka.pos.${hash}`;
function localSpot(hash: string): (Spot & { at: number }) | null {
  try {
    const v = JSON.parse(stored(posKey(hash), 'null'));
    if (v && Number.isInteger(v.page) && v.page >= 1 && typeof v.frac === 'number' && typeof v.at === 'number') return v;
  } catch {
    /* нет места — с начала */
  }
  return null;
}

export default function PdfReader({ hash, at, nav, titles, onClose }: { hash: string; at?: ReaderAt; nav: Nav; titles: Title[]; onClose: (r: Reading | null) => void }) {
  const [view, setView] = useState<BookView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [ready, setReady] = useState(false);
  const [look, setLook] = useState<PdfLook>(readLook);
  const [page, setPage] = useState(1);
  const [pages, setPages] = useState(0);
  const [frac, setFrac] = useState(0);
  const [scale, setScale] = useState(1);
  const [bars, setBars] = useState(false);
  const [panel, setPanel] = useState<Panel>(null);
  const [outline, setOutline] = useState<OutlineItem[]>([]);
  const [back, setBack] = useState<Spot[]>([]);
  const [query, setQuery] = useState('');
  const [hits, setHits] = useState<PdfHit[] | null>(null);
  const [searching, setSearching] = useState<{ done: number; total: number } | null>(null);
  const [noText, setNoText] = useState(false);
  const [scrub, setScrub] = useState<number | null>(null);
  const [goPage, setGoPage] = useState('');
  const [marks, setMarks] = useState<Mark[]>([]);
  const [tab, setTab] = useState<Tab>('toc');
  const [selBar, setSelBar] = useState<{ x: number; y: number } | null>(null);
  const [quotePic, setQuotePic] = useState<Mark | null>(null);
  const [confirmId, setConfirmId] = useState<string | null>(null);
  const [wide, setWide] = useState(() => window.innerWidth >= 880);

  const boxRef = useRef<HTMLDivElement>(null);
  const innerRef = useRef<HTMLDivElement>(null);
  const viewer = useRef<PDFViewer | null>(null);
  const bus = useRef<EventBus | null>(null);
  const links = useRef<PDFLinkService | null>(null);
  const doc = useRef<PDFDocumentProxy | null>(null);
  const task = useRef<PDFDocumentLoadingTask | null>(null);
  const modes = useRef<{ scroll: { VERTICAL: number; PAGE: number }; spread: { NONE: number; ODD: number } } | null>(null);
  const texts = useRef(new Map<number, Promise<PageText>>());
  const bookRef = useRef<BookView | null>(null);
  const cur = useRef<Spot>({ page: 1, frac: 0 });
  const navRef = useRef(nav);
  navRef.current = nav;
  const marksRef = useRef(marks);
  marksRef.current = marks;
  const lookRef = useRef(look);
  lookRef.current = look;
  const hitRef = useRef<{ page: number; offset: number; len: number; scroll: boolean } | null>(null);
  const session = useRef({ base: 0, years: {} as Record<string, number>, active: 0, last: Date.now(), saved: 0, savedSpot: '' });
  const selRange = useRef<{ range: Range; text: string } | null>(null);
  const hadSelection = useRef(false);
  const searchToken = useRef(0);
  const swipe = useRef<{ x: number; y: number; two: boolean } | null>(null);
  const skipClick = useRef(false);
  const wheelAt = useRef(0);

  const t = titles.find((x) => x.id === view?.title?.id) ?? view?.title ?? null;

  const textOf = useCallback((n: number): Promise<PageText> => {
    const d = doc.current;
    if (!d || n < 1 || n > d.numPages) return Promise.resolve(EMPTY_TEXT);
    let p = texts.current.get(n);
    if (!p) {
      p = pageText(d, n).catch(() => EMPTY_TEXT);
      texts.current.set(n, p);
    }
    return p;
  }, []);

  const layerOf = (n: number): HTMLElement | null => innerRef.current?.querySelector(`.page[data-page-number="${n}"] .textLayer`) ?? null;

  const percentOf = useCallback((s: Spot) => {
    const total = doc.current?.numPages || bookRef.current?.total || 1;
    return Math.max(0, Math.min(100, ((s.page - 1 + s.frac) / total) * 100));
  }, []);

  /** Цитаты на видимых страницах и найденное — подсветкой поверх текстового слоя. */
  const paint = useCallback(() => {
    const quotes: Range[] = [];
    for (const m of marksRef.current) {
      if (m.kind !== 'quote' || m.book !== hash) continue;
      const layer = layerOf(m.chapter + 1);
      const r = layer && rangeIn(layer, m.offset, m.end - m.offset);
      if (r) quotes.push(r);
    }
    highlight('polka-quote', quotes);
    const h = hitRef.current;
    const layer = h ? layerOf(h.page) : null;
    const hr = h && layer ? rangeIn(layer, h.offset, h.len) : null;
    highlight('polka-hit', hr ? [hr] : []);
    if (h?.scroll && hr && boxRef.current) {
      h.scroll = false;
      const box = boxRef.current;
      const r = hr.getBoundingClientRect();
      const b = box.getBoundingClientRect();
      if (r.top < b.top + 40 || r.bottom > b.bottom - 40) box.scrollTop += r.top - b.top - box.clientHeight / 3;
    }
  }, [hash]);

  /** Где верх экрана: страница и доля её высоты. */
  const measure = useCallback((): Spot => {
    const v = viewer.current;
    const box = boxRef.current;
    if (!v || !box || !v.pagesCount) return cur.current;
    const top = box.getBoundingClientRect().top;
    const n0 = v.currentPageNumber;
    for (const k of [n0, n0 - 1, n0 + 1, n0 - 2, n0 + 2]) {
      const div = v.getPageView(k - 1)?.div as HTMLElement | undefined;
      if (!div) continue;
      const r = div.getBoundingClientRect();
      if (r.height > 0 && r.top <= top + 1 && r.bottom > top) return { page: k, frac: Math.max(0, Math.min(0.999, (top - r.top) / r.height)) };
    }
    return { page: n0, frac: 0 };
  }, []);

  /** Встать на место: страница сверху и ещё доля её высоты. */
  const goTo = useCallback((s: Spot) => {
    const v = viewer.current;
    const box = boxRef.current;
    if (!v || !box || !v.pagesCount) return;
    const n = Math.max(1, Math.min(v.pagesCount, s.page));
    v.scrollPageIntoView({ pageNumber: n });
    const div = v.getPageView(n - 1)?.div as HTMLElement | undefined;
    if (div && s.frac > 0) box.scrollTop += s.frac * div.getBoundingClientRect().height;
  }, []);

  /** Место — в журнал: не чаще раза в две минуты, при выходе и в конце книги — сразу. */
  const saveReading = useCallback(
    (force: boolean): Reading | null => {
      const v = bookRef.current;
      const s = session.current;
      if (!v?.title || !viewer.current?.pagesCount) return null;
      const now = Date.now();
      const sessionMin = Math.round(s.active / 60_000);
      const minutes = s.base + sessionMin;
      const { page: p, frac: f } = cur.current;
      const offset = Math.round(f * 10_000);
      const where = `${p - 1}:${offset}:${minutes}`;
      if (where === s.savedSpot) return null;
      if (!force && now - s.saved < 120_000) return null;
      s.saved = now;
      s.savedSpot = where;
      const year = String(new Date().getFullYear());
      const r: Reading = {
        title: v.title.id,
        book: hash,
        chapter: p - 1,
        offset,
        percent: Math.round(percentOf(cur.current) * 10) / 10,
        at: Math.floor(now / 1000),
        minutes,
        years: { ...s.years, [year]: (s.years[year] ?? 0) + sessionMin },
      };
      api.readingSave(r).catch(() => undefined);
      return r;
    },
    [hash, percentOf],
  );

  /** Прокрутили: запомнить место, посчитать время чтения. */
  const moved = useCallback(() => {
    const s = measure();
    const prev = cur.current;
    cur.current = s;
    setPage(s.page);
    setFrac(s.frac);
    const now = Date.now();
    const ses = session.current;
    ses.active += Math.min(now - ses.last, 3 * 60_000);
    ses.last = now;
    if (s.page !== prev.page || Math.abs(s.frac - prev.frac) > 0.04) {
      store(posKey(hash), JSON.stringify({ page: s.page, frac: Math.round(s.frac * 1000) / 1000, at: Math.floor(now / 1000) }));
      saveReading(false);
    }
  }, [hash, measure, saveReading]);
  const movedRef = useRef(moved);
  movedRef.current = moved;
  const paintRef = useRef(paint);
  paintRef.current = paint;

  const close = useCallback(() => onClose(saveReading(true)), [onClose, saveReading]);
  const remember = useCallback(() => setBack((b) => [...b.slice(-9), { ...cur.current }]), []);

  /** Масштаб и способ листания — из вида. */
  const applyLook = useCallback((l: PdfLook) => {
    const v = viewer.current;
    const m = modes.current;
    if (!v || !m || !v.pagesCount) return;
    const here = cur.current;
    v.scrollMode = l.flow === 'pages' ? m.scroll.PAGE : m.scroll.VERTICAL;
    v.spreadMode = l.spread && window.innerWidth >= 880 ? m.spread.ODD : m.spread.NONE;
    if (l.zoom) v.currentScale = l.zoom;
    else v.currentScaleValue = l.fit;
    goTo(here);
  }, [goTo]);

  // Открыть книгу: сведения из ядра, файл — в pdf.js.
  useEffect(() => {
    let alive = true;
    (async () => {
      const v = await api.bookOpen(hash);
      if (!alive) return;
      bookRef.current = v;
      if (v.title) navRef.current.update(v.title);
      setView(v);
      const [data, pdfjs] = await Promise.all([api.bookData(hash), loadPdf()]);
      if (!alive || !boxRef.current || !innerRef.current) return;
      const { lib, viewer: V } = pdfjs;
      modes.current = { scroll: { VERTICAL: V.ScrollMode.VERTICAL, PAGE: V.ScrollMode.PAGE }, spread: { NONE: V.SpreadMode.NONE, ODD: V.SpreadMode.ODD } };
      const loading = lib.getDocument(docOptions(new Uint8Array(data)));
      task.current = loading;
      let d: PDFDocumentProxy;
      try {
        d = await loading.promise;
      } catch (e) {
        throw new Error(pdfError(e));
      }
      if (!alive) return;
      doc.current = d;
      setPages(d.numPages);
      const eventBus = new V.EventBus();
      const linkService = new V.PDFLinkService({ eventBus });
      // Переход по ссылке внутри PDF — с возвратом «Назад».
      const goDest = linkService.goToDestination.bind(linkService);
      linkService.goToDestination = async (dest: unknown) => {
        remember();
        return goDest(dest as string);
      };
      const pdfViewer = new V.PDFViewer({
        container: boxRef.current,
        viewer: innerRef.current,
        eventBus,
        linkService,
        textLayerMode: 1,
        annotationMode: lib.AnnotationMode.ENABLE,
        removePageBorders: true,
        // Телефону — поменьше холст: большие страницы иначе съедают память.
        maxCanvasPixels: isMobileApp ? 2 ** 23 : 2 ** 25,
      });
      linkService.setViewer(pdfViewer);
      bus.current = eventBus;
      links.current = linkService;
      viewer.current = pdfViewer;
      eventBus.on('pagechanging', () => movedRef.current());
      eventBus.on('updateviewarea', () => movedRef.current());
      eventBus.on('scalechanging', (e: { scale: number }) => setScale(e.scale));
      eventBus.on('textlayerrendered', () => paintRef.current());
      const init = new Promise<void>((resolve) => eventBus.on('pagesinit', () => resolve(), { once: true }));
      pdfViewer.setDocument(d);
      linkService.setDocument(d, null);
      await init;
      if (!alive) return;
      applyLook(lookRef.current);

      // Где начать: своё место на этом устройстве или пришедшее с другого (оно новее).
      const local = localSpot(hash);
      let synced: Reading | undefined;
      if (v.title) {
        try {
          synced = (await api.readings()).find((r) => r.title === v.title?.id);
        } catch {
          /* без журнала — по своему месту */
        }
      }
      if (synced) {
        session.current.base = synced.minutes;
        session.current.years = { ...(synced.years ?? {}) };
        if (synced.book === hash) session.current.savedSpot = `${synced.chapter}:${synced.offset}:${synced.minutes}`;
        session.current.saved = Date.now();
      }
      let start: Spot = { page: 1, frac: 0 };
      let note: string | undefined;
      if (synced && (!local || synced.at > local.at + 30)) {
        if (synced.book === hash) {
          start = { page: synced.chapter + 1, frac: Math.min(0.999, synced.offset / 10_000) };
          if (synced.percent >= 0.5) note = 'Продолжаем с места, где читали на другом устройстве';
        } else {
          start = { page: Math.min(d.numPages, Math.floor((synced.percent / 100) * d.numPages) + 1), frac: 0 };
          note = `Продолжаем примерно с ${Math.round(synced.percent)} % — как на другом устройстве`;
        }
      } else if (local) {
        start = { page: local.page, frac: local.frac };
      }
      if (at) {
        start = { page: at.chapter + 1, frac: 0 };
        hitRef.current = { page: at.chapter + 1, offset: at.offset, len: at.len, scroll: true };
      }
      cur.current = start;
      goTo(start);
      setReady(true);
      if (note) navRef.current.toast(note);

      // Оглавление из файла: страницы находим уже в фоне.
      try {
        const raw = (await d.getOutline()) ?? [];
        const flat: OutlineItem[] = [];
        const walk = (items: typeof raw, depth: number) => {
          for (const it of items) {
            if (flat.length >= 3000) return;
            flat.push({ title: (it.title || '').replace(/\s+/g, ' ').trim() || 'Без названия', depth, dest: it.dest, url: it.url ?? null, page: null });
            if (it.items?.length) walk(it.items, depth + 1);
          }
        };
        walk(raw, 0);
        if (!alive) return;
        setOutline(flat);
        for (const it of flat) {
          try {
            const dest = typeof it.dest === 'string' ? await d.getDestination(it.dest) : it.dest;
            const ref = Array.isArray(dest) ? dest[0] : null;
            if (ref && typeof ref === 'object') it.page = (await d.getPageIndex(ref as { num: number; gen: number })) + 1;
            else if (Number.isInteger(ref)) it.page = (ref as number) + 1;
          } catch {
            /* у пункта нет страницы — просто без неё */
          }
        }
        if (alive) setOutline([...flat]);
      } catch {
        /* без оглавления */
      }

      // Записи без обложки — первая страница.
      if (v.title && !v.title.cover) {
        try {
          const url = await coverFromDoc(d);
          const updated = url ? await api.bookCover(hash, url) : null;
          if (updated && alive) navRef.current.update(updated);
        } catch {
          /* без обложки */
        }
      }
    })().catch((e) => alive && setError(message(e)));
    return () => {
      alive = false;
      highlight('polka-hit', []);
      highlight('polka-quote', []);
      viewer.current?.setDocument(null as unknown as PDFDocumentProxy);
      task.current?.destroy().catch(() => undefined);
      viewer.current = null;
      doc.current = null;
    };
    // Книга открывается один раз; место для начала берётся при открытии.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [hash]);

  // Вид: запомнить и применить на том же месте.
  useEffect(() => {
    store(LOOK_KEY, JSON.stringify(look));
    if (ready) applyLook(look);
  }, [look, ready, applyLook]);

  // Размер окна: «по ширине» и «целиком» пересчитывает сам pdf.js; две страницы — только на широком.
  useEffect(() => {
    const box = boxRef.current;
    if (!box || !ready) return;
    let frame = 0;
    const ro = new ResizeObserver(() => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => {
        const w = window.innerWidth >= 880;
        setWide(w);
        const v = viewer.current;
        if (v && !lookRef.current.zoom) v.currentScaleValue = lookRef.current.fit;
        if (v && modes.current) v.spreadMode = lookRef.current.spread && w ? modes.current.spread.ODD : modes.current.spread.NONE;
      });
    });
    ro.observe(box);
    return () => {
      ro.disconnect();
      cancelAnimationFrame(frame);
    };
  }, [ready]);

  useWakeLock(() => saveReading(true));

  // Закладки и цитаты этой записи.
  useEffect(() => {
    if (!view?.title) return;
    api
      .marks(view.title.id)
      .then(setMarks)
      .catch(() => undefined);
  }, [view]);
  useEffect(() => {
    if (ready) paint();
  }, [marks, ready, paint]);

  /** Масштаб в `factor` раз вокруг точки (по умолчанию — середина экрана). */
  const zoomBy = useCallback((factor: number, origin?: [number, number]) => {
    const v = viewer.current;
    const box = boxRef.current;
    if (!v || !box) return;
    const b = box.getBoundingClientRect();
    v.updateScale({ scaleFactor: factor, origin: origin ?? [b.left + b.width / 2, b.top + b.height / 2], drawingDelay: 250 });
    setLook((l) => ({ ...l, zoom: Math.round(v.currentScale * 1000) / 1000 }));
  }, []);

  // Щипок — масштаб; Ctrl+колесо на компьютере — тоже.
  useEffect(() => {
    const box = boxRef.current;
    if (!box || !ready) return;
    let pinch: { d: number; last: number } | null = null;
    const dist = (t: TouchList) => Math.hypot(t[0].clientX - t[1].clientX, t[0].clientY - t[1].clientY);
    const onStart = (e: TouchEvent) => {
      if (e.touches.length === 2) pinch = { d: dist(e.touches), last: 1 };
    };
    const onMove = (e: TouchEvent) => {
      const v = viewer.current;
      if (!pinch || e.touches.length !== 2 || !v) return;
      e.preventDefault();
      const factor = dist(e.touches) / pinch.d;
      const step = factor / pinch.last;
      if (Math.abs(step - 1) < 0.02) return;
      pinch.last = factor;
      v.updateScale({ scaleFactor: step, origin: [(e.touches[0].clientX + e.touches[1].clientX) / 2, (e.touches[0].clientY + e.touches[1].clientY) / 2], drawingDelay: 300 });
    };
    const onEnd = (e: TouchEvent) => {
      if (pinch && e.touches.length < 2) {
        pinch = null;
        const v = viewer.current;
        if (v) setLook((l) => ({ ...l, zoom: Math.round(v.currentScale * 1000) / 1000 }));
      }
    };
    const onWheel = (e: WheelEvent) => {
      if (!e.ctrlKey) return;
      e.preventDefault();
      zoomBy(e.deltaY < 0 ? 1.1 : 1 / 1.1, [e.clientX, e.clientY]);
    };
    box.addEventListener('touchstart', onStart, { passive: true });
    box.addEventListener('touchmove', onMove, { passive: false });
    box.addEventListener('touchend', onEnd);
    box.addEventListener('touchcancel', onEnd);
    box.addEventListener('wheel', onWheel, { passive: false });
    return () => {
      box.removeEventListener('touchstart', onStart);
      box.removeEventListener('touchmove', onMove);
      box.removeEventListener('touchend', onEnd);
      box.removeEventListener('touchcancel', onEnd);
      box.removeEventListener('wheel', onWheel);
    };
  }, [ready, zoomBy]);

  /** Дальше или назад: лентой — на экран, по страницам — на страницу; в самом конце — «Конец книги». */
  const turn = useCallback(
    (dir: 1 | -1) => {
      const v = viewer.current;
      const box = boxRef.current;
      if (!v || !box) return;
      highlight('polka-hit', []);
      hitRef.current = null;
      const atEnd = box.scrollTop + box.clientHeight >= box.scrollHeight - 4;
      if (lookRef.current.flow === 'pages') {
        // Увеличенная страница сначала прокручивается до края.
        if (dir > 0 && !atEnd) return box.scrollBy({ top: box.clientHeight * 0.88, behavior: 'smooth' });
        if (dir < 0 && box.scrollTop > 4) return box.scrollBy({ top: -box.clientHeight * 0.88, behavior: 'smooth' });
        const moved = dir > 0 ? v.nextPage() : v.previousPage();
        if (!moved && dir > 0) {
          saveReading(true);
          setPanel('end');
        } else if (moved && dir < 0) {
          requestAnimationFrame(() => (box.scrollTop = box.scrollHeight));
        }
        return;
      }
      if (dir > 0 && atEnd) {
        saveReading(true);
        setPanel('end');
        return;
      }
      box.scrollBy({ top: dir * box.clientHeight * 0.88, behavior: 'smooth' });
    },
    [saveReading],
  );

  const jumpTo = useCallback(
    (s: Spot, hit?: { offset: number; len: number }) => {
      remember();
      setPanel(null);
      setBars(false);
      hitRef.current = hit ? { page: s.page, ...hit, scroll: true } : null;
      goTo(s);
      paint();
    },
    [goTo, paint, remember],
  );

  // Клавиши: стрелки, PageUp/PageDown, пробел, Home/End, Ctrl +/−; Escape закрывает панель или книгу.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement) {
        if (e.key === 'Escape') setPanel(null);
        return;
      }
      if (e.key === 'Escape') {
        if (panel) setPanel(null);
        else if (bars) setBars(false);
        else close();
        return;
      }
      if (panel) return;
      const v = viewer.current;
      if ((e.ctrlKey || e.metaKey) && (e.key === '+' || e.key === '=' || e.key === '-')) {
        e.preventDefault();
        zoomBy(e.key === '-' ? 1 / 1.15 : 1.15);
      } else if (e.key === 'PageDown' || (e.key === ' ' && !e.shiftKey)) {
        e.preventDefault();
        turn(1);
      } else if (e.key === 'PageUp' || (e.key === ' ' && e.shiftKey)) {
        e.preventDefault();
        turn(-1);
      } else if (e.key === 'ArrowRight' && v) {
        e.preventDefault();
        if (!v.nextPage()) setPanel('end');
      } else if (e.key === 'ArrowLeft' && v) {
        e.preventDefault();
        v.previousPage();
      } else if (e.key === 'Home' && v) {
        e.preventDefault();
        goTo({ page: 1, frac: 0 });
      } else if (e.key === 'End' && v) {
        e.preventDefault();
        goTo({ page: v.pagesCount, frac: 0 });
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [turn, panel, bars, close, zoomBy, goTo]);

  // Выделили текст — кнопка «В цитаты» под выделением.
  useEffect(() => {
    if (!ready) return;
    let timer = 0;
    const onSelection = () => {
      window.clearTimeout(timer);
      timer = window.setTimeout(() => {
        const sel = window.getSelection();
        const inner = innerRef.current;
        if (!sel || sel.isCollapsed || !sel.rangeCount || !inner || !inner.contains(sel.getRangeAt(0).commonAncestorContainer)) {
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

  useAndroidBack(() => {
    if (panel) setPanel(null);
    else if (bars) setBars(false);
    else close();
  });

  const onBoxClick = (e: ReactMouseEvent<HTMLDivElement>) => {
    const target = e.target as HTMLElement;
    const a = target.closest('a') as HTMLAnchorElement | null;
    if (a) {
      // Внешние ссылки — в браузере; внутренние pdf.js открывает сам.
      const href = a.getAttribute('href') ?? '';
      if (/^(https?:|mailto:)/i.test(href)) {
        e.preventDefault();
        api.openLink(href).catch((err) => nav.toast(message(err)));
      }
      return;
    }
    if (skipClick.current) {
      skipClick.current = false;
      return;
    }
    const sel = window.getSelection();
    if (sel && !sel.isCollapsed) return;
    if (hadSelection.current) {
      hadSelection.current = false;
      setSelBar(null);
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
  // По страницам: свайп влево и вправо листает (если страница не увеличена шире экрана).
  const onTouchStart = (e: ReactTouchEvent) => {
    swipe.current = e.touches.length === 1 ? { x: e.touches[0].clientX, y: e.touches[0].clientY, two: false } : swipe.current && { ...swipe.current, two: true };
  };
  const onTouchEnd = (e: ReactTouchEvent) => {
    const s = swipe.current;
    swipe.current = null;
    const box = boxRef.current;
    if (!s || s.two || !box || lookRef.current.flow !== 'pages' || box.scrollWidth > box.clientWidth + 2) return;
    const sel = window.getSelection();
    if (sel && !sel.isCollapsed) return;
    const dx = e.changedTouches[0].clientX - s.x;
    const dy = e.changedTouches[0].clientY - s.y;
    if (Math.abs(dx) > 50 && Math.abs(dx) > Math.abs(dy) * 1.3) {
      skipClick.current = true;
      setTimeout(() => (skipClick.current = false), 400);
      const v = viewer.current;
      if (dx < 0 && v && !v.nextPage()) setPanel('end');
      if (dx > 0) v?.previousPage();
    }
  };
  // По страницам: колесо у края страницы листает дальше.
  const onWheel = (e: ReactWheelEvent) => {
    const box = boxRef.current;
    if (e.ctrlKey || !box || lookRef.current.flow !== 'pages' || panel) return;
    const now = Date.now();
    const down = e.deltaY > 0;
    const edge = down ? box.scrollTop + box.clientHeight >= box.scrollHeight - 4 : box.scrollTop <= 4;
    if (!edge || Math.abs(e.deltaY) < 6 || now - wheelAt.current < 350) return;
    wheelAt.current = now;
    turn(down ? 1 : -1);
  };

  const here = useMemo(() => marks.find((m) => m.kind === 'bookmark' && m.book === hash && m.chapter === page - 1) ?? null, [marks, hash, page]);

  const toggleBookmark = async () => {
    const v = bookRef.current;
    if (!v?.title) return;
    try {
      if (here) {
        await api.markDelete(here.id);
        setMarks((l) => l.filter((m) => m.id !== here.id));
        nav.toast('Закладка убрана');
        return;
      }
      const s = cur.current;
      const text = (await textOf(s.page)).shown.slice(0, 140).trim() || `Страница ${s.page}`;
      const m = await api.markSave({ id: '', title: v.title.id, book: hash, kind: 'bookmark', chapter: s.page - 1, offset: Math.round(s.frac * 10_000), end: 0, text, note: '', percent: Math.round(percentOf(s) * 10) / 10, created: 0 });
      setMarks((l) => [...l, m]);
      nav.toast('Закладка поставлена');
    } catch (e) {
      nav.toast(message(e));
    }
  };

  /** Выделение: страница и знаки от начала её текста. */
  const selectionSpot = () => {
    const sel = selRange.current;
    if (!sel) return null;
    const node = sel.range.startContainer;
    const el = node instanceof Element ? node : node.parentElement;
    const layer = el?.closest('.textLayer');
    const pageEl = el?.closest('.page') as HTMLElement | null;
    if (!layer || !pageEl) return null;
    const n = Number(pageEl.dataset.pageNumber);
    const offset = offsetIn(layer, node, sel.range.startOffset);
    const total = layer.textContent?.length ?? 0;
    const end = Math.min(total, layer.contains(sel.range.endContainer) ? offsetIn(layer, sel.range.endContainer, sel.range.endOffset) : offset + sel.range.toString().length);
    const text = sel.text.replace(/[ \t ]+/g, ' ').replace(/\s*\n\s*/g, ' ').trim();
    return Number.isInteger(n) && end > offset && text ? { page: n, offset, end, text } : null;
  };
  const dropSelection = () => {
    window.getSelection()?.removeAllRanges();
    selRange.current = null;
    setSelBar(null);
  };
  const saveQuote = async () => {
    const place = selectionSpot();
    const v = bookRef.current;
    if (!place || !v?.title) return dropSelection();
    try {
      const m = await api.markSave({ id: '', title: v.title.id, book: hash, kind: 'quote', chapter: place.page - 1, offset: place.offset, end: place.end, text: place.text.slice(0, 4000), note: '', percent: Math.round(percentOf({ page: place.page, frac: 0 }) * 10) / 10, created: 0 });
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
    const q = place.text.slice(0, 200);
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
    const d = doc.current;
    const nq = normQuery(q);
    if (!d || nq.length < 2) return;
    const token = ++searchToken.current;
    const out: PdfHit[] = [];
    let anyText = false;
    setHits(null);
    setNoText(false);
    setSearching({ done: 0, total: d.numPages });
    for (let n = 1; n <= d.numPages && out.length < 300; n++) {
      const pt = await textOf(n);
      if (token !== searchToken.current) return;
      if (pt.norm) anyText = true;
      out.push(...findInPage(pt, n, nq, 300 - out.length));
      if (n % 20 === 0) {
        setSearching({ done: n, total: d.numPages });
        setHits([...out]);
      }
    }
    setHits(out);
    setSearching(null);
    setNoText(!anyText);
  };
  const runSearch = async (e: FormEvent) => {
    e.preventDefault();
    await find(query);
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
  const total = pages || view?.total || 0;
  const pageAt = (percent: number) => Math.max(1, Math.min(total, Math.floor((percent / 100) * total) + 1));
  const commitScrub = () => {
    if (scrub === null) return;
    const n = pageAt(scrub);
    setScrub(null);
    jumpTo({ page: n, frac: 0 });
  };
  const goToTyped = (e: FormEvent) => {
    e.preventDefault();
    const n = Number(goPage);
    if (Number.isInteger(n) && n >= 1 && n <= total) {
      setGoPage('');
      jumpTo({ page: n, frac: 0 });
    }
  };
  const set = (patch: Partial<PdfLook>) => setLook((l) => ({ ...l, ...patch }));

  const byPlace = (a: Mark, b: Mark) => a.chapter - b.chapter || a.offset - b.offset;
  const bookmarks = marks.filter((m) => m.kind === 'bookmark' && m.book === hash).sort(byPlace);
  const quotes = marks.filter((m) => m.kind === 'quote' && m.book === hash).sort(byPlace);
  const themeId = resolveTheme(look.theme);
  const theme = THEMES[themeId];
  const style = {
    '--rd-bg': theme.bg,
    '--rd-ink': theme.ink,
    '--rd-muted': theme.muted,
    '--rd-link': theme.link,
    '--rd-line': theme.line,
    '--rd-hit': theme.hit,
    '--rd-bar': theme.bar,
  } as CSSProperties;

  const shownPage = scrub !== null ? pageAt(scrub) : page;
  const percent = percentOf({ page, frac });
  const shown = scrub ?? percent;
  const left = timeLeft((Math.max(0, total - (page - 1 + frac)) * CHARS_PER_PAGE) / readSpeed());
  const section = useMemo(() => [...outline].reverse().find((it) => it.page !== null && it.page <= shownPage), [outline, shownPage]);
  const sectionTitle = section?.title ?? '';
  const where = (m: Mark) => `Стр. ${m.chapter + 1} · ${pct(m.percent)}`;

  return (
    <div className={`reader pdf-reader theme-${themeId}`} style={style}>
      <div className="pdf-wrap">
        <div className={`pdf-box${look.flow === 'pages' ? ' pdf-paged' : ''}`} ref={boxRef} onPointerDown={onPointerDown} onClick={onBoxClick} onTouchStart={onTouchStart} onTouchEnd={onTouchEnd} onWheel={onWheel}>
          <div ref={innerRef} className="pdfViewer" />
        </div>
      </div>
      {here && !bars && (
        <svg className="rd-ribbon" viewBox="0 0 24 24" aria-label="Закладка на этой странице" role="img">
          <path d="M19 21l-7-4-7 4V5a2 2 0 0 1 2-2h10a2 2 0 0 1 2 2Z" />
        </svg>
      )}
      <div className="rd-status" aria-hidden={bars}>
        <span className="rd-status-title">{sectionTitle}</span>
        <span className="num">{ready ? `${page} / ${total} · ${pct(percent)}` : ''}</span>
      </div>

      {!ready && !error && <div className="rd-wait">Открываю PDF…</div>}
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
            <b>{t?.title || view.meta.title || 'PDF'}</b>
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
          <p className="rd-chapter">{sectionTitle}</p>
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
            <span>{`Стр. ${shownPage} из ${total}`}</span>
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
            hitRef.current = null;
            goTo(last);
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
              empty={tab === 'bookmarks' ? 'Закладок пока нет. Поставить — значок закладки вверху, когда открыты панели.' : 'Цитат пока нет. Выделите текст на странице и нажмите «В цитаты».'}
              where={where}
              confirmId={confirmId}
              onOpen={(m) => (m.kind === 'quote' ? jumpTo({ page: m.chapter + 1, frac: 0 }, { offset: m.offset, len: m.end - m.offset }) : jumpTo({ page: m.chapter + 1, frac: m.offset / 10_000 }))}
              onShare={(m) => setQuotePic(m)}
              onRemove={removeMark}
            />
          ) : (
            <>
              <form className="rd-search pdf-goto" onSubmit={goToTyped}>
                <input type="number" inputMode="numeric" min={1} max={total} value={goPage} placeholder={`Страница 1–${total}`} aria-label="Номер страницы" onChange={(e) => setGoPage(e.target.value)} />
                <button type="submit" className="btn soft small" disabled={!goPage}>
                  Перейти
                </button>
              </form>
              {outline.length ? (
                <ol className="rd-toc">
                  {outline.map((it, i) => (
                    <li key={i}>
                      <button
                        type="button"
                        className={it === section ? 'on' : ''}
                        style={{ paddingLeft: 12 + Math.min(it.depth, 4) * 16 }}
                        onClick={() => {
                          if (it.url) return api.openLink(it.url).catch((e) => nav.toast(message(e)));
                          if (it.page) return jumpTo({ page: it.page, frac: 0 });
                          if (it.dest) {
                            setPanel(null);
                            setBars(false);
                            links.current?.goToDestination(it.dest as string);
                          }
                        }}
                      >
                        <span>{it.title}</span>
                        {it.page !== null && <span className="small muted num">{it.page}</span>}
                      </button>
                    </li>
                  ))}
                </ol>
              ) : (
                <p className="muted">В PDF нет оглавления — можно перейти на страницу по номеру.</p>
              )}
            </>
          )}
        </Sheet>
      )}

      {panel === 'search' && (
        <Sheet title="Поиск по книге" side onClose={() => setPanel(null)}>
          <form className="rd-search" onSubmit={runSearch}>
            <input type="search" value={query} autoFocus placeholder="Слово или фраза" aria-label="Что найти" onChange={(e) => setQuery(e.target.value)} maxLength={200} />
            <button type="submit" className="btn primary small" disabled={!!searching || query.trim().length < 2}>
              Найти
            </button>
          </form>
          {searching && <p className="muted">Ищу… страница {searching.done} из {searching.total}</p>}
          {noText && !searching && <p className="small muted">В этом PDF нет текста — похоже, это сканы страниц. Искать и выделять цитаты в нём не получится.</p>}
          {hits && !searching && !noText && <p className="small muted">{hits.length ? `${plural(hits.length, 'место', 'места', 'мест')}${hits.length >= 300 ? ' (первые)' : ''}` : 'Ничего не нашлось'}</p>}
          {hits && hits.length > 0 && (
            <ul className="rd-hits">
              {hits.map((h, i) => (
                <li key={i}>
                  <button type="button" onClick={() => jumpTo({ page: h.page, frac: 0 }, { offset: h.offset, len: h.len })}>
                    <span className="small muted">Стр. {h.page}</span>
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
              <button type="button" className="btn soft" aria-label="Мельче" disabled={scale <= 0.3} onClick={() => zoomBy(1 / 1.15)}>
                <Icon name="minus" />
              </button>
              <span className="num">{Math.round(scale * 100)} %</span>
              <button type="button" className="btn soft" aria-label="Крупнее" disabled={scale >= 5} onClick={() => zoomBy(1.15)}>
                <Icon name="plus" />
              </button>
            </div>
            <Seg
              cols
              small
              label="Масштаб"
              value={look.zoom ? 'custom' : look.fit}
              options={[
                ['auto', 'Авто'],
                ['page-width', 'По ширине'],
                ['page-fit', 'Страница целиком'],
              ]}
              onChange={(fit) => set({ fit: fit as Fit, zoom: null })}
            />
            <Seg
              cols
              small
              label="Листать"
              value={look.flow}
              options={[
                ['scroll', 'Лентой'],
                ['pages', 'По страницам'],
              ]}
              onChange={(flow) => set({ flow })}
            />
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
            <p className="small muted">В тёмной теме страницы PDF показываются в обратных цветах, в сепии — теплее.</p>
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
          <p>Вы дочитали «{t?.title || view.meta.title || 'PDF'}».</p>
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
