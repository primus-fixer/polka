import { useEffect, useState } from 'react';
import type { Nav } from '../App';
import { api, inApp, message, type Goal, type Settings as S } from '../api';
import { plural } from '../format';
import { SCHEMES, type ThemeMode } from '@primus-fixer/kit/ui';
import { Cut, Seg } from '../ui';
import BackupSection from './Backup';
import BookFilesSection from './BookFiles';
import ClaudeSection from './Claude';
import CloudSection from './Cloud';
import SecuritySection, { type Security } from './Security';
import WifiSection from './Wifi';

type Props = {
  scheme: string;
  setScheme: (s: string) => void;
  mode: ThemeMode;
  setMode: (m: ThemeMode) => void;
  goal: Goal;
  year: number;
  onGoal: () => void;
  /** Пришли записи с другого устройства — перечитать полку. */
  onSynced: () => void;
  nav: Nav;
  /** Вход по PIN-коду и отпечатку. */
  security: Security;
};

const METHOD: Record<string, string> = {
  dpapi: 'ключ базы защищён учётной записью Windows',
  'android-keystore': 'ключ базы — в хранилище ключей Android',
  file: 'ключ базы — в файле рядом с базой (режим разработки)',
  demo: 'демо в браузере: данные не сохраняются',
};

/** «24 книги и 12 фильмов» или «не задана». */
function goalHint(g: Goal): string {
  const parts = [g.books ? plural(g.books, 'книга', 'книги', 'книг') : '', g.screen ? plural(g.screen, 'фильм', 'фильма', 'фильмов') : ''].filter(Boolean);
  return parts.length ? parts.join(' и ') : 'не задана';
}

export default function Settings({ scheme, setScheme, mode, setMode, goal, year, onGoal, onSynced, nav, security }: Props) {
  const [info, setInfo] = useState<S | null>(null);
  const [books, setBooks] = useState(goal.books ? String(goal.books) : '');
  const [screen, setScreen] = useState(goal.screen ? String(goal.screen) : '');
  const [kp, setKp] = useState('');
  const [google, setGoogle] = useState('');
  const dark = mode === 'dark' || (mode === 'system' && window.matchMedia('(prefers-color-scheme: dark)').matches);

  const load = () => api.settings(year).then(setInfo).catch((e) => nav.toast(message(e)));
  useEffect(() => {
    load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const saveGoal = async () => {
    const n = (s: string) => (s.trim() && Number(s) > 0 ? Math.min(1000, Math.round(Number(s))) : null);
    try {
      await api.setGoal(year, { books: n(books), screen: n(screen) });
      onGoal();
      nav.toast('Цель сохранена');
    } catch (e) {
      nav.toast(message(e));
    }
  };

  const saveKey = async (name: 'poiskkino' | 'google', value: string, clear: () => void) => {
    try {
      await api.setKey(name, value.trim());
      clear();
      await load();
      nav.toast(value.trim() ? 'Ключ сохранён на этом устройстве' : 'Ключ удалён');
    } catch (e) {
      nav.toast(message(e));
    }
  };

  return (
    <div className="stack settings">
      <h1 className="screen-title">Настройки</h1>

      <Cut id="look" title="Вид" hint={`${SCHEMES[scheme]?.name ?? ''}, ${mode === 'dark' ? 'тёмная' : mode === 'light' ? 'светлая' : 'как в системе'}`}>
        <div className="schemes" role="group" aria-label="Расцветка">
          {Object.entries(SCHEMES).map(([id, s]) => {
            const t = dark ? s.dark : s.light;
            return (
              <button key={id} type="button" className={`scheme${id === scheme ? ' on' : ''}`} aria-pressed={id === scheme} onClick={() => setScheme(id)}>
                <span className="swatch" style={{ background: `linear-gradient(90deg, ${t.own} 50%, ${t.work} 50%)` }} />
                <span>{s.name}</span>
              </button>
            );
          })}
        </div>
        <Seg
          cols
          small
          label="Тема"
          value={mode}
          options={[
            ['light', 'Светлая'],
            ['dark', 'Тёмная'],
            ['system', 'Как в системе'],
          ]}
          onChange={setMode}
        />
      </Cut>

      <Cut id="goal" title={`Цель на ${year} год`} hint={goalHint(goal)}>
        <div className="two">
          <label className="field">
            <span>Книг</span>
            <input inputMode="numeric" value={books} onChange={(e) => setBooks(e.target.value)} placeholder="без цели" />
          </label>
          <label className="field">
            <span>Фильмов и сериалов</span>
            <input inputMode="numeric" value={screen} onChange={(e) => setScreen(e.target.value)} placeholder="без цели" />
          </label>
        </div>
        <button type="button" className="btn line" onClick={saveGoal}>
          Сохранить цель
        </button>
      </Cut>

      <Cut id="catalogs" title="Каталоги" hint={info ? (info.has_poiskkino ? 'книги и кино' : 'книги; для кино нужен ключ') : undefined}>
        <div className="row block">
          <span>Книги</span>
          <span className="small muted">Open Library, Google Books и FantLab — работают без ключа.</span>
        </div>
        <form
          className="key-form"
          onSubmit={(e) => {
            e.preventDefault();
            saveKey('poiskkino', kp, () => setKp(''));
          }}
        >
          <label className="field">
            <span>Фильмы и сериалы — ключ ПоискКино</span>
            <input value={kp} onChange={(e) => setKp(e.target.value)} placeholder={info?.has_poiskkino ? 'ключ указан — введите новый, чтобы заменить' : 'ключ из Telegram-бота @poiskkinodev_bot'} autoComplete="off" spellCheck={false} />
          </label>
          <div className="key-actions">
            <button type="submit" className="btn line small" disabled={!kp.trim()}>
              Сохранить
            </button>
            {info?.has_poiskkino && (
              <button type="button" className="link-btn warn-text" onClick={() => saveKey('poiskkino', '', () => setKp(''))}>
                Удалить ключ
              </button>
            )}
          </div>
          <p className="small muted">{info?.has_poiskkino ? 'Ключ указан. Бесплатно — до 200 запросов в сутки.' : 'Без ключа фильмы и сериалы добавляются вручную.'}</p>
        </form>
        <form
          className="key-form"
          onSubmit={(e) => {
            e.preventDefault();
            saveKey('google', google, () => setGoogle(''));
          }}
        >
          <label className="field">
            <span>Google Books — ключ</span>
            <input value={google} onChange={(e) => setGoogle(e.target.value)} placeholder={info?.has_google ? 'ключ указан — введите новый, чтобы заменить' : 'нужен, если Google пишет «без ключа не отвечает»'} autoComplete="off" spellCheck={false} />
          </label>
          <div className="key-actions">
            <button type="submit" className="btn line small" disabled={!google.trim()}>
              Сохранить
            </button>
            {info?.has_google && (
              <button type="button" className="link-btn warn-text" onClick={() => saveKey('google', '', () => setGoogle(''))}>
                Удалить ключ
              </button>
            )}
          </div>
          <p className="small muted">{info?.has_google ? 'Ключ указан. Бесплатно — до 1000 запросов в день.' : 'Без ключа Google отвечает не из всех сетей. С ключом — до 1000 запросов в день.'}</p>
        </form>
        <p className="small muted">Ключи хранятся только на этом устройстве, в зашифрованном виде.</p>
      </Cut>

      <ClaudeSection nav={nav} onChange={onSynced} />

      <WifiSection onSynced={onSynced} />

      <CloudSection />

      <BookFilesSection nav={nav} onChange={onSynced} />

      <BackupSection nav={nav} />

      <SecuritySection sec={security} />

      <Cut id="data" title="Данные">
        <p className="small muted">
          Записи и обложки — в зашифрованной базе на устройстве{info ? `; ${METHOD[info.key_method] ?? info.key_method}` : ''}. На другие устройства они уходят только по Wi-Fi или через облако — зашифрованными.
        </p>
        {!inApp && <p className="small warn-text">Это демо в браузере: изменения пропадут после перезагрузки.</p>}
      </Cut>

      <p className="small muted3">Полка {__APP_VERSION__}</p>
    </div>
  );
}
