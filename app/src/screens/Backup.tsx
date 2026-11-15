import { useEffect, useRef, useState } from 'react';
import type { Nav } from '../App';
import { api, inApp, isMobileApp, message, type BackupEvery, type BackupInfo, type BackupStatus } from '../api';
import { NB, count } from '../format';
import { Bar, Cut, Icon, Seg } from '../ui';

const MONTHS = ['января', 'февраля', 'марта', 'апреля', 'мая', 'июня', 'июля', 'августа', 'сентября', 'октября', 'ноября', 'декабря'];

/** «сегодня в 18:30», «вчера в 9:05», «7 ноября в 18:30», «7 ноября 2025 в 18:30». */
export function whenText(unix: number, now = Date.now()): string {
  const d = new Date(unix * 1000);
  const t = new Date(now);
  const time = `${d.getHours()}:${String(d.getMinutes()).padStart(2, '0')}`;
  const day = (x: Date) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime();
  const diff = Math.round((day(t) - day(d)) / 86400000);
  if (diff === 0) return `сегодня в ${time}`;
  if (diff === 1) return `вчера в ${time}`;
  const date = `${d.getDate()}${NB}${MONTHS[d.getMonth()]}` + (d.getFullYear() === t.getFullYear() ? '' : ` ${d.getFullYear()}`);
  return `${date} в ${time}`;
}

/** «12,4 МБ», «830 КБ». */
export function bytesText(bytes: number): string {
  if (bytes >= 1 << 30) return (bytes / (1 << 30)).toFixed(1).replace('.', ',') + NB + 'ГБ';
  if (bytes >= 1 << 20) return (bytes / (1 << 20)).toFixed(1).replace('.', ',') + NB + 'МБ';
  return Math.max(1, Math.round(bytes / 1024)) + NB + 'КБ';
}

/** Последняя копия с любого устройства: «секунды|устройство». */
export function sharedLast(v: string | null): { at: number; device: string } | null {
  if (!v) return null;
  const i = v.indexOf('|');
  const at = Number(i < 0 ? v : v.slice(0, i));
  return Number.isFinite(at) && at > 0 ? { at, device: i < 0 ? '' : v.slice(i + 1) } : null;
}


const EVERY: [BackupEvery, string][] = [
  ['day', 'Каждый день'],
  ['week', 'Раз в неделю'],
  ['off', 'Выключены'],
];

/** Загрузка выбранного файла, пароль, что в копии и восстановление — на компьютере и на телефоне. */
function Restore({ first, nav, pick, onClose }: { first: File; nav: Nav; pick: () => Promise<File | null>; onClose: () => void }) {
  const [file, setFile] = useState(first);
  const [step, setStep] = useState<'upload' | 'password' | 'check' | 'preview' | 'restore'>('upload');
  const [id, setId] = useState<string | null>(null);
  const [done, setDone] = useState(0);
  const [password, setPassword] = useState('');
  const [info, setInfo] = useState<BackupInfo | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    setStep('upload');
    setInfo(null);
    setDone(0);
    setError(null);
    api.backupUpload(file, (d) => alive && setDone(d)).then(
      (got) => {
        // Выбрали другой файл, пока этот загружался, — его загрузка не нужна.
        if (!alive) return void api.backupDiscard(got).catch(() => undefined);
        setId(got);
        setStep('password');
      },
      (e) => alive && setError(message(e)),
    );
    return () => {
      alive = false;
    };
  }, [file]);

  const discard = () => {
    if (id) api.backupDiscard(id).catch(() => undefined);
  };
  const cancel = () => {
    discard();
    onClose();
  };
  const another = async () => {
    const f = await pick();
    if (!f) return;
    discard();
    setId(null);
    setFile(f);
  };
  const check = async () => {
    if (!id) return;
    setError(null);
    setStep('check');
    try {
      setInfo(await api.backupInspect(id, password));
      setStep('preview');
    } catch (e) {
      setStep('password');
      setError(message(e));
    }
  };
  const restore = async () => {
    if (!id) return;
    setError(null);
    setStep('restore');
    try {
      const r = await api.backupRestore(id, password);
      nav.replaceAll(r.list);
      const parts: string[] = [];
      if (r.titles) parts.push(count(r.titles, 'запись', 'записи', 'записей'));
      if (r.blobs) parts.push(count(r.blobs, 'обложка', 'обложки', 'обложек'));
      if (r.books) parts.push(count(r.books, 'файл книги', 'файла книг', 'файлов книг'));
      nav.toast(
        r.events === 0 && r.blobs === 0 && !r.books
          ? 'Всё из копии уже было на устройстве'
          : 'Восстановлено: ' + (parts.length ? parts.join(', ') : count(r.events, 'изменение', 'изменения', 'изменений')),
      );
      setPassword('');
      onClose();
    } catch (e) {
      setStep('preview');
      setError(message(e));
    }
  };

  return (
    <div className="restore stack-s" role="group" aria-label="Восстановление из копии">
      <div className="restore-head">
        <span className="restore-name">{file.name}</span>
        <button type="button" className="icon-btn" aria-label="Закрыть восстановление" onClick={cancel} disabled={step === 'restore'}>
          <Icon name="x" size={20} stroke={2} />
        </button>
      </div>
      {step === 'upload' && !error && (
        <div className="stack-s">
          <Bar pct={file.size ? Math.round((done / file.size) * 100) : 0} />
          <span className="small muted">
            Загружаю файл: {bytesText(done)} из {bytesText(file.size)}
          </span>
        </div>
      )}
      {(step === 'password' || step === 'check') && (
        <form
          className="key-form"
          onSubmit={(e) => {
            e.preventDefault();
            check();
          }}
        >
          <label className="field">
            <span>Пароль этой копии</span>
            <input type="password" autoComplete="current-password" value={password} onChange={(e) => setPassword(e.target.value)} disabled={step === 'check'} autoFocus />
          </label>
          <div className="key-actions">
            <button type="submit" className="btn line small" disabled={!password || step === 'check'}>
              {step === 'check' ? 'Проверяю копию…' : 'Открыть копию'}
            </button>
          </div>
        </form>
      )}
      {(step === 'preview' || step === 'restore') && info && (
        <div className="stack-s">
          <div className="row block">
            <span>
              Копия от {whenText(info.created)}
              {info.device ? `, «${info.device}»` : ''}
            </span>
            <span className="small muted">
              {[count(info.titles, 'запись', 'записи', 'записей'), count(info.blobs, 'обложка', 'обложки', 'обложек'), ...(info.books ? [count(info.books, 'файл книги', 'файла книг', 'файлов книг')] : []), bytesText(info.bytes)].join(' · ')}
              {info.app ? ` · «Полка» ${info.app}` : ''}
            </span>
          </div>
          <p className="small muted">Записи из копии добавятся к нынешним: ничего не пропадёт и не задвоится. То, что удалили или поправили уже после копии, так и останется.</p>
          <div className="key-actions">
            <button type="button" className="btn primary small" onClick={restore} disabled={step === 'restore'}>
              {step === 'restore' ? 'Восстанавливаю…' : 'Добавить к записям'}
            </button>
            <button type="button" className="link-btn" onClick={cancel} disabled={step === 'restore'}>
              Отмена
            </button>
          </div>
        </div>
      )}
      {error && <p className="small warn-text">{error}</p>}
      {(step === 'upload' || step === 'password') && (error || step === 'password') && (
        <button type="button" className="link-btn" onClick={another}>
          Выбрать другой файл
        </button>
      )}
    </div>
  );
}

/** Пароль копии: два поля, не короче восьми символов. */
function PasswordForm({ first, cloud, onSaved, onCancel }: { first: boolean; cloud: boolean; onSaved: (s: BackupStatus) => void; onCancel?: () => void }) {
  const [a, setA] = useState('');
  const [b, setB] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const save = async () => {
    if ([...a].length < 8) return setError('Пароль копии — не короче 8 символов');
    if (a !== b) return setError('Пароли не совпадают');
    setBusy(true);
    setError(null);
    try {
      const s = await api.backupPassword(a);
      setA('');
      setB('');
      onSaved(s);
    } catch (e) {
      setError(message(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <form
      className="key-form"
      onSubmit={(e) => {
        e.preventDefault();
        save();
      }}
    >
      <p className="small muted">
        Не короче 8 символов, лучше — фраза из нескольких слов. Пароль нигде не хранится и не восстанавливается: забудете — копию не открыть.
        {first ? '' : ' Прежние копии открываются прежним паролем.'}
        {cloud
          ? ' Этим же паролем зашифровано облако: новый пароль уйдёт туда при ближайшем обмене. Если старый пароль мог узнать кто-то ещё, после смены отключите облако с удалением данных и подключите снова — сменится и ключ облака.'
          : ''}
      </p>
      <label className="field">
        <span>Пароль копии</span>
        <input type="password" autoComplete="new-password" value={a} onChange={(e) => setA(e.target.value)} />
      </label>
      <label className="field">
        <span>Ещё раз</span>
        <input type="password" autoComplete="new-password" value={b} onChange={(e) => setB(e.target.value)} />
      </label>
      {error && <p className="small warn-text">{error}</p>}
      <div className="key-actions">
        <button type="submit" className="btn line small" disabled={busy || !a || !b}>
          {busy ? 'Сохраняю…' : first ? 'Задать пароль' : 'Сменить пароль'}
        </button>
        {onCancel && (
          <button type="button" className="link-btn" onClick={onCancel} disabled={busy}>
            Отмена
          </button>
        )}
      </div>
    </form>
  );
}

/** Настройки → «Резервная копия». */
export default function BackupSection({ nav }: { nav: Nav }) {
  const [st, setSt] = useState<BackupStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [changing, setChanging] = useState(false);
  const [making, setMaking] = useState(false);
  const [restoring, setRestoring] = useState<File | null>(null);
  const input = useRef<HTMLInputElement>(null);
  const waiting = useRef<((f: File | null) => void) | null>(null);

  const load = () => api.backupStatus().then(setSt, (e) => setError(message(e)));
  useEffect(() => {
    load();
    const un = api.onBackup(load).catch(() => () => undefined);
    return () => {
      un.then((f) => f());
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Окно выбора файла открывается прямо по нажатию — иначе система его не покажет.
  const pick = () =>
    new Promise<File | null>((resolve) => {
      waiting.current = resolve;
      if (input.current) {
        input.current.value = '';
        input.current.click();
      }
    });
  const startRestore = async () => {
    const f = await pick();
    if (f) setRestoring(f);
  };

  const act = async (p: Promise<BackupStatus>) => {
    setError(null);
    try {
      setSt(await p);
    } catch (e) {
      setError(message(e));
    }
  };
  const now = async () => {
    setMaking(true);
    setError(null);
    try {
      const last = await api.backupNow();
      nav.toast('Копия сохранена и проверена: ' + last.path);
      load();
    } catch (e) {
      setError(message(e));
      load();
    } finally {
      setMaking(false);
    }
  };

  const desktop = st ? st.desktop : !isMobileApp;
  const shared = sharedLast(st?.shared ?? null);
  const lastText = st?.last ? whenText(st.last.at) : shared ? `${whenText(shared.at)}${shared.device ? `, «${shared.device}»` : ''}` : null;

  return (
    <Cut id="backup" title="Резервная копия" hint={!st ? undefined : st.every === 'off' ? 'автокопии выключены' : st.every === 'week' ? 'раз в неделю' : 'каждый день'}>
      <input
        ref={input}
        type="file"
        hidden
        // Файл копии на телефоне система не знает — показываем все файлы.
        accept={isMobileApp ? undefined : '.polka'}
        onChange={(e) => {
          const f = e.target.files?.[0] ?? null;
          waiting.current?.(f);
          waiting.current = null;
        }}
      />
      {desktop ? (
        <>
          <p className="small muted">Все записи и обложки в одном файле, зашифрованном паролем копии. Файл открывается на любом компьютере или телефоне с «Полкой» — по паролю.</p>
          <div className="row">
            <span>Последняя копия</span>
            <span className="row-val">
              {lastText ?? <span className="warn-text">ещё не было</span>}
              {st?.last && <span className="small muted backup-sub">{(st.last.auto ? 'автокопия' : 'по кнопке') + ', ' + bytesText(st.last.bytes)}</span>}
            </span>
          </div>
          {st?.error && <p className="small warn-text">Автокопия не получилась: {st.error}</p>}

          <div className="row">
            <span className="row-label">
              <span>Пароль копии</span>
              <span className={'small ' + (st && !st.password ? 'warn-text' : 'muted')}>{st ? (st.password ? (st.cloud ? 'Задан, им же зашифровано облако' : 'Задан') : 'Не задан — без него копии не делаются') : '…'}</span>
            </span>
            {st?.password && !changing && (
              <button type="button" className="btn line small" onClick={() => setChanging(true)}>
                Сменить
              </button>
            )}
          </div>
          {st && (!st.password || changing) && (
            <PasswordForm
              first={!st.password}
              cloud={st.cloud}
              onSaved={(s) => {
                setSt(s);
                setChanging(false);
                nav.toast(st.password ? 'Пароль копии сменён' : 'Пароль копии задан: копии будут делаться сами');
              }}
              onCancel={st.password ? () => setChanging(false) : undefined}
            />
          )}

          <div className="row">
            <span className="row-label backup-dir">
              <span>Папка</span>
              <span className="small muted">{st?.dir ?? '…'}</span>
              {st && !st.chosen && <span className="small muted">По умолчанию. Выберите папку Яндекс Диска на компьютере — копии сами уйдут в облако.</span>}
              {st?.chosen && (
                <button type="button" className="link-btn" onClick={() => act(api.backupDir(null))}>
                  Вернуть папку по умолчанию
                </button>
              )}
            </span>
            <button type="button" className="btn line small" onClick={() => act(api.backupPickDir())}>
              Выбрать папку
            </button>
          </div>

          <div className="stack-s">
            <span className="small muted">Автокопии</span>
            <Seg label="Как часто делать автокопию" value={st?.every ?? 'day'} options={EVERY} onChange={(v) => act(api.backupEvery(v))} small />
            <p className="small muted">
              {st?.every === 'off'
                ? 'Только по кнопке «Сделать копию».'
                : `${st?.every === 'week' ? 'Раз в неделю' : 'Раз в день'}, пока «Полка» открыта. Хранятся последние ${st?.keep ?? 14}, старые удаляются сами.`}
            </p>
          </div>

          <div className="key-actions">
            <button type="button" className="btn primary small" disabled={!st?.password || making || st?.running} onClick={now}>
              {making || st?.running ? 'Делаю копию…' : 'Сделать копию'}
            </button>
            {!restoring && (
              <button type="button" className="btn line small" onClick={startRestore}>
                Восстановить из копии
              </button>
            )}
          </div>
        </>
      ) : (
        <>
          <p className="small muted">
            Копии делает «Полка» на компьютере: задайте там пароль копии. Записи телефона попадают в копию после обмена по Wi-Fi или через облако, а новый телефон получит всё обратно из облака или от компьютера.
          </p>
          <div className="row">
            <span>Последняя копия</span>
            <span className="row-val">{lastText ?? <span className="warn-text">ещё не было</span>}</span>
          </div>
          {!restoring && (
            <div className="row">
              <span className="row-label">
                <span>Восстановить из файла</span>
                <span className="small muted">Если компьютера нет под рукой: скачайте файл копии на телефон</span>
              </span>
              <button type="button" className="btn line small" onClick={startRestore}>
                Выбрать файл
              </button>
            </div>
          )}
        </>
      )}
      {restoring && <Restore key={restoring.name + restoring.size + restoring.lastModified} first={restoring} nav={nav} pick={pick} onClose={() => setRestoring(null)} />}
      {error && <p className="small warn-text">{error}</p>}
      {!inApp && <p className="small muted">В демо копия делается понарошку, а восстановление не работает.</p>}
    </Cut>
  );
}
