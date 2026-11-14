//! Windows: DPAPI для ключа и Windows Hello для проверки личности.

use crate::{Error, Result};
use windows::core::{factory, w, HSTRING};
use windows::Security::Credentials::UI::{UserConsentVerificationResult, UserConsentVerifier, UserConsentVerifierAvailability};
use windows::Win32::Foundation::{LocalFree, HLOCAL, HWND};
use windows::Win32::Security::Cryptography::{CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB};
use windows::Win32::System::WinRT::IUserConsentVerifierInterop;
use windows_future::IAsyncOperation;

/// Дополнительная «соль» DPAPI: чужая программа под той же учётной записью
/// без неё ключ не расшифрует простым вызовом.
const ENTROPY: &[u8] = b"polka-db-key-v1";

fn err(e: windows::core::Error) -> Error {
    Error::Store(e.message().to_string())
}

fn blob(data: &[u8]) -> CRYPT_INTEGER_BLOB {
    CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 }
}

/// Копирует результат DPAPI и освобождает память, которую выделила система.
fn take(out: CRYPT_INTEGER_BLOB) -> Vec<u8> {
    if out.pbData.is_null() {
        return Vec::new();
    }
    let v = unsafe { std::slice::from_raw_parts(out.pbData, out.cbData as usize) }.to_vec();
    unsafe {
        let _ = LocalFree(Some(HLOCAL(out.pbData as *mut core::ffi::c_void)));
    }
    v
}

pub fn protect(secret: &[u8]) -> Result<Vec<u8>> {
    let input = blob(secret);
    let entropy = blob(ENTROPY);
    let mut out = CRYPT_INTEGER_BLOB::default();
    unsafe { CryptProtectData(&input, w!("Polka"), Some(&entropy as *const CRYPT_INTEGER_BLOB), None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut out) }.map_err(err)?;
    Ok(take(out))
}

pub fn unprotect(data: &[u8]) -> Result<Vec<u8>> {
    let input = blob(data);
    let entropy = blob(ENTROPY);
    let mut out = CRYPT_INTEGER_BLOB::default();
    unsafe { CryptUnprotectData(&input, None, Some(&entropy as *const CRYPT_INTEGER_BLOB), None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut out) }.map_err(err)?;
    Ok(take(out))
}

pub fn hello_available() -> bool {
    UserConsentVerifier::CheckAvailabilityAsync()
        .and_then(|op| op.join())
        .map(|a| a == UserConsentVerifierAvailability::Available)
        .unwrap_or(false)
}

pub fn hello_verify(hwnd: isize, message: &str) -> Result<bool> {
    let interop = factory::<UserConsentVerifier, IUserConsentVerifierInterop>().map_err(err)?;
    let op: IAsyncOperation<UserConsentVerificationResult> =
        unsafe { interop.RequestVerificationForWindowAsync(HWND(hwnd as *mut core::ffi::c_void), &HSTRING::from(message)) }.map_err(err)?;
    Ok(op.join().map_err(err)? == UserConsentVerificationResult::Verified)
}
