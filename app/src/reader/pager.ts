// Листание главы колонками CSS: страница — колонка (или две рядом на широком экране).
// Место в главе — номер знака от начала текста (UTF-16, как в ядре): после смены шрифта или
// размера окна по нему находится та же страница.

export class Pager {
  /** Ширина и высота страницы, промежуток между колонками. */
  private w = 0;
  private h = 0;
  private gap = 0;
  per = 1;
  pages = 1;
  page = 0;
  private texts: Text[] = [];
  private starts: number[] = [];
  /** Знаков текста в главе. */
  length = 0;
  private range = document.createRange();

  constructor(
    private view: HTMLElement,
    private flow: HTMLElement,
  ) {}

  /** Текстовые узлы главы и их начала — после каждой новой главы. */
  index() {
    this.texts = [];
    this.starts = [];
    let at = 0;
    const walker = document.createTreeWalker(this.flow, NodeFilter.SHOW_TEXT);
    for (let n = walker.nextNode(); n; n = walker.nextNode()) {
      const t = n as Text;
      this.texts.push(t);
      this.starts.push(at);
      at += t.length;
    }
    this.length = at;
  }

  private get step(): number {
    return this.w + this.gap;
  }

  /** Раскладка по колонкам под текущий размер окна. */
  layout(spread: boolean) {
    const w = Math.max(1, Math.floor(this.view.clientWidth));
    const h = Math.max(1, Math.floor(this.view.clientHeight));
    const per = spread && w >= 880 ? 2 : 1;
    const gap = per === 2 ? Math.max(48, Math.round(w * 0.07)) : Math.max(32, Math.round(w * 0.08));
    const col = per === 2 ? (w - gap) / 2 : w;
    const s = this.flow.style;
    s.width = `${w}px`;
    s.height = `${h}px`;
    s.columnWidth = `${col}px`;
    s.columnCount = String(per);
    s.columnGap = `${gap}px`;
    s.setProperty('--rd-page-h', `${h}px`);
    this.w = w;
    this.h = h;
    this.gap = gap;
    this.per = per;
    // Сколько страниц: по пустому блоку в конце главы, без него — по ширине содержимого.
    const left = this.flow.getBoundingClientRect().left;
    const rects = this.flow.querySelector('.rd-end')?.getClientRects();
    this.pages =
      rects && rects.length
        ? Math.max(1, Math.floor((rects[rects.length - 1].left - left + 1) / this.step) + 1)
        : Math.max(1, Math.ceil(Math.round((this.flow.scrollWidth + gap) / (col + gap)) / per));
    this.page = Math.min(this.page, this.pages - 1);
  }

  /** Показать страницу; `animate` — листать плавно. */
  go(page: number, animate = false) {
    this.page = Math.max(0, Math.min(this.pages - 1, page));
    this.flow.classList.toggle('anim', animate);
    this.flow.style.transform = `translate3d(${-this.page * this.step}px, 0, 0)`;
  }

  /** Палец тянет страницу: сдвиг без анимации. */
  drag(dx: number) {
    this.flow.classList.remove('anim');
    this.flow.style.transform = `translate3d(${-this.page * this.step + dx}px, 0, 0)`;
  }

  private pageOfRect(r: DOMRect): number {
    const left = this.flow.getBoundingClientRect().left;
    return Math.max(0, Math.min(this.pages - 1, Math.floor((r.left - left + 1) / this.step)));
  }

  private charRect(t: Text, i: number): DOMRect | null {
    if (i < 0 || i >= t.length) return null;
    this.range.setStart(t, i);
    this.range.setEnd(t, i + 1);
    const rects = this.range.getClientRects();
    for (let k = 0; k < rects.length; k++) {
      const r = rects[k];
      if (r.width > 0 || r.height > 0) return r;
    }
    return null;
  }

  /** Страница первого видимого знака в узле `i` с позиции `from` или дальше (схлопнутые пробелы не видны). */
  private pageFrom(i: number, from: number): number | null {
    for (let k = i; k < this.texts.length && k < i + 60; k++) {
      const t = this.texts[k];
      const start = k === i ? from : 0;
      for (let j = start; j < t.length && j < start + 24; j++) {
        const r = this.charRect(t, j);
        if (r) return this.pageOfRect(r);
      }
    }
    return null;
  }

  /** Узел, в котором знак `off`. */
  private nodeAt(off: number): number {
    let lo = 0;
    let hi = this.texts.length - 1;
    while (lo < hi) {
      const mid = (lo + hi + 1) >> 1;
      if (this.starts[mid] <= off) lo = mid;
      else hi = mid - 1;
    }
    return lo;
  }

  /** Страница, на которой знак `off`. */
  pageOfOffset(off: number): number {
    if (!this.texts.length || off <= 0) return 0;
    if (off >= this.length) return this.pages - 1;
    const i = this.nodeAt(off);
    return this.pageFrom(i, off - this.starts[i]) ?? 0;
  }

  /** Первый знак страницы `p`: место, которое запоминается. */
  offsetOfPage(p: number): number {
    if (p <= 0 || !this.texts.length) return 0;
    const n = this.texts.length;
    // Первый узел, чей первый видимый знак — на странице p или дальше.
    let lo = 0;
    let hi = n;
    while (lo < hi) {
      const mid = (lo + hi) >> 1;
      const pg = this.pageFrom(mid, 0);
      if (pg === null || pg >= p) hi = mid;
      else lo = mid + 1;
    }
    // Начало страницы может быть внутри предыдущего узла.
    const prev = lo - 1;
    if (prev >= 0) {
      const t = this.texts[prev];
      let a = 0;
      let b = t.length;
      while (a < b) {
        const m = (a + b) >> 1;
        const pg = this.pageFrom(prev, m);
        if (pg === null || pg >= p) b = m;
        else a = m + 1;
      }
      if (a < t.length) return this.starts[prev] + a;
    }
    return lo < n ? this.starts[lo] : this.length;
  }

  /** Страница метки (`id` из книги); null — такой метки в главе нет. */
  pageOfAnchor(id: string): number | null {
    const el = this.flow.querySelector(`[data-a="${CSS.escape(id)}"]`);
    if (!el) return null;
    let rects = el.getClientRects();
    if (!rects.length) {
      this.range.selectNode(el);
      rects = this.range.getClientRects();
    }
    return rects.length ? this.pageOfRect(rects[0]) : null;
  }

  /** Диапазон текста от знака `off` длиной `len` — для подсветки найденного. */
  rangeAt(off: number, len: number): Range | null {
    if (!this.texts.length || off < 0 || off >= this.length) return null;
    const a = this.nodeAt(off);
    const b = this.nodeAt(Math.min(this.length - 1, off + Math.max(1, len) - 1));
    const r = document.createRange();
    r.setStart(this.texts[a], off - this.starts[a]);
    r.setEnd(this.texts[b], Math.min(this.texts[b].length, off + len - this.starts[b]));
    return r;
  }

  /** Текст главы от знака `off`, не длиннее `len` — подпись закладки. Абзацы — через пробел. */
  textFrom(off: number, len: number): string {
    if (!this.texts.length || off >= this.length) return '';
    let i = this.nodeAt(Math.max(0, off));
    let s = this.texts[i].data.slice(Math.max(0, off - this.starts[i]));
    const block = (t: Text) => t.parentElement?.closest('p,div,h1,h2,h3,h4,h5,h6,li,blockquote,section,aside,td,th,dd,dt,figcaption,pre');
    while (s.length < len * 2 && i + 1 < this.texts.length) {
      const next = this.texts[i + 1];
      s += (block(next) !== block(this.texts[i]) ? ' ' : '') + next.data;
      i++;
    }
    const clean = s.replace(/\s+/g, ' ').trim();
    return clean.length > len ? `${clean.slice(0, len).trimEnd()}…` : clean;
  }

  /** Сколько страниц (колонок) видно сразу и высота страницы — для подписей. */
  size(): { w: number; h: number } {
    return { w: this.w, h: this.h };
  }
}
