import { useState, type CSSProperties, type ReactNode } from 'react';
import { coverSrc, type Kind, type Title } from './api';
import { ICONS } from './icons';
import { store, stored } from './storage';

export function Icon({ name, size = 20, stroke = 1.75, style }: { name: string; size?: number; stroke?: number; style?: CSSProperties }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={stroke} strokeLinecap="round" strokeLinejoin="round" aria-hidden="true" style={style}>
      <path d={ICONS[name] ?? ''} />
    </svg>
  );
}

// Обложка-заглушка: цвет — от названия, чтобы у книги он всегда был один и тот же.
const PALETTE: [string, string][] = [
  ['#9A4A2C', '#FFF4EA'],
  ['#1F3B5C', '#E8F0F7'],
  ['#C79A2B', '#1E1608'],
  ['#2B2B2B', '#F2F2F2'],
  ['#4A5560', '#F3F5F7'],
  ['#E6D9BC', '#2A2316'],
  ['#B9733A', '#1E1208'],
  ['#5E3F6E', '#F5EEF8'],
  ['#4E6B3A', '#EEF3E8'],
  ['#24555E', '#E8F4F2'],
  ['#7A2E46', '#FBEFF3'],
  ['#A9C6D8', '#14232E'],
  ['#3E5A6E', '#EEF4F8'],
  ['#B03A2E', '#FFF1EC'],
];

export function palette(key: string): [string, string] {
  let h = 0;
  for (const ch of key) h = (h * 31 + (ch.codePointAt(0) ?? 0)) >>> 0;
  return PALETTE[h % PALETTE.length];
}

export function kindIcon(kind: Kind): string {
  return kind === 'book' ? 'book' : 'film';
}

/** Кто под названием: фамилия автора у книги, год у кино. */
export function shortBy(t: Pick<Title, 'kind' | 'creators' | 'year'>): string {
  if (t.kind === 'book') {
    const a = t.creators[0] ?? '';
    if (t.creators.length > 1) {
      const last = a.split(' ').pop() ?? a;
      return t.creators.every((c) => c.split(' ').pop()?.slice(0, 5) === last.slice(0, 5)) ? `${last.replace(/ий$/, 'ие')}` : `${last} и др.`;
    }
    return a;
  }
  return t.year ?? '';
}

type CoverProps = { t: Pick<Title, 'title' | 'kind' | 'creators' | 'year' | 'cover' | 'rating'>; size?: 'grid' | 'hero' | 'mini' | 'pick'; badge?: boolean; src?: string | null };

export function Cover({ t, size = 'grid', badge = false, src }: CoverProps) {
  const [broken, setBroken] = useState(false);
  const url = src ?? (t.cover ? coverSrc(t.cover) : null);
  const rating = badge && t.rating ? <span className="badge num">{t.rating}</span> : null;
  if (url && !broken) {
    return (
      <span className={`cover ${size}`}>
        <img src={url} alt="" loading="lazy" decoding="async" onError={() => setBroken(true)} />
        {rating}
      </span>
    );
  }
  const [bg, fg] = palette(t.title);
  if (size === 'mini') {
    return (
      <span className="cover mini ph" style={{ background: bg, color: fg }}>
        <Icon name={kindIcon(t.kind)} size={16} style={{ opacity: 0.75 }} />
      </span>
    );
  }
  return (
    <span className={`cover ph ${size}`} style={{ background: bg, color: fg }}>
      <span className="ph-line" />
      <span className="ph-title">{t.title}</span>
      <span className="ph-by">{shortBy(t)}</span>
      {rating}
    </span>
  );
}

export function Seg<T extends string>({ value, options, onChange, label, cols, small }: { value: T; options: [T, string][]; onChange: (v: T) => void; label: string; cols?: boolean; small?: boolean }) {
  return (
    <div
      className={`seg${small ? ' small' : ''}`}
      role="group"
      aria-label={label}
      style={cols ? { gridTemplateColumns: `repeat(${options.length}, minmax(max-content, 1fr))` } : { gridAutoFlow: 'column', gridAutoColumns: 'auto' }}
    >
      {options.map(([v, l]) => (
        <button key={v} type="button" className={v === value ? 'on' : ''} aria-pressed={v === value} onClick={() => onChange(v)}>
          {l}
        </button>
      ))}
    </div>
  );
}

export function Bar({ pct, color = 'var(--own)', thick }: { pct: number; color?: string; thick?: boolean }) {
  return (
    <span className={`bar${thick ? ' thick' : ''}`}>
      <span style={{ width: `${Math.max(0, Math.min(100, pct))}%`, background: color }} />
    </span>
  );
}

export function RatingPicker({ value, onChange }: { value: number | null | undefined; onChange: (v: number | null) => void }) {
  return (
    <div className="ratings" role="group" aria-label="Оценка от 1 до 10">
      {Array.from({ length: 10 }, (_, i) => i + 1).map((n) => (
        <button key={n} type="button" className={`num${value === n ? ' on' : ''}`} aria-pressed={value === n} aria-label={`Оценка ${n}`} onClick={() => onChange(value === n ? null : n)}>
          {n}
        </button>
      ))}
    </div>
  );
}

export function TagPicker({ value, known, onChange, placeholder = '+ метка', addLabel = 'Новая метка' }: { value: string[]; known: string[]; onChange: (v: string[]) => void; placeholder?: string; addLabel?: string }) {
  const [draft, setDraft] = useState('');
  const same = (a: string, b: string) => a.toLowerCase() === b.toLowerCase();
  const isOn = (t: string) => value.some((v) => same(v, t));
  // Порядок не прыгает при выборе; своё, которого нет среди известных, — в конце.
  const all = [...known, ...value.filter((v) => !known.some((k) => same(k, v)))];
  const toggle = (t: string) => onChange(isOn(t) ? value.filter((x) => !same(x, t)) : [...value, t]);
  const add = () => {
    const t = draft.trim();
    if (t && !isOn(t)) onChange([...value, t]);
    setDraft('');
  };
  return (
    <div className="chips">
      {all.map((t) => (
        <button key={t} type="button" className={`chip${isOn(t) ? ' sel' : ''}`} aria-pressed={isOn(t)} onClick={() => toggle(t)}>
          {t}
        </button>
      ))}
      <form
        className="chip-add"
        onSubmit={(e) => {
          e.preventDefault();
          add();
        }}
      >
        <input value={draft} onChange={(e) => setDraft(e.target.value)} onBlur={add} placeholder={placeholder} aria-label={addLabel} maxLength={40} />
      </form>
    </div>
  );
}

/** Раздел под катом: заголовок — кнопка, содержимое — по нажатию; открытые разделы запоминаются на устройстве. */
export function Cut({ id, title, hint, className, children }: { id: string; title: string; hint?: string; className?: string; children: ReactNode }) {
  const [open, setOpen] = useState(() => stored(`cut.${id}`, '0') === '1');
  const toggle = () => {
    store(`cut.${id}`, open ? '0' : '1');
    setOpen(!open);
  };
  return (
    <section className={`cut${open ? ' open' : ''}${className ? ` ${className}` : ''}`} id={id}>
      <button type="button" className="cut-head" aria-expanded={open} aria-controls={`${id}-body`} onClick={toggle}>
        <span className="cut-title">{title}</span>
        {hint && <span className="cut-hint">{hint}</span>}
        <Icon name="down" size={18} stroke={2} />
      </button>
      {open && (
        <div className="stack-s cut-body" id={`${id}-body`}>
          {children}
        </div>
      )}
    </section>
  );
}

export function Row({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="row">
      <span className="muted">{label}</span>
      <span className="row-val">{children}</span>
    </div>
  );
}

export function Header({ title, onBack, backLabel = 'Назад', right }: { title?: string; onBack: () => void; backLabel?: string; right?: ReactNode }) {
  return (
    <div className={'topbar' + (right ? ' with-right' : '')}>
      <button type="button" className="icon-btn" aria-label={backLabel} onClick={onBack}>
        <Icon name="left" size={24} stroke={2} />
      </button>
      {title ? <h1 className="topbar-title">{title}</h1> : <span />}
      {right ?? <span />}
    </div>
  );
}
