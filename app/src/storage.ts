/** Настройки вида на этом устройстве: localStorage, а если его нет — просто не запоминаем. */

export function stored(key: string, fallback: string): string {
  try {
    return localStorage.getItem(key) ?? fallback;
  } catch {
    return fallback;
  }
}

export function store(key: string, value: string) {
  try {
    localStorage.setItem(key, value);
  } catch {
    /* без хранилища просто не запомним */
  }
}
