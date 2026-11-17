// Связь по Wi-Fi — общий раздел из kit; здесь — слова «Полки».
import { WifiSection as Section, type WifiWords } from '@primus-fixer/kit/ui';
import { api, inApp, isMobileApp, message, scanQr } from '../api';

export { agoText } from '@primus-fixer/kit/ui';

const WORDS: WifiWords = { name: 'Полка', feminine: true, files: 'обложек', filesNote: 'Обложки едут вместе с записями.', where: 'Настройки → Обмен по Wi-Fi', port: 47510 };

/** Настройки → «Обмен по Wi-Fi»: связанные устройства, код связи, кто рядом. */
export default function WifiSection({ onSynced }: { onSynced: () => void }) {
  return <Section api={api} words={WORDS} mobile={isMobileApp} demo={!inApp} scanQr={scanQr} message={message} onSynced={onSynced} />;
}
