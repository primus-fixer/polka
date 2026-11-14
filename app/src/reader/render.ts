// Глава из ядра — дерево [тег, атрибуты, …дети] — в узлы страницы.
// Всё строится через createElement: разметка книги не попадает в innerHTML, теги и атрибуты —
// только из списков ниже (ядро уже отобрало их, здесь — вторая проверка).
import type { BookNode } from '../api';

const TAGS = new Set([
  'p', 'div', 'span', 'h1', 'h2', 'h3', 'h4', 'h5', 'h6', 'blockquote', 'pre', 'code', 'em', 'i', 'strong', 'b', 'u', 's', 'sub', 'sup',
  'small', 'br', 'hr', 'img', 'a', 'ul', 'ol', 'li', 'dl', 'dt', 'dd', 'table', 'thead', 'tbody', 'tfoot', 'tr', 'th', 'td', 'caption',
  'figure', 'figcaption', 'section', 'aside', 'cite', 'q', 'abbr', 'mark', 'ruby', 'rt', 'rp',
]);
const BLOCKS = new Set(['p', 'div', 'h1', 'h2', 'h3', 'h4', 'h5', 'h6', 'blockquote', 'pre', 'li', 'dd', 'dt', 'section', 'aside', 'figure', 'td', 'th', 'table', 'ul', 'ol']);
/** Классы разметки FB2 (эпиграф, стихи…): на странице — с приставкой bk-. */
const CLASSES = new Set(['gap', 'epigraph', 'cite', 'poem', 'stanza', 'verse', 'author', 'date', 'subtitle', 'annotation', 'image']);
const STYLE = new Set([
  'font-style', 'font-weight', 'font-variant', 'text-align', 'text-decoration', 'vertical-align', 'text-transform', 'display', 'white-space',
  'break-before', 'break-after', 'break-inside', 'font-size', 'text-indent', 'margin-left', 'margin-right', 'margin-top', 'margin-bottom',
]);
const SAFE_VALUE = /^[a-z0-9 .%-]{1,40}$/i;

function applyStyle(el: HTMLElement, css: string) {
  for (const decl of css.split(';')) {
    const at = decl.indexOf(':');
    if (at < 0) continue;
    const prop = decl.slice(0, at).trim();
    const value = decl.slice(at + 1).trim();
    if (STYLE.has(prop) && SAFE_VALUE.test(value)) el.style.setProperty(prop, value);
  }
}

function append(parent: Node, n: BookNode, image: (src: string) => string, depth: number) {
  if (typeof n === 'string') {
    parent.appendChild(document.createTextNode(n));
    return;
  }
  if (!Array.isArray(n) || depth > 250) return;
  const [tag, attrs, ...kids] = n;
  const name = typeof tag === 'string' && TAGS.has(tag) ? tag : 'span';
  const el = document.createElement(name);
  if (attrs && typeof attrs === 'object') {
    for (const [k, v] of Object.entries(attrs)) {
      if (typeof v !== 'string') continue;
      switch (k) {
        case 'id':
          el.dataset.a = v;
          break;
        case 'class':
          if (CLASSES.has(v)) el.className = `bk-${v}`;
          break;
        case 'style':
          applyStyle(el, v);
          break;
        case 'src':
          if (el instanceof HTMLImageElement) {
            el.decoding = 'async';
            el.src = image(v);
          }
          break;
        case 'alt':
          if (el instanceof HTMLImageElement) el.alt = v;
          break;
        case 'to':
          el.dataset.to = v;
          el.classList.add('bk-link');
          break;
        case 'ext':
          if (/^(https?:\/\/|mailto:)/i.test(v)) {
            el.dataset.ext = v;
            el.title = v;
            el.classList.add('bk-link');
          }
          break;
        case 'note':
          el.dataset.note = '1';
          break;
        case 'colspan':
        case 'rowspan':
          if (/^\d{1,2}$/.test(v)) el.setAttribute(k, v);
          break;
        case 'start':
          if (/^-?\d{1,6}$/.test(v)) el.setAttribute('start', v);
          break;
      }
    }
  }
  for (const k of kids) append(el, k as BookNode, image, depth + 1);
  parent.appendChild(el);
}

/** Узлы главы для страницы; `image` — адрес картинки по пути внутри книги. */
export function build(nodes: BookNode[], image: (src: string) => string): DocumentFragment {
  const frag = document.createDocumentFragment();
  for (const n of nodes) append(frag, n, image, 0);
  return frag;
}

export function textOf(nodes: BookNode[]): string {
  let s = '';
  for (const n of nodes) {
    if (typeof n === 'string') s += n;
    else if (Array.isArray(n)) s += textOf(n.slice(2) as BookNode[]);
  }
  return s;
}

/** Сноска по метке: сам блок или ближайший блок вокруг строчной метки. */
export function findNote(nodes: BookNode[], id: string): BookNode[] | null {
  const path: BookNode[] = [];
  const walk = (list: BookNode[]): BookNode | null => {
    for (const n of list) {
      if (typeof n === 'string' || !Array.isArray(n)) continue;
      if (n[1]?.id === id) return n;
      path.push(n);
      const found = walk(n.slice(2) as BookNode[]);
      if (found) return found;
      path.pop();
    }
    return null;
  };
  const hit = walk(nodes);
  if (!hit || typeof hit === 'string') return null;
  if (BLOCKS.has(hit[0]) && textOf([hit]).trim()) return [hit];
  for (let i = path.length - 1; i >= 0; i--) {
    const a = path[i];
    if (typeof a !== 'string' && BLOCKS.has(a[0]) && textOf([a]).trim()) return [a];
  }
  return textOf([hit]).trim() ? [hit] : null;
}
