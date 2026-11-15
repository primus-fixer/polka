// Настройки → Claude: коннектор для приложения Claude на компьютере — как в «Кармане».
import { useEffect, useState } from 'react';
import type { Nav } from '../App';
import { api, message, type ClaudeStatus } from '../api';
import { Cut, Icon } from '../ui';

const whenText = (at: number) => {
  const d = new Date(at * 1000);
  const p = (n: number) => String(n).padStart(2, '0');
  return `${p(d.getDate())}.${p(d.getMonth() + 1)} в ${p(d.getHours())}:${p(d.getMinutes())}`;
};

/** Примеры вопросов — чтобы было понятно, зачем это. */
const EXAMPLES = ['Что я прочитал в этом году и что понравилось больше всего?', 'Посоветуй, что взять из „Хочу“ на выходные', 'Я дочитал „Щегла“, поставь 9 из 10', 'Отметь, что я посмотрел третий сезон „Во все тяжкие“'];

export default function ClaudeSection({ nav, onChange }: { nav: Nav; onChange: () => void }) {
  const [st, setSt] = useState<ClaudeStatus | null>(null);
  const [manual, setManual] = useState(false);
  const [saved, setSaved] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const fail = (e: unknown) => nav.toast(message(e));

  useEffect(() => {
    api.claudeStatus().then(setSt, fail);
    // Claude что-то записал, пока открыты настройки, — обновить список действий.
    const off = api.onClaude(() => api.claudeStatus().then(setSt, () => undefined)).catch(() => () => undefined);
    return () => {
      off.then((f) => f());
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  if (!st) return null;

  if (!st.desktop)
    return (
      <Cut id="claude" title="Claude" hint="на компьютере">
        <p className="small muted">Коннектор для приложения Claude — в «Полке» на компьютере: там Claude отвечает на вопросы о полке и, если разрешить, добавляет записи. Сделанное им приходит на телефон при обмене по Wi-Fi или через облако.</p>
      </Cut>
    );

  const set = async (on: boolean, write: boolean, what: 'on' | 'write') => {
    try {
      const s = await api.claudeSet(on, write);
      setSt(s);
      if (what === 'on') nav.toast(s.on ? 'Подключение для Claude включено' : 'Подключение для Claude выключено');
      else nav.toast(s.write ? 'Claude может записывать — каждое действие с «Вернуть»' : 'Claude только читает');
    } catch (e) {
      fail(e);
    }
  };
  const bundle = async (open: boolean) => {
    setBusy(true);
    try {
      const path = await api.claudeBundle(open);
      setSaved(path);
      nav.toast(open ? 'Расширение открыто — подтвердите установку в приложении Claude' : 'Расширение сохранено в «Загрузки»');
    } catch (e) {
      fail(e);
    } finally {
      setBusy(false);
    }
  };
  const undo = async (id: number) => {
    try {
      setSt(await api.claudeUndo(id));
      onChange();
      nav.toast('Отменено');
    } catch (e) {
      fail(e);
    }
  };
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(st.config);
      nav.toast('Скопировано');
    } catch {
      nav.toast('Не скопировалось — выделите текст и скопируйте вручную');
    }
  };

  return (
    <Cut id="claude" title="Claude" hint={!st.on ? 'выключено' : st.write ? 'чтение и запись' : 'только чтение'}>
      <p className="small muted">«Полка» подключается к приложению Claude на этом компьютере как расширение. В разговоре можно спрашивать о прочитанном и просмотренном, а если разрешить запись — просить добавить книгу, поставить оценку или отметить серии.</p>
      <ul className="small muted claude-examples">
        {EXAMPLES.map((x) => (
          <li key={x}>«{x}»</li>
        ))}
      </ul>
      <label className="cloud-check">
        <input type="checkbox" role="switch" aria-label="Подключение для Claude" checked={st.on} onChange={(e) => set(e.target.checked, st.write, 'on')} />
        <span className="row-label">
          <span>Подключение для Claude</span>
          <span className="small muted">Claude читает полку, «Хочу», итоги года и цитаты — когда вы спросите</span>
        </span>
      </label>
      <label className="cloud-check sub" style={{ opacity: st.on ? 1 : 0.55 }}>
        <input type="checkbox" role="switch" aria-label="Разрешить запись" checked={st.write} disabled={!st.on} onChange={(e) => set(true, e.target.checked, 'write')} />
        <span className="row-label">
          <span>Разрешить запись</span>
          <span className="small muted">Добавлять записи, менять статус, оценку, метки и заметки, отмечать серии, удалять — каждое действие с кнопкой «Вернуть»</span>
        </span>
      </label>
      <div className="sec-actions">
        <button type="button" className="btn primary small" disabled={busy} onClick={() => bundle(true)}>
          <Icon name="plug" size={18} />
          <span>Установить в Claude</span>
        </button>
        <button type="button" className="btn line small" disabled={busy} onClick={() => bundle(false)}>
          Сохранить файл расширения
        </button>
      </div>
      <p className="small muted">
        «Установить в Claude» сохраняет «Полка для Claude.mcpb» в «Загрузки» и открывает его — приложение Claude предложит установить расширение. Или в приложении Claude: Настройки → Extensions → Advanced settings → Install Extension… Если «Полка» закрыта, Claude откроет её сам.
      </p>
      {saved && <p className="small muted backup-dir">Файл: {saved}</p>}
      <button type="button" className="link-btn small fold" aria-expanded={manual} onClick={() => setManual(!manual)}>
        <span>Ручная настройка</span>
        <Icon name={manual ? 'up' : 'down'} size={16} />
      </button>
      {manual && (
        <div className="stack-s">
          <p className="small muted">Если расширение не ставится: в приложении Claude — Настройки → Developer → Edit Config, вставьте этот текст в claude_desktop_config.json (или добавьте «polka» к уже описанным серверам) и перезапустите приложение Claude.</p>
          <pre className="code-box">{st.config}</pre>
          <div className="sec-actions">
            <button type="button" className="btn line small" onClick={copy}>
              Скопировать
            </button>
          </div>
        </div>
      )}
      {st.actions.length > 0 && (
        <div className="stack-s">
          <h3 className="label">Последние действия Claude</h3>
          {st.actions.map((a) => (
            <div key={a.id} className="row claude-action">
              <span className="row-label">
                <span className={a.undone ? 'muted3 struck' : ''}>{a.text}</span>
                <span className="small muted3">{whenText(a.at)}</span>
              </span>
              {a.undone ? (
                <span className="small muted3">отменено</span>
              ) : (
                <button type="button" className="link-btn small" onClick={() => undo(a.id)}>
                  <Icon name="undo" size={16} />
                  <span>Отменить</span>
                </button>
              )}
            </div>
          ))}
          <p className="small muted3">Список — с последнего запуска «Полки»; сами записи остаются на полке как обычные. Отменить нельзя, если после Claude запись уже меняли.</p>
        </div>
      )}
      <p className="small muted">
        «Полка» ничего не открывает в интернет: приложение Claude запускает polka.exe --mcp, а тот передаёт запросы запущенной «Полке» только внутри компьютера и с ключом, который «Полка» каждый раз создаёт заново. Claude видит только то, что запросит в разговоре по вашей просьбе, — это уходит в Anthropic вместе с разговором. Пока «Полка» заблокирована PIN-кодом, Claude ничего не получает. Ключ API не нужен: разговор идёт в вашем приложении Claude, по вашей подписке.
      </p>
    </Cut>
  );
}
