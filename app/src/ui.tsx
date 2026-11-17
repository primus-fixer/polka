import { useState } from 'react';
import { addIcons, Icon } from '@primus-fixer/kit/ui';
import { coverSrc, type Kind, type Title } from './api';
import { ICONS } from './icons';

// Общие элементы экрана — из kit; значки «Полки» добавляются к значкам kit.
addIcons(ICONS);
export { Bar, Cut, Header, Icon, Row, Seg } from '@primus-fixer/kit/ui';

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
