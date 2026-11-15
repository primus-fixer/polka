import { useCallback, useEffect, useState } from 'react';
import { api, inApp, message, type CloudLast, type CloudProvider, type CloudStatus, type CloudStep } from '../api';
import { NB, count } from '../format';
import { Seg } from '../ui';
import { whenText } from './Backup';


const EVERY: [CloudStatus['every'], string][] = [
  ['auto', 'Сам'],
  ['manual', 'Только по кнопке'],
];

export function stepText(s: CloudStep | null): string {
  if (!s || s.step === 'events') return 'Обмениваюсь записями…';
  const n = `${Math.min(s.done + 1, s.total)} из ${s.total}`;
  if (s.step === 'books') return `Файлы книг: ${n}`;
  return s.step === 'upload' ? `Отправляю обложки: ${n}` : `Получаю обложки: ${n}`;
}

export function lastText(l: CloudLast): string {
  const parts = [`получено ${l.received}, отправлено ${l.sent}`];
  if (l.files_received + l.files_sent > 0) parts.push(`обложек: получено ${l.files_received}, отправлено ${l.files_sent}`);
  const br = l.books_received ?? 0;
  const bs = l.books_sent ?? 0;
  if (br + bs > 0) parts.push(`файлов книг: получено ${br}, отправлено ${bs}`);
  return parts.join('; ');
}

/** Пароль копий для облака: ввести (облако уже есть или пароль копий задан) или придумать. */
function CloudPassword({ st, onDone, onCancel }: { st: CloudStatus; onDone: (s: CloudStatus) => void; onCancel: () => void }) {
  const invent = !st.cloud_exists && !(st.desktop && st.backup_password);
  const [a, setA] = useState('');
  const [b, setB] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const save = async () => {
    if ([...a].length < 8) return setError('Пароль копий — не короче 8 символов');
    if (invent && a !== b) return setError('Пароли не совпадают');
    setBusy(true);
    setError(null);
    try {
      const s = await api.cloudPassword(a);
      setA('');
      setB('');
      onDone(s);
    } catch (e) {
      setError(message(e));
    } finally {
      setBusy(false);
    }
  };
  const text = st.cloud_exists
    ? 'В облаке уже есть данные «Полки». Введите пароль копий, которым они зашифрованы.'
    : st.desktop && st.backup_password
      ? 'Введите пароль копий этого компьютера — им будут зашифрованы данные в облаке.'
      : `Придумайте пароль копий: им шифруются данные в облаке${st.desktop ? ' и резервные копии' : ''}. Не короче 8 символов, лучше — фраза из нескольких слов. Запишите его: без пароля новое устройство не получит данные из облака.`;
  return (
    <form
      className="key-form"
      aria-label="Пароль копий для облака"
      onSubmit={(e) => {
        e.preventDefault();
        save();
      }}
    >
      <p className="small muted">{text}</p>
      <label className="field">
        <span>Пароль копий</span>
        <input type="password" autoComplete={invent ? 'new-password' : 'current-password'} value={a} onChange={(e) => setA(e.target.value)} disabled={busy} />
      </label>
      {invent && (
        <label className="field">
          <span>Ещё раз</span>
          <input type="password" autoComplete="new-password" value={b} onChange={(e) => setB(e.target.value)} disabled={busy} />
        </label>
      )}
      {error && <p className="small warn-text">{error}</p>}
      <div className="key-actions">
        <button type="submit" className="btn primary small" disabled={busy || !a || (invent && !b)}>
          {busy ? 'Проверяю пароль…' : 'Подключить'}
        </button>
        <button type="button" className="link-btn" disabled={busy} onClick={onCancel}>
          Отмена
        </button>
      </div>
    </form>
  );
}

/** Вход по логину и паролю: Яндекс Диск (пароль приложения) или другой сервер WebDAV. */
function DavForm({ kind, st, again, onDone, onCancel }: { kind: CloudProvider; st: CloudStatus; again: boolean; onDone: (s: CloudStatus) => void; onCancel: () => void }) {
  const ya = kind === 'yadisk';
  const [url, setUrl] = useState(st.dav && st.provider === kind ? st.dav.url : '');
  const [user, setUser] = useState(st.dav && st.provider === kind ? st.dav.user : '');
  const [password, setPassword] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const submit = async () => {
    setBusy(true);
    setError(null);
    try {
      const s = await api.cloudDavLogin(kind, ya ? null : url, user, password);
      setPassword('');
      onDone(s);
    } catch (e) {
      setError(message(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <form
      className="key-form"
      aria-label={ya ? 'Вход в Яндекс Диск' : 'Вход на сервер WebDAV'}
      onSubmit={(e) => {
        e.preventDefault();
        submit();
      }}
    >
      <p className="cloud-form-title">{ya ? 'Яндекс Диск — вход по паролю приложения' : 'Свой сервер WebDAV'}</p>
      {ya ? (
        <>
          <p className="small muted">
            Нужен <b>пароль приложения</b>, а не пароль от аккаунта Яндекса: Яндекс ID → Безопасность → Пароли приложений → «Файлы WebDAV». Яндекс показывает его один раз — сразу скопируйте. Новый пароль
            начинает действовать через 2–3{NB}часа.
          </p>
          <div className="key-actions">
            <button type="button" className="btn line small" onClick={() => api.cloudAppPasswords().catch((e) => setError(message(e)))}>
              Открыть Яндекс ID
            </button>
          </div>
        </>
      ) : (
        <>
          <p className="small muted">Подойдёт сервер WebDAV с https: Nextcloud, Synology или другой NAS. Адрес — папка WebDAV пользователя, например https://cloud.example.ru/remote.php/dav/files/имя.</p>
          <label className="field">
            <span>Адрес сервера</span>
            <input type="url" inputMode="url" autoComplete="url" spellCheck={false} placeholder="https://…" value={url} onChange={(e) => setUrl(e.target.value)} disabled={busy} />
          </label>
        </>
      )}
      <label className="field">
        <span>{ya ? 'Логин Яндекса' : 'Логин'}</span>
        <input autoComplete="username" autoCapitalize="none" spellCheck={false} value={user} onChange={(e) => setUser(e.target.value)} disabled={busy} />
      </label>
      <label className="field">
        <span>{ya ? 'Пароль приложения' : 'Пароль'}</span>
        <input type="password" autoComplete="current-password" value={password} onChange={(e) => setPassword(e.target.value)} disabled={busy} />
      </label>
      <p className="small muted3">Пароль хранится только на этом устройстве, зашифрованным, и не попадает в копии. «Полка» работает только в своей папке «Полка».</p>
      {error && <p className="small warn-text">{error}</p>}
      <div className="key-actions">
        <button type="submit" className="btn primary small" disabled={busy || !user.trim() || !password || (!ya && !url.trim())}>
          {busy ? 'Проверяю вход…' : again ? 'Войти снова' : 'Войти'}
        </button>
        {!again && (
          <button type="button" className="link-btn" disabled={busy} onClick={onCancel}>
            Отмена
          </button>
        )}
      </div>
    </form>
  );
}

/** Настройки → «Облако»: Яндекс Диск по паролю приложения или свой сервер WebDAV. */
export default function CloudSection() {
  const [st, setSt] = useState<CloudStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [asking, setAsking] = useState(false);
  const [wipe, setWipe] = useState(false);
  const [form, setForm] = useState<CloudProvider | null>(null);

  const load = useCallback(() => api.cloudStatus().then(setSt, (e) => setError(message(e))), []);
  useEffect(() => {
    load();
    const un = api.onCloud(() => load()).catch(() => () => undefined);
    return () => {
      un.then((f) => f());
    };
  }, [load]);

  const act = async (p: () => Promise<CloudStatus>): Promise<CloudStatus | null> => {
    setBusy(true);
    setError(null);
    try {
      const s = await p();
      setSt(s);
      setNotice(s.notice);
      return s;
    } catch (e) {
      setError(message(e));
      return null;
    } finally {
      setBusy(false);
    }
  };
  const disconnect = async () => {
    const s = await act(() => api.cloudDisconnect(wipe));
    if (s) {
      setAsking(false);
      setWipe(false);
    }
  };
  const signedIn = (s: CloudStatus) => {
    setSt(s);
    setNotice(s.notice);
    setForm(null);
  };

  const who = st?.dav ? (st.provider === 'webdav' ? st.dav.host : st.dav.user) : null;
  const davLink = st?.provider ?? null;

  return (
    <section className="stack-s cloud" id="cloud">
      <h2 className="sec">Облако</h2>
      {st?.state === 'off' && !form && (
        <>
          <p className="small muted">
            Записи и обложки ходят через ваш Яндекс Диск или свой сервер — устройствам не нужно быть в одной сети. В облаке лежат только зашифрованные файлы в папке «Полка»: открыть их можно только
            паролем копий.
          </p>
          <div className="key-actions">
            <button
              type="button"
              className="btn primary small"
              disabled={busy}
              onClick={() => {
                setError(null);
                setNotice(null);
                setForm('yadisk');
              }}
            >
              Яндекс Диск
            </button>
            <button
              type="button"
              className="btn line small"
              disabled={busy}
              onClick={() => {
                setError(null);
                setNotice(null);
                setForm('webdav');
              }}
            >
              Другой сервер WebDAV
            </button>
          </div>
        </>
      )}

      {st?.state === 'off' && form && <DavForm key={form} kind={form} st={st} again={false} onDone={signedIn} onCancel={() => setForm(null)} />}

      {st?.state === 'password' && (
        <>
          {st.error && <p className="small warn-text">{st.error.text}</p>}
          <p className="small muted">
            {st.title}: вход выполнен{who ? ` (${who})` : ''}.
          </p>
          <CloudPassword
            st={st}
            onDone={(s) => {
              setSt(s);
              setNotice(s.notice);
            }}
            onCancel={() => act(() => api.cloudDisconnect(false)).then(() => setNotice(null))}
          />
        </>
      )}

      {st?.state === 'relogin' && davLink && (
        <>
          <p className="small warn-text">{st.error?.text ?? `${st.title} не принимает пароль — войдите снова.`}</p>
          <p className="small muted">Записи и пароль копий остаются на устройстве: после входа обмен продолжится сам.</p>
          <DavForm key={davLink} kind={davLink} st={st} again onDone={signedIn} onCancel={() => undefined} />
          {!asking && (
            <button type="button" className="link-btn" disabled={busy} onClick={() => setAsking(true)}>
              Отключить облако
            </button>
          )}
        </>
      )}

      {st?.state === 'ready' && (
        <>
          <div className="row">
            <span className="row-label">
              <span>{st.title}</span>
              {who && <span className="small muted">{who}</span>}
            </span>
            <span className="row-val ok-text">подключён</span>
          </div>
          <div className="row">
            <span>Последний обмен</span>
            <span className="row-val cloud-last" aria-live="polite">
              {st.running ? (
                stepText(st.progress)
              ) : st.last ? (
                <>
                  {whenText(st.last.at)}
                  <span className="small muted">{lastText(st.last)}</span>
                </>
              ) : (
                <span className="small muted">ещё не было</span>
              )}
            </span>
          </div>
          {!st.running && st.last && (st.last.books_waiting ?? 0) > 0 && (
            <p className="small muted">Ещё {count(st.last.books_waiting ?? 0, 'файл книги придёт', 'файла книг придут', 'файлов книг придут')}, когда другое устройство отправит их в облако целиком.</p>
          )}
          {!st.running && st.last && st.last.files_waiting > 0 && (
            <p className="small muted">Ещё {count(st.last.files_waiting, 'обложка придёт', 'обложки придут', 'обложек придут')}, когда их отправит другое устройство.</p>
          )}
          {st.error && <p className="small warn-text">{st.error.text}</p>}
          {st.last?.problems.map((p) => (
            <p key={p} className="small muted3">
              {p}
            </p>
          ))}
          {st.key_pending && <p className="small muted">Новый пароль копий уйдёт в облако при ближайшем обмене.</p>}
          <div className="stack-s">
            <span className="small muted">Обмен через облако</span>
            <Seg label="Когда обмениваться через облако" value={st.every} options={EVERY} onChange={(v) => act(() => api.cloudEvery(v))} small />
            <p className="small muted">{st.every === 'auto' ? 'При открытии «Полки», через минуту после изменений и каждые 15 минут, пока она открыта.' : 'Только по кнопке «Обменяться сейчас».'}</p>
          </div>
          {!asking && (
            <div className="key-actions">
              <button type="button" className="btn primary small" disabled={busy || st.running} onClick={() => act(api.cloudSyncNow)}>
                {st.running ? 'Обмениваюсь…' : 'Обменяться сейчас'}
              </button>
              <button type="button" className="link-btn" disabled={busy} onClick={() => setAsking(true)}>
                Отключить
              </button>
            </div>
          )}
        </>
      )}

      {asking && st && (st.state === 'ready' || st.state === 'relogin') && (
        <div className="paircard stack-s" role="group" aria-label="Отключить облако">
          <p>Отключить облако на этом устройстве?</p>
          {st.state === 'ready' && (
            <label className="cloud-check">
              <input type="checkbox" checked={wipe} onChange={(e) => setWipe(e.target.checked)} />
              <span className="row-label">
                <span>Удалить данные «Полки» из облака</span>
                <span className="small muted">Другие устройства тоже перестанут обмениваться через облако. На самих устройствах всё останется.</span>
              </span>
            </label>
          )}
          <div className="key-actions">
            <button type="button" className={'btn small ' + (wipe ? 'warn' : 'line')} disabled={busy} onClick={disconnect}>
              {busy ? 'Отключаю…' : wipe ? 'Отключить и удалить' : 'Отключить'}
            </button>
            <button
              type="button"
              className="link-btn"
              disabled={busy}
              onClick={() => {
                setAsking(false);
                setWipe(false);
              }}
            >
              Отмена
            </button>
          </div>
        </div>
      )}

      {notice && <p className="small muted">{notice}</p>}
      {error && <p className="small warn-text">{error}</p>}
      {!inApp && <p className="small muted">В демо облако понарошку: вход и обмен только изображаются, файлы никуда не уходят.</p>}
    </section>
  );
}
