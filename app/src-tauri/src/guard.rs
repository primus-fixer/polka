//! Вход в «Полку»: PIN-код, отпечаток (Android) или Windows Hello, автоблокировка — как в «Кармане».
//!
//! Пока «Полка» заблокирована, команды с данными отвечают `locked`, и экран показывает вход.
//! Забытый PIN сбрасывается после проверки блокировкой устройства: экраном блокировки
//! телефона или Windows Hello.

use crate::{err, AppState, Res};
use polka_core::db::Store;
use polka_core::pin;
use serde::Serialize;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager};

/// Ответ команд с данными, пока «Полка» заблокирована.
pub const LOCKED: &str = "locked";
/// Через сколько в фоне снова спрашивать вход.
const LOCK_AFTER: Duration = Duration::from_secs(5 * 60);
/// Сколько действует подтверждение устройством для нового PIN-кода.
const RECOVERY: Duration = Duration::from_secs(5 * 60);

#[derive(Default)]
pub struct Guard {
    pub locked: bool,
    pin: bool,
    hidden_since: Option<Instant>,
    recovery_until: Option<Instant>,
}

impl Guard {
    pub fn new(pin: bool) -> Self {
        Guard { locked: pin, pin, ..Default::default() }
    }

    fn recovering(&self) -> bool {
        self.recovery_until.is_some_and(|t| Instant::now() < t)
    }
}

fn lock_after() -> Duration {
    // Для сквозных проверок в отладочной сборке время можно сократить.
    #[cfg(debug_assertions)]
    if let Some(s) = std::env::var("POLKA_LOCK_AFTER_SECS").ok().and_then(|v| v.parse().ok()) {
        return Duration::from_secs(s);
    }
    LOCK_AFTER
}

#[derive(Serialize)]
pub struct LockStatus {
    /// PIN-код задан.
    pin: bool,
    locked: bool,
    /// Вход по отпечатку или Windows Hello включён.
    bio: bool,
    /// Отпечаток или Windows Hello доступны на устройстве сейчас.
    bio_ready: bool,
    /// fingerprint, face или hello.
    bio_kind: &'static str,
    /// Чем подтвердить себя, если PIN забыт: device (экран блокировки телефона), hello или пусто.
    recover: &'static str,
    /// Секунд до следующей попытки ввода PIN.
    wait: u64,
    /// Сколько ещё можно ошибиться до паузы.
    left: u32,
    /// Устройство подтвердило владельца — можно задать новый PIN без старого.
    recovery: bool,
    /// Чем защищён ключ базы: dpapi, android-keystore или file.
    key: &'static str,
    error: Option<String>,
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn meta_u64(st: &Store, key: &str) -> u64 {
    st.meta(key).ok().flatten().and_then(|v| v.parse().ok()).unwrap_or(0)
}

pub fn has_pin(st: &Store) -> bool {
    st.meta(pin::META_HASH).ok().flatten().is_some_and(|h| !h.is_empty())
}

fn reset_fails(st: &Store) -> Res<()> {
    st.set_meta(pin::META_FAILS, "0").map_err(err)?;
    st.set_meta(pin::META_UNTIL, "0").map_err(err)
}

/// Проверяет PIN с учётом пауз после ошибок. Ошибка — текст для экрана.
fn check_pin(st: &Store, code: &str) -> Result<(), String> {
    let now = now_secs();
    if meta_u64(st, pin::META_UNTIL) > now {
        return Err("Слишком много попыток, подождите".into());
    }
    let Some(hash) = st.meta(pin::META_HASH).map_err(err)?.filter(|h| !h.is_empty()) else {
        return Err("PIN-код не задан".into());
    };
    if pin::verify(code, &hash) {
        return reset_fails(st);
    }
    let fails = meta_u64(st, pin::META_FAILS) as u32 + 1;
    st.set_meta(pin::META_FAILS, &fails.to_string()).map_err(err)?;
    let pause = pin::pause_after(fails);
    if pause > 0 {
        st.set_meta(pin::META_UNTIL, &(now + pause).to_string()).map_err(err)?;
    }
    Err("Неверный PIN-код".into())
}

/// Что умеет устройство: отпечаток или Windows Hello и сброс PIN.
struct Bio {
    ready: bool,
    kind: &'static str,
    recover: &'static str,
}

/// Только для сквозных проверок на Linux в отладочной сборке: POLKA_FAKE_AUTH — отпечаток,
/// POLKA_FAKE_DEVICE — блокировка устройства; значение `cancel` — человек отменил проверку.
#[cfg(all(debug_assertions, not(any(mobile, windows))))]
fn fake_auth(device: bool) -> Option<String> {
    let own = if device { std::env::var("POLKA_FAKE_DEVICE").ok() } else { None };
    own.or_else(|| std::env::var("POLKA_FAKE_AUTH").ok())
}

#[allow(unused_variables)]
async fn bio_info(app: &AppHandle) -> Bio {
    #[cfg(mobile)]
    {
        use tauri_plugin_biometric::{BiometricExt, BiometryType};
        let a = app.clone();
        // В фоновом потоке: вызов плагина ждёт ответа от Android.
        let status = tauri::async_runtime::spawn_blocking(move || a.biometric().status().ok().map(|s| (s.is_available, matches!(s.biometry_type, BiometryType::FaceID)))).await;
        let (ready, kind) = match status {
            Ok(Some((true, face))) => (true, if face { "face" } else { "fingerprint" }),
            _ => (false, ""),
        };
        Bio { ready, kind, recover: "device" }
    }
    #[cfg(windows)]
    {
        let ready = tauri::async_runtime::spawn_blocking(polka_secure::hello_available).await.unwrap_or(false);
        Bio { ready, kind: "hello", recover: if ready { "hello" } else { "" } }
    }
    #[cfg(not(any(mobile, windows)))]
    {
        #[cfg(debug_assertions)]
        if fake_auth(false).is_some() {
            return Bio { ready: true, kind: "fingerprint", recover: "device" };
        }
        Bio { ready: false, kind: "", recover: "" }
    }
}

/// Проверка личности системой. `device` разрешает экран блокировки телефона (сброс PIN).
/// Ok(false) — человек отменил проверку.
#[allow(unused_variables)]
async fn system_auth(app: &AppHandle, reason: &str, device: bool) -> Res<bool> {
    #[cfg(mobile)]
    {
        use tauri::plugin::mobile::PluginInvokeError;
        use tauri_plugin_biometric::{AuthOptions, BiometricExt, Error};
        let a = app.clone();
        let reason = reason.to_string();
        let options = AuthOptions {
            allow_device_credential: device,
            cancel_title: Some(if device { "Отмена".into() } else { "Ввести PIN".into() }),
            title: Some("Полка".into()),
            confirmation_required: Some(false),
            ..Default::default()
        };
        // Ответ переводим в простые строки ещё в фоновом потоке.
        let result: Result<(), (Option<String>, String)> = tauri::async_runtime::spawn_blocking(move || match a.biometric().authenticate(reason, options) {
            Ok(()) => Ok(()),
            Err(Error::PluginInvoke(PluginInvokeError::InvokeRejected(e))) => Err((e.code, e.message.unwrap_or_default())),
            Err(e) => Err((None, e.to_string())),
        })
        .await
        .map_err(err)?;
        match result {
            Ok(()) => Ok(true),
            Err((code, msg)) => match code.as_deref() {
                Some("userCancel" | "systemCancel" | "appCancel" | "authenticationFailed") => Ok(false),
                Some("passcodeNotSet") => Err("На телефоне не задана блокировка экрана. Включите её в настройках телефона и повторите.".into()),
                Some("biometryLockout") => Err("Слишком много попыток отпечатка. Введите PIN-код или попробуйте позже.".into()),
                Some("biometryNotEnrolled") => Err("На телефоне не добавлен отпечаток пальца.".into()),
                Some("biometryNotAvailable") => Err("Сканер отпечатков сейчас недоступен.".into()),
                _ => Err(if msg.is_empty() { "Проверка не удалась".into() } else { msg }),
            },
        }
    }
    #[cfg(windows)]
    {
        if !tauri::async_runtime::spawn_blocking(polka_secure::hello_available).await.unwrap_or(false) {
            return Err("Windows Hello не настроен. Включите его: Параметры → Учётные записи → Варианты входа — и повторите.".into());
        }
        let window = app.get_webview_window("main").ok_or("нет окна приложения")?;
        let hwnd = window.hwnd().map_err(err)?.0 as isize;
        let reason = reason.to_string();
        tauri::async_runtime::spawn_blocking(move || polka_secure::hello_verify(hwnd, &reason)).await.map_err(err)?.map_err(err)
    }
    #[cfg(not(any(mobile, windows)))]
    {
        #[cfg(debug_assertions)]
        if let Some(mode) = fake_auth(device) {
            return Ok(mode != "cancel");
        }
        Err("Проверка отпечатком на этой системе недоступна".into())
    }
}

fn status_of(state: &AppState, bio: &Bio, error: Option<String>) -> Res<LockStatus> {
    let g = state.guard.lock().map_err(err)?;
    let st = state.store.lock().map_err(err)?;
    let fails = meta_u64(&st, pin::META_FAILS) as u32;
    Ok(LockStatus {
        pin: g.pin,
        locked: g.locked,
        bio: g.pin && st.meta(pin::META_BIO).ok().flatten().as_deref() == Some("1"),
        bio_ready: bio.ready,
        bio_kind: bio.kind,
        recover: bio.recover,
        wait: meta_u64(&st, pin::META_UNTIL).saturating_sub(now_secs()),
        left: pin::tries_left(fails),
        recovery: g.recovering(),
        key: polka_secure::method(),
        error,
    })
}

async fn reply(app: &AppHandle, error: Option<String>) -> Res<LockStatus> {
    let bio = bio_info(app).await;
    status_of(&app.state::<AppState>(), &bio, error)
}

/// Работа с базой в фоновом потоке: Argon2 не должен держать главный поток.
async fn blocking<T: Send + 'static>(app: &AppHandle, f: impl FnOnce(&AppState) -> Res<T> + Send + 'static) -> Res<T> {
    let a = app.clone();
    tauri::async_runtime::spawn_blocking(move || f(&a.state::<AppState>())).await.map_err(err)?
}

fn unlock(state: &AppState) -> Res<()> {
    let mut g = state.guard.lock().map_err(err)?;
    g.locked = false;
    g.hidden_since = None;
    Ok(())
}

#[tauri::command]
pub async fn lock_status(app: AppHandle) -> Res<LockStatus> {
    reply(&app, None).await
}

#[tauri::command]
pub async fn pin_unlock(app: AppHandle, code: String) -> Res<LockStatus> {
    let error = blocking(&app, move |state| {
        let st = state.store.lock().map_err(err)?;
        match check_pin(&st, &code) {
            Ok(()) => {
                drop(st);
                unlock(state)?;
                Ok(None)
            }
            Err(e) => Ok(Some(e)),
        }
    })
    .await?;
    reply(&app, error).await
}

/// Новый PIN. Если PIN уже есть, нужен текущий — или подтверждение устройством («Забыли PIN?»).
#[tauri::command]
pub async fn pin_set(app: AppHandle, code: String, current: Option<String>) -> Res<LockStatus> {
    if !pin::valid(&code) {
        return Err(format!("PIN-код — {} цифры", pin::PIN_LEN));
    }
    let error = blocking(&app, move |state| {
        let recovering = state.guard.lock().map_err(err)?.recovering();
        let st = state.store.lock().map_err(err)?;
        if has_pin(&st) && !recovering {
            let Some(current) = current else { return Ok(Some("Введите текущий PIN-код".to_string())) };
            if let Err(e) = check_pin(&st, &current) {
                return Ok(Some(e));
            }
        }
        st.set_meta(pin::META_HASH, &pin::hash(&code).map_err(err)?).map_err(err)?;
        reset_fails(&st)?;
        drop(st);
        let mut g = state.guard.lock().map_err(err)?;
        g.pin = true;
        g.locked = false;
        g.recovery_until = None;
        Ok(None)
    })
    .await?;
    reply(&app, error).await
}

/// Выключает вход по PIN-коду (и отпечатку). Нужен текущий PIN.
#[tauri::command]
pub async fn pin_off(app: AppHandle, current: Option<String>) -> Res<LockStatus> {
    let error = blocking(&app, move |state| {
        let recovering = state.guard.lock().map_err(err)?.recovering();
        let st = state.store.lock().map_err(err)?;
        if !recovering {
            let Some(current) = current else { return Ok(Some("Введите текущий PIN-код".to_string())) };
            if let Err(e) = check_pin(&st, &current) {
                return Ok(Some(e));
            }
        }
        st.set_meta(pin::META_HASH, "").map_err(err)?;
        st.set_meta(pin::META_BIO, "0").map_err(err)?;
        reset_fails(&st)?;
        drop(st);
        let mut g = state.guard.lock().map_err(err)?;
        g.pin = false;
        g.locked = false;
        g.recovery_until = None;
        Ok(None)
    })
    .await?;
    reply(&app, error).await
}

#[tauri::command]
pub async fn bio_unlock(app: AppHandle) -> Res<LockStatus> {
    let enabled = {
        let state = app.state::<AppState>();
        let st = state.store.lock().map_err(err)?;
        st.meta(pin::META_BIO).map_err(err)?.as_deref() == Some("1")
    };
    if !enabled {
        return reply(&app, Some("Вход по отпечатку выключен".into())).await;
    }
    let error = match system_auth(&app, "Вход в «Полку»", false).await {
        Ok(true) => {
            let state = app.state::<AppState>();
            reset_fails(&*state.store.lock().map_err(err)?)?;
            unlock(&state)?;
            None
        }
        Ok(false) => None,
        Err(e) => Some(e),
    };
    reply(&app, error).await
}

/// Включает или выключает вход по отпечатку. Включение сразу проверяет отпечаток.
#[tauri::command]
pub async fn bio_set(app: AppHandle, on: bool) -> Res<LockStatus> {
    {
        let state = app.state::<AppState>();
        let g = state.guard.lock().map_err(err)?;
        if g.locked {
            return Err(LOCKED.into());
        }
        if on && !g.pin {
            return Err("Сначала задайте PIN-код".into());
        }
    }
    let mut error = None;
    let allowed = if on {
        match system_auth(&app, "Включить вход по отпечатку", false).await {
            Ok(ok) => ok,
            Err(e) => {
                error = Some(e);
                false
            }
        }
    } else {
        true
    };
    if allowed {
        let state = app.state::<AppState>();
        state.store.lock().map_err(err)?.set_meta(pin::META_BIO, if on { "1" } else { "0" }).map_err(err)?;
    }
    reply(&app, error).await
}

/// «Забыли PIN?»: проверка блокировкой устройства, после неё можно задать новый PIN.
#[tauri::command]
pub async fn pin_recover(app: AppHandle) -> Res<LockStatus> {
    let error = match system_auth(&app, "Сброс PIN-кода «Полки»", true).await {
        Ok(true) => {
            let state = app.state::<AppState>();
            reset_fails(&*state.store.lock().map_err(err)?)?;
            let mut g = state.guard.lock().map_err(err)?;
            g.locked = false;
            g.hidden_since = None;
            g.recovery_until = Some(Instant::now() + RECOVERY);
            None
        }
        Ok(false) => None,
        Err(e) => Some(e),
    };
    reply(&app, error).await
}

#[tauri::command]
pub async fn lock_now(app: AppHandle) -> Res<LockStatus> {
    {
        let state = app.state::<AppState>();
        let mut g = state.guard.lock().map_err(err)?;
        if g.pin {
            g.locked = true;
            drop(g);
            crate::lan::on_lock(&state);
        }
    }
    reply(&app, None).await
}

/// Экран сообщает, что приложение свернули или снова открыли (в дополнение к фокусу окна).
#[tauri::command]
pub async fn app_visible(app: AppHandle, visible: bool) -> Res<LockStatus> {
    on_focus(&app, visible);
    reply(&app, None).await
}

/// Окно потеряло или получило фокус: через 5 минут в фоне «Полка» блокируется.
pub fn on_focus(app: &AppHandle, focused: bool) {
    let state = app.state::<AppState>();
    let Ok(mut g) = state.guard.lock() else { return };
    if !focused {
        if g.hidden_since.is_none() {
            g.hidden_since = Some(Instant::now());
        }
        return;
    }
    // Вернулись в «Полку» — заодно обменяться с облаком, если давно не обменивались.
    crate::cloud::resumed();
    if let Some(since) = g.hidden_since.take() {
        if g.pin && !g.locked && since.elapsed() >= lock_after() {
            g.locked = true;
            drop(g);
            // Пока «Полка» заблокирована, по прежнему QR-коду не подключиться.
            crate::lan::on_lock(&state);
            let _ = app.emit("locked", ());
        }
    }
}
