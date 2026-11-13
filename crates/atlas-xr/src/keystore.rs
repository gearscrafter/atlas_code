//! Token encryption with the Android Keystore (AES-256-GCM).
//!
//! The key is generated inside the Keystore and **cannot be exported**: neither the app nor
//! whoever copies its files has it; they can only ask the system to encrypt or
//! decrypt. On hardware with a TEE (like the Quest) the key lives in it.
//!
//! Format of the encrypted data: `[IV length: 1 byte][IV][ciphertext + GCM tag]`.

use jni::errors::Result as JniResult;
use jni::objects::{JObject, JValue};
use jni::{jni_sig, jni_str, Env, JavaVM};

use atlas_git::SecretBox;

/// Name of the key inside the Keystore.
const ALIAS: &str = "atlas_code_credenciales_v1";
const ENCRYPT_MODE: i32 = 1;
const DECRYPT_MODE: i32 = 2;
/// `KeyProperties.PURPOSE_ENCRYPT | PURPOSE_DECRYPT`.
const PURPOSES: i32 = 1 | 2;
/// GCM authentication tag, in bits.
const GCM_TAG_BITS: i32 = 128;

pub struct AndroidKeystore;

/// Runs `f` with a JNI `Env`, translating the errors (and the Java
/// exceptions, which `attach_current_thread` catches) into a readable message.
fn with_env<T>(f: impl FnOnce(&mut Env) -> JniResult<T>) -> Result<T, String> {
    let vm = JavaVM::singleton().map_err(|e| format!("no JVM available: {e}"))?;
    vm.attach_current_thread(f).map_err(|e| format!("Keystore: {e}"))
}

/// Returns the Keystore's AES key, creating it the first time.
fn secret_key<'l>(env: &mut Env<'l>) -> JniResult<JObject<'l>> {
    let store_type = env.new_string("AndroidKeyStore")?;
    let store = env
        .call_static_method(
            jni_str!("java/security/KeyStore"),
            jni_str!("getInstance"),
            jni_sig!("(Ljava/lang/String;)Ljava/security/KeyStore;"),
            &[JValue::Object(&store_type)],
        )?
        .l()?;
    let null = JObject::null();
    env.call_method(
        &store,
        jni_str!("load"),
        jni_sig!("(Ljava/security/KeyStore$LoadStoreParameter;)V"),
        &[JValue::Object(&null)],
    )?
    .v()?;

    let alias = env.new_string(ALIAS)?;
    let exists = env
        .call_method(
            &store,
            jni_str!("containsAlias"),
            jni_sig!("(Ljava/lang/String;)Z"),
            &[JValue::Object(&alias)],
        )?
        .z()?;
    if !exists {
        generate_key(env, &alias)?;
    }

    env.call_method(
        &store,
        jni_str!("getKey"),
        jni_sig!("(Ljava/lang/String;[C)Ljava/security/Key;"),
        &[JValue::Object(&alias), JValue::Object(&null)],
    )?
    .l()
}

/// Creates the key: AES-256, GCM mode, no padding, only for encrypting and decrypting.
fn generate_key(env: &mut Env, alias: &JObject) -> JniResult<()> {
    let builder = env.new_object(
        jni_str!("android/security/keystore/KeyGenParameterSpec$Builder"),
        jni_sig!("(Ljava/lang/String;I)V"),
        &[JValue::Object(alias), JValue::Int(PURPOSES)],
    )?;

    // `setBlockModes` and `setEncryptionPaddings` take `String...`.
    let null = JObject::null();
    let gcm = env.new_string("GCM")?;
    let modes = env.new_object_array(1, jni_str!("java/lang/String"), &null)?;
    modes.set_element(env, 0, &gcm)?;
    let builder = env
        .call_method(
            &builder,
            jni_str!("setBlockModes"),
            jni_sig!("([Ljava/lang/String;)Landroid/security/keystore/KeyGenParameterSpec$Builder;"),
            &[JValue::Object(&modes)],
        )?
        .l()?;

    let no_padding = env.new_string("NoPadding")?;
    let paddings = env.new_object_array(1, jni_str!("java/lang/String"), &null)?;
    paddings.set_element(env, 0, &no_padding)?;
    let builder = env
        .call_method(
            &builder,
            jni_str!("setEncryptionPaddings"),
            jni_sig!("([Ljava/lang/String;)Landroid/security/keystore/KeyGenParameterSpec$Builder;"),
            &[JValue::Object(&paddings)],
        )?
        .l()?;

    let builder = env
        .call_method(
            &builder,
            jni_str!("setKeySize"),
            jni_sig!("(I)Landroid/security/keystore/KeyGenParameterSpec$Builder;"),
            &[JValue::Int(256)],
        )?
        .l()?;
    let spec = env
        .call_method(
            &builder,
            jni_str!("build"),
            jni_sig!("()Landroid/security/keystore/KeyGenParameterSpec;"),
            &[],
        )?
        .l()?;

    let algorithm = env.new_string("AES")?;
    let provider = env.new_string("AndroidKeyStore")?;
    let generator = env
        .call_static_method(
            jni_str!("javax/crypto/KeyGenerator"),
            jni_str!("getInstance"),
            jni_sig!("(Ljava/lang/String;Ljava/lang/String;)Ljavax/crypto/KeyGenerator;"),
            &[JValue::Object(&algorithm), JValue::Object(&provider)],
        )?
        .l()?;
    env.call_method(
        &generator,
        jni_str!("init"),
        jni_sig!("(Ljava/security/spec/AlgorithmParameterSpec;)V"),
        &[JValue::Object(&spec)],
    )?
    .v()?;
    env.call_method(&generator, jni_str!("generateKey"), jni_sig!("()Ljavax/crypto/SecretKey;"), &[])?
        .l()?;
    Ok(())
}

fn new_cipher<'l>(env: &mut Env<'l>) -> JniResult<JObject<'l>> {
    let transformation = env.new_string("AES/GCM/NoPadding")?;
    env.call_static_method(
        jni_str!("javax/crypto/Cipher"),
        jni_str!("getInstance"),
        jni_sig!("(Ljava/lang/String;)Ljavax/crypto/Cipher;"),
        &[JValue::Object(&transformation)],
    )?
    .l()
}

fn seal_with(env: &mut Env, plain: &[u8]) -> JniResult<Vec<u8>> {
    let key = secret_key(env)?;
    let cipher = new_cipher(env)?;
    env.call_method(
        &cipher,
        jni_str!("init"),
        jni_sig!("(ILjava/security/Key;)V"),
        &[JValue::Int(ENCRYPT_MODE), JValue::Object(&key)],
    )?
    .v()?;

    let iv = env.call_method(&cipher, jni_str!("getIV"), jni_sig!("()[B"), &[])?.l()?;
    let iv = env.cast_local::<jni::objects::JByteArray>(iv)?;
    let iv = env.convert_byte_array(&iv)?;

    let input = env.byte_array_from_slice(plain)?;
    let output = env
        .call_method(&cipher, jni_str!("doFinal"), jni_sig!("([B)[B"), &[JValue::Object(&input)])?
        .l()?;
    let output = env.cast_local::<jni::objects::JByteArray>(output)?;
    let output = env.convert_byte_array(&output)?;

    let mut sealed = Vec::with_capacity(1 + iv.len() + output.len());
    sealed.push(iv.len() as u8);
    sealed.extend_from_slice(&iv);
    sealed.extend_from_slice(&output);
    Ok(sealed)
}

fn open_with(env: &mut Env, sealed: &[u8], iv_len: usize) -> JniResult<Vec<u8>> {
    let (iv, body) = sealed[1..].split_at(iv_len);
    let key = secret_key(env)?;
    let cipher = new_cipher(env)?;

    let iv_array = env.byte_array_from_slice(iv)?;
    let spec = env.new_object(
        jni_str!("javax/crypto/spec/GCMParameterSpec"),
        jni_sig!("(I[B)V"),
        &[JValue::Int(GCM_TAG_BITS), JValue::Object(&iv_array)],
    )?;
    env.call_method(
        &cipher,
        jni_str!("init"),
        jni_sig!("(ILjava/security/Key;Ljava/security/spec/AlgorithmParameterSpec;)V"),
        &[JValue::Int(DECRYPT_MODE), JValue::Object(&key), JValue::Object(&spec)],
    )?
    .v()?;

    let input = env.byte_array_from_slice(body)?;
    let output = env
        .call_method(&cipher, jni_str!("doFinal"), jni_sig!("([B)[B"), &[JValue::Object(&input)])?
        .l()?;
    let output = env.cast_local::<jni::objects::JByteArray>(output)?;
    env.convert_byte_array(&output)
}

impl SecretBox for AndroidKeystore {
    fn seal(&self, plain: &[u8]) -> Result<Vec<u8>, String> {
        with_env(|env| seal_with(env, plain))
    }

    fn open(&self, sealed: &[u8]) -> Result<Vec<u8>, String> {
        let iv_len = *sealed.first().ok_or("empty encrypted data")? as usize;
        if sealed.len() < 1 + iv_len + 16 {
            return Err("datos cifrados demasiado cortos".into());
        }
        with_env(|env| open_with(env, sealed, iv_len))
    }
}

/// Checks that the Keystore really works: it encrypts and decrypts a test value.
/// If it fails, `atlas_git` is left without a cipher and refuses to save tokens in plain text.
pub fn self_test() -> Result<(), String> {
    let ks = AndroidKeystore;
    let sample = b"atlas-keystore-self-test";
    let sealed = ks.seal(sample)?;
    if sealed.windows(sample.len()).any(|w| w == sample) {
        return Err("the ciphertext contains the original".into());
    }
    let opened = ks.open(&sealed)?;
    if opened != sample {
        return Err("decrypting does not return the original".into());
    }
    Ok(())
}
