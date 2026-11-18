// Облако — общий раздел из kit; здесь — слова «Полки».
import { CloudSection as Section, type CloudWords } from '@primus-fixer/kit/ui';
import { api, inApp, message } from '../api';

export { lastText, stepText } from '@primus-fixer/kit/ui';

export const CLOUD_WORDS: CloudWords = {
  name: 'Полка',
  nameGen: 'Полки',
  feminine: true,
  what: 'Записи и обложки',
  blobs: { many: 'обложек', upload: 'Отправляю обложки', download: 'Получаю обложки', waiting: ['обложка придёт', 'обложки придут', 'обложек придут'] },
  whole: { label: 'Файлы книг', many: 'файлов книг', waiting: ['файл книги придёт', 'файла книг придут', 'файлов книг придут'] },
};

/** Настройки → «Облако»: Яндекс Диск по паролю приложения или свой сервер WebDAV. */
export default function CloudSection() {
  return <Section api={api} words={CLOUD_WORDS} message={message} demo={!inApp} />;
}
