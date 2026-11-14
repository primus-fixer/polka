//! Android: ключ базы шифруется ключом AES-256-GCM из Android Keystore.
//! Сам ключ Keystore не покидает защищённое хранилище телефона, а при удалении
//! приложения удаляется вместе с ним.

use crate::{Error, Result};
use jni::objects::{JByteArray, JObject, JValue};
use jni::{JNIEnv, JavaVM};

const ALIAS: &str = "polka-db-key";
/// Формат: версия (1 байт), IV (12 байт), шифртекст с меткой GCM.
const VERSION: u8 = 1;
const IV_LEN: usize = 12;

fn jerr(env: &mut JNIEnv, e: jni::errors::Error) -> Error {
    // Исключение Java нужно снять, иначе следующие вызовы JNI упадут.
    if env.exception_check().unwrap_or(false) {
        let _ = env.exception_describe();
        let _ = env.exception_clear();
    }
    Error::Store(e.to_string())
}

fn with_env<T>(f: impl FnOnce(&mut JNIEnv) -> Result<T>) -> Result<T> {
    let ctx = ndk_context::android_context();
    let vm = unsafe { JavaVM::from_raw(ctx.vm().cast()) }.map_err(|e| Error::Store(e.to_string()))?;
    let mut env = vm.attach_current_thread().map_err(|e| Error::Store(e.to_string()))?;
    f(&mut env)
}

/// Ключ из Android Keystore; при первом вызове создаётся.
fn secret_key<'l>(env: &mut JNIEnv<'l>) -> jni::errors::Result<JObject<'l>> {
    let provider = env.new_string("AndroidKeyStore")?;
    let ks = env
        .call_static_method("java/security/KeyStore", "getInstance", "(Ljava/lang/String;)Ljava/security/KeyStore;", &[JValue::Object(&provider)])?
        .l()?;
    env.call_method(&ks, "load", "(Ljava/security/KeyStore$LoadStoreParameter;)V", &[JValue::Object(&JObject::null())])?;
    let alias = env.new_string(ALIAS)?;
    let exists = env.call_method(&ks, "containsAlias", "(Ljava/lang/String;)Z", &[JValue::Object(&alias)])?.z()?;
    if !exists {
        // new KeyGenParameterSpec.Builder(alias, PURPOSE_ENCRYPT | PURPOSE_DECRYPT)
        let builder = env.new_object("android/security/keystore/KeyGenParameterSpec$Builder", "(Ljava/lang/String;I)V", &[JValue::Object(&alias), JValue::Int(3)])?;
        let gcm = env.new_string("GCM")?;
        let modes = env.new_object_array(1, "java/lang/String", &gcm)?;
        env.call_method(&builder, "setBlockModes", "([Ljava/lang/String;)Landroid/security/keystore/KeyGenParameterSpec$Builder;", &[JValue::Object(&modes)])?;
        let none = env.new_string("NoPadding")?;
        let paddings = env.new_object_array(1, "java/lang/String", &none)?;
        env.call_method(&builder, "setEncryptionPaddings", "([Ljava/lang/String;)Landroid/security/keystore/KeyGenParameterSpec$Builder;", &[JValue::Object(&paddings)])?;
        env.call_method(&builder, "setKeySize", "(I)Landroid/security/keystore/KeyGenParameterSpec$Builder;", &[JValue::Int(256)])?;
        let spec = env.call_method(&builder, "build", "()Landroid/security/keystore/KeyGenParameterSpec;", &[])?.l()?;
        let aes = env.new_string("AES")?;
        let generator = env
            .call_static_method("javax/crypto/KeyGenerator", "getInstance", "(Ljava/lang/String;Ljava/lang/String;)Ljavax/crypto/KeyGenerator;", &[JValue::Object(&aes), JValue::Object(&provider)])?
            .l()?;
        env.call_method(&generator, "init", "(Ljava/security/spec/AlgorithmParameterSpec;)V", &[JValue::Object(&spec)])?;
        env.call_method(&generator, "generateKey", "()Ljavax/crypto/SecretKey;", &[])?;
    }
    env.call_method(&ks, "getKey", "(Ljava/lang/String;[C)Ljava/security/Key;", &[JValue::Object(&alias), JValue::Object(&JObject::null())])?.l()
}

fn cipher<'l>(env: &mut JNIEnv<'l>) -> jni::errors::Result<JObject<'l>> {
    let name = env.new_string("AES/GCM/NoPadding")?;
    env.call_static_method("javax/crypto/Cipher", "getInstance", "(Ljava/lang/String;)Ljavax/crypto/Cipher;", &[JValue::Object(&name)])?.l()
}

fn encrypt(env: &mut JNIEnv, secret: &[u8]) -> jni::errors::Result<Vec<u8>> {
    let key = secret_key(env)?;
    let c = cipher(env)?;
    // Cipher.ENCRYPT_MODE = 1; IV выбирает Keystore.
    env.call_method(&c, "init", "(ILjava/security/Key;)V", &[JValue::Int(1), JValue::Object(&key)])?;
    let iv: JByteArray = env.call_method(&c, "getIV", "()[B", &[])?.l()?.into();
    let iv = env.convert_byte_array(&iv)?;
    let input = env.byte_array_from_slice(secret)?;
    let out: JByteArray = env.call_method(&c, "doFinal", "([B)[B", &[JValue::Object(&input)])?.l()?.into();
    let out = env.convert_byte_array(&out)?;
    let mut blob = Vec::with_capacity(1 + iv.len() + out.len());
    blob.push(VERSION);
    blob.extend_from_slice(&iv);
    blob.extend_from_slice(&out);
    Ok(blob)
}

fn decrypt(env: &mut JNIEnv, iv: &[u8], data: &[u8]) -> jni::errors::Result<Vec<u8>> {
    let key = secret_key(env)?;
    let c = cipher(env)?;
    let iv = env.byte_array_from_slice(iv)?;
    let spec = env.new_object("javax/crypto/spec/GCMParameterSpec", "(I[B)V", &[JValue::Int(128), JValue::Object(&iv)])?;
    // Cipher.DECRYPT_MODE = 2
    env.call_method(&c, "init", "(ILjava/security/Key;Ljava/security/spec/AlgorithmParameterSpec;)V", &[JValue::Int(2), JValue::Object(&key), JValue::Object(&spec)])?;
    let input = env.byte_array_from_slice(data)?;
    let out: JByteArray = env.call_method(&c, "doFinal", "([B)[B", &[JValue::Object(&input)])?.l()?.into();
    env.convert_byte_array(&out)
}

pub fn protect(secret: &[u8]) -> Result<Vec<u8>> {
    with_env(|env| encrypt(env, secret).map_err(|e| jerr(env, e)))
}

pub fn unprotect(blob: &[u8]) -> Result<Vec<u8>> {
    if blob.len() <= 1 + IV_LEN || blob[0] != VERSION {
        return Err(Error::Format);
    }
    let (iv, data) = blob[1..].split_at(IV_LEN);
    with_env(|env| decrypt(env, iv, data).map_err(|e| jerr(env, e)))
}
