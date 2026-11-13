//! Bridge to Meta's system keyboard. The app is a `NativeActivity` without any text
//! field, and Meta's keyboard refuses to come up without one. That is why a Java helper is loaded at
//! runtime (`crates/atlas-xr/java`, precompiled to `assets/atlas_input.dex`) that creates the
//! text field and collects what is typed; here its three functions are just called.

use std::ffi::c_void;

use jni::errors::Result as JniResult;
use jni::objects::{JClass, JObject, JString, JValue};
use jni::refs::Global;
use jni::sys::jobject;
use jni::{jni_sig, jni_str, Env, JavaVM};

/// The helper, already compiled. It lives for the whole run (the class loader reads it from here).
static DEX: &[u8] = include_bytes!("../assets/atlas_input.dex");
const CLASS: &str = "com.atlascode.xr.AtlasInput";

/// What was typed since the last read: characters as they are, `\u{8}` to delete and `\n` for Enter.
pub struct SysInput {
    class: Global<JClass<'static>>,
}

/// Runs `f` with a JNI `Env`, translating the errors into a readable message.
fn with_env<T>(f: impl FnOnce(&mut Env) -> JniResult<T>) -> Result<T, String> {
    let vm = JavaVM::singleton().map_err(|e| format!("no JVM available: {e}"))?;
    vm.attach_current_thread(f).map_err(|e| format!("system keyboard: {e}"))
}

impl SysInput {
    /// Loads the helper. `activity` is the app's activity (`AndroidApp::activity_as_ptr`).
    pub fn new(activity: *mut c_void) -> Result<Self, String> {
        with_env(|env| {
            let activity = unsafe { JObject::from_raw(env, activity as jobject) };
            let parent = env
                .call_method(&activity, jni_str!("getClassLoader"), jni_sig!("()Ljava/lang/ClassLoader;"), &[])?
                .l()?;
            // The .dex is read directly from the app's memory, without writing any file.
            let buffer = unsafe { env.new_direct_byte_buffer(DEX.as_ptr() as *mut u8, DEX.len())? };
            let buffer = JObject::from(buffer);
            let loader = env.new_object(
                jni_str!("dalvik/system/InMemoryDexClassLoader"),
                jni_sig!("(Ljava/nio/ByteBuffer;Ljava/lang/ClassLoader;)V"),
                &[JValue::Object(&buffer), JValue::Object(&parent)],
            )?;
            let name = env.new_string(CLASS)?;
            let class = env
                .call_method(&loader, jni_str!("loadClass"), jni_sig!("(Ljava/lang/String;)Ljava/lang/Class;"), &[JValue::Object(&name)])?
                .l()?;
            let class = env.cast_local::<JClass>(class)?;
            Ok(Self { class: env.new_global_ref(class)? })
        })
    }

    /// Requests the system keyboard.
    pub fn show(&self, activity: *mut c_void) -> Result<(), String> {
        self.call_activity("show", activity)
    }

    /// Hides the system keyboard.
    pub fn hide(&self, activity: *mut c_void) -> Result<(), String> {
        self.call_activity("hide", activity)
    }

    fn call_activity(&self, method: &str, activity: *mut c_void) -> Result<(), String> {
        with_env(|env| {
            let activity = unsafe { JObject::from_raw(env, activity as jobject) };
            let method = jni::strings::JNIString::new(method);
            env.call_static_method(&self.class, &method, jni_sig!("(Landroid/app/Activity;)V"), &[JValue::Object(&activity)])?;
            Ok(())
        })
    }

    /// Returns and clears what has been typed.
    pub fn poll(&self) -> Result<String, String> {
        with_env(|env| {
            let text = env.call_static_method(&self.class, jni_str!("poll"), jni_sig!("()Ljava/lang/String;"), &[])?.l()?;
            let text = env.cast_local::<JString>(text)?;
            text.try_to_string(env)
        })
    }
}
