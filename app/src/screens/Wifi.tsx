import { useEffect, useRef, useState } from 'react';
import { api, inApp, isMobileApp, message, scanQr, type LanNearby, type LanPeer, type LanView, type PairResult } from '../api';
import { NB } from '../format';
import { Cut } from '../ui';

/** «2 мин назад», «вчера в 18:30», «12 ноября в 09:05». */
export function agoText(unix: number, now = Date.now()): string {
  const sec = Math.max(0, Math.floor(now / 1000 - unix));
  if (sec < 60) return 'только что';
  if (sec < 3600) return `${Math.floor(sec / 60)}${NB}мин назад`;
  const d = new Date(unix * 1000);
  const t = d.toLocaleTimeString('ru-RU', { hour: '2-digit', minute: '2-digit' });
  const today = new Date(now);
  const y = new Date(now - 86400000);
  if (d.toDateString() === today.toDateString()) return `сегодня в ${t}`;
  if (d.toDateString() === y.toDateString()) return `вчера в ${t}`;
  return `${d.toLocaleDateString('ru-RU', { day: 'numeric', month: 'long' })} в ${t}`;
}

const mmss = (s: number) => `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`;

/** Итог: связь по коду («Связано с …») или обмен («Обмен с …»). */
type Result = PairResult & { synced?: boolean };

function resultText(r: Result): string {
  const files = r.files_sent + r.files_received ? `; обложек: отправлено ${r.files_sent}, получено ${r.files_received}` : '';
  return `${r.remembered && !r.synced ? 'Связано с' : 'Обмен с'} «${r.peer_name}»: отправлено ${r.sent}, получено ${r.received}${files}`;
}

/** Ввод кода: «6WVG-HEWK», дефис ставится сам. */
function codeInput(v: string): string {
  const raw = v.replace(/[\s-]/g, '').toUpperCase().slice(0, 8);
  return raw.length > 4 ? raw.slice(0, 4) + '-' + raw.slice(4) : raw;
}

function PeerRow({ p, busy, onSync, onUnpair }: { p: LanPeer; busy: boolean; onSync: () => void; onUnpair: () => void }) {
  const [ask, setAsk] = useState(false);
  const state = p.syncing ? 'обмениваюсь…' : p.online ? 'в сети' : 'не в сети';
  const last = p.last ? `обмен ${agoText(p.last)}` : 'ещё не обменивались';
  return (
    <div className="peerrow">
      <div className="row peer-line">
        <span className="peer-name">
          <span className={'dot' + (p.online ? ' on' : '')} aria-hidden="true" />
          <span className="row-label">
            <span>{p.name}</span>
            <span className="small muted">
              {state} · {last}
            </span>
          </span>
        </span>
        {!ask && (
          <span className="key-actions">
            <button type="button" className="btn line small" disabled={busy || p.syncing || !p.online} onClick={onSync} aria-label={`Обменяться с «${p.name}»`}>
              Обменяться
            </button>
            <button type="button" className="link-btn" onClick={() => setAsk(true)} aria-label={`Отвязать «${p.name}»`}>
              Отвязать
            </button>
          </span>
        )}
      </div>
      {p.error && !p.syncing && <p className="small warn-text">{p.error}</p>}
      {ask && (
        <div className="stack-s">
          <p className="small muted">Отвязать «{p.name}»? Обмен по Wi-Fi с ним прекратится, записи останутся на обоих устройствах. Связать снова можно по новому коду.</p>
          <div className="key-actions">
            <button type="button" className="btn warn small" onClick={onUnpair}>
              Отвязать
            </button>
            <button type="button" className="link-btn" onClick={() => setAsk(false)}>
              Отмена
            </button>
          </div>
        </div>
      )}
    </div>
  );
}

/** Настройки → «Обмен по Wi-Fi»: связанные устройства, код связи, кто рядом. */
export default function WifiSection({ onSynced }: { onSynced: () => void }) {
  const [view, setView] = useState<LanView | null>(null);
  const [secs, setSecs] = useState(0);
  // Кому вводим код: устройство из списка или адрес.
  const [target, setTarget] = useState<LanNearby | 'addr' | null>(null);
  const [code, setCode] = useState('');
  const [addr, setAddr] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<Result | null>(null);
  const scanning = useRef(false);
  const alive = useRef(true);

  const apply = (v: LanView) => {
    if (!alive.current) return;
    setView(v);
    setSecs(v.code ? v.code.secs : 0);
  };
  const load = () => api.lanStatus().then(apply, () => undefined);
  const scan = async () => {
    if (scanning.current) return;
    scanning.current = true;
    try {
      apply(await api.lanScan());
    } catch {
      /* экран закрывается */
    } finally {
      scanning.current = false;
    }
  };

  useEffect(() => {
    alive.current = true;
    scan();
    const un = api.onLan(() => load()).catch(() => () => undefined);
    return () => {
      alive.current = false;
      un.then((f) => f());
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Пока раздел открыт: с кодом — состояние раз в 2 секунды, без кода — поиск рядом раз в 5 секунд.
  const showing = !!view?.code;
  useEffect(() => {
    const id = window.setInterval(() => (showing ? load() : scan()), showing ? 2000 : 5000);
    return () => window.clearInterval(id);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [showing]);
  useEffect(() => {
    if (!showing) return;
    const id = window.setInterval(() => setSecs((s) => Math.max(0, s - 1)), 1000);
    return () => window.clearInterval(id);
  }, [showing]);

  const act = async <T,>(fn: () => Promise<T>): Promise<T | undefined> => {
    setBusy(true);
    setError(null);
    try {
      return await fn();
    } catch (e) {
      setError(message(e));
      return undefined;
    } finally {
      setBusy(false);
    }
  };
  const paired = (r: PairResult | undefined) => {
    if (!r) return;
    setResult(r);
    setTarget(null);
    setCode('');
    setAddr('');
    onSynced();
    load();
  };
  const pair = () => {
    if (!target) return;
    setResult(null);
    act(() => (target === 'addr' ? api.lanPair(null, addr, code) : api.lanPair(target.device, null, code))).then(paired);
  };

  const v = view;
  const peers = v?.peers ?? [];
  return (
    <Cut id="wifi" title="Обмен по Wi-Fi" className="wifi" hint={peers.length ? `связано: ${peers.length}` : 'не связано'}>
      {peers.length > 0 && (
        <div className="stack-s">
          {peers.map((p) => (
            <PeerRow
              key={p.id}
              p={p}
              busy={busy}
              onSync={() => act(() => api.lanSyncNow(p.id)).then((r) => r && (setResult({ ...r, synced: true }), onSynced(), load()))}
              onUnpair={() => act(() => api.lanUnpair(p.id)).then((nv) => nv && apply(nv))}
            />
          ))}
          <p className="small muted">Связанные устройства в одной сети обмениваются сами — вскоре после изменений и раз в минуту, пока «Полка» открыта. Обложки едут вместе с записями.</p>
        </div>
      )}

      <span className="small muted wifi-sub">{peers.length ? 'Связать ещё одно устройство' : 'Связать устройства'}</span>
      {v?.code ? (
        <div className="paircard stack-s">
          <p className="small muted">Код связи с этим устройством:</p>
          <p className="paircode" aria-label={'Код связи ' + v.code.code.split('').join(' ')}>
            {v.code.code}
          </p>
          <p className="small muted">
            Действует ещё {mmss(secs)} и срабатывает один раз. На другом устройстве откройте Настройки → Обмен по Wi-Fi, выберите «{v.name}» в списке «Рядом в сети» и введите код
            {isMobileApp ? '.' : ' — или отсканируйте QR-код телефоном.'}
          </p>
          {v.code.wrong > 0 && <p className="small warn-text">Неверных попыток: {v.code.wrong} из 5. После пятой код отменится.</p>}
          <div className="qr" dangerouslySetInnerHTML={{ __html: v.code.qr_svg }} aria-label="QR-код: код связи и адрес этого устройства" role="img" />
          <p className="small muted3">
            Адрес этого устройства: {v.code.hosts.join(', ')}
            {v.code.port !== 47510 ? `, порт ${v.code.port}` : ''}. Не показывайте код и QR-код посторонним: по ним можно связаться с «Полкой» и получить записи.
          </p>
          <div className="key-actions">
            <button type="button" className="btn line small" disabled={busy} onClick={() => act(api.lanCodeStop).then((nv) => nv && apply(nv))}>
              Отменить код
            </button>
          </div>
        </div>
      ) : (
        <>
          <p className="small muted">
            На одном устройстве нажмите «Показать код», на другом выберите это устройство в списке «Рядом в сети» и введите код. Устройства запомнят друг друга и дальше будут обмениваться сами. Нужна
            одна сеть Wi-Fi — или телефон раздаёт точку доступа.
          </p>
          <div className="key-actions">
            <button type="button" className="btn primary small" disabled={busy} onClick={() => act(api.lanCodeStart).then((nv) => nv && (setResult(null), apply(nv)))}>
              Показать код
            </button>
            {isMobileApp && (
              <button
                type="button"
                className="btn line small"
                disabled={busy}
                onClick={async () => {
                  const text = await act(scanQr);
                  if (text) paired(await act(() => api.lanPairQr(text)));
                }}
              >
                Сканировать QR-код
              </button>
            )}
          </div>
          <div className="stack-s">
            <span className="small muted">Рядом в сети</span>
            {v && v.nearby.length > 0 ? (
              v.nearby.map((n) => (
                <div key={n.device} className="row">
                  <span className="row-label">
                    <span>{n.name || 'Устройство без имени'}</span>
                    <span className="small muted">показан код · {n.addr.replace(/:47510$/, '')}</span>
                  </span>
                  <button
                    type="button"
                    className="btn primary small"
                    onClick={() => {
                      setTarget(n);
                      setCode('');
                      setError(null);
                    }}
                    aria-label={`Ввести код «${n.name}»`}
                  >
                    Ввести код
                  </button>
                </div>
              ))
            ) : (
              <p className="small muted3">{v ? 'Устройств с показанным кодом не видно. Покажите код на другом устройстве — оно появится здесь.' : 'Ищу…'}</p>
            )}
            {target !== 'addr' && (
              <button type="button" className="link-btn" onClick={() => (setTarget('addr'), setError(null))}>
                Нет в списке? Связать по адресу
              </button>
            )}
          </div>
          {target && (
            <form
              className="paircard key-form"
              aria-label="Ввод кода связи"
              onSubmit={(e) => {
                e.preventDefault();
                pair();
              }}
            >
              {target === 'addr' && (
                <label className="field">
                  <span>Адрес того устройства — он под кодом</span>
                  <input inputMode="decimal" autoComplete="off" placeholder="192.168.1.34" value={addr} onChange={(e) => setAddr(e.target.value)} />
                </label>
              )}
              <label className="field">
                <span>{target === 'addr' ? 'Код с экрана того устройства' : `Код с экрана «${target.name}»`}</span>
                <input
                  className="codefield"
                  autoComplete="off"
                  autoCapitalize="characters"
                  spellCheck={false}
                  placeholder="XXXX-XXXX"
                  value={code}
                  onChange={(e) => setCode(codeInput(e.target.value))}
                  autoFocus
                />
              </label>
              <div className="key-actions">
                <button type="submit" className="btn primary small" disabled={busy || code.replace('-', '').length !== 8 || (target === 'addr' && !addr.trim())}>
                  {busy ? 'Связываю…' : 'Связать'}
                </button>
                <button type="button" className="link-btn" disabled={busy} onClick={() => (setTarget(null), setCode(''), setError(null))}>
                  Отмена
                </button>
              </div>
            </form>
          )}
        </>
      )}
      {error && <p className="small warn-text">{error}</p>}
      {v?.note && !v.code && !result && <p className={'small ' + (v.note.startsWith('Код') ? 'warn-text' : 'muted')}>{v.note}</p>}
      {result && <p className="small ok-text">{resultText(result)}</p>}
      {result?.hint && <p className="small warn-text">{result.hint}</p>}
      {!inApp && <p className="small muted">В демо код показывается понарошку, а связь и обмен не работают.</p>}
    </Cut>
  );
}
