//! JNI entry points for `com.desklink.android.NativeBridge`.
//!
//! Every function catches panics and converts errors into `IllegalStateException`, so a
//! failure never unwinds into the Android runtime.
use crate::{
    agent::{Agent, Receiver},
    bridge::{self, Bridge, Taken},
    logcat, profile,
};
use anyhow::{Context, Result};
use desklink_protocol::ClipboardContent;
use jni::{
    JNIEnv,
    objects::{JByteArray, JClass, JString},
    sys::{JNI_FALSE, JNI_TRUE, jboolean, jbyteArray, jint, jstring},
};
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    path::PathBuf,
    ptr,
    sync::{Arc, OnceLock},
    time::Duration,
};

static AGENT: OnceLock<Agent> = OnceLock::new();
/// Drives the computer the phone presents itself to as a Bluetooth keyboard and mouse. Android
/// connects a HID device to one computer at a time, so there is at most one.
static KEYBOARD: OnceLock<Agent> = OnceLock::new();

fn agent() -> &'static Agent {
    AGENT.get_or_init(|| {
        logcat::init();
        Agent::new(Arc::new(Bridge::new()))
    })
}

fn keyboard() -> &'static Agent {
    KEYBOARD.get_or_init(|| {
        logcat::init();
        Agent::new(Arc::new(Bridge::new()))
    })
}

fn take_actions(env: &mut JNIEnv, agent: &Agent, timeout_ms: jint) -> Result<jbyteArray> {
    let timeout = Duration::from_millis(timeout_ms.clamp(0, 10_000) as u64);
    match agent.bridge().take(timeout) {
        Taken::Actions(actions) => Ok(env
            .byte_array_from_slice(&bridge::encode(&actions))?
            .into_raw()),
        Taken::Timeout => Ok(env.byte_array_from_slice(&[])?.into_raw()),
        Taken::Closed => Ok(ptr::null_mut()),
    }
}

fn guard<T>(env: &mut JNIEnv, default: T, f: impl FnOnce(&mut JNIEnv) -> Result<T>) -> T {
    let outcome = catch_unwind(AssertUnwindSafe(|| f(&mut *env)));
    let message = match outcome {
        Ok(Ok(value)) => return value,
        Ok(Err(error)) => format!("{error:#}"),
        Err(_) => "Desklink native code failed unexpectedly.".to_string(),
    };
    if !env.exception_check().unwrap_or(false) {
        let _ = env.throw_new("java/lang/IllegalStateException", message);
    }
    default
}

fn text(env: &mut JNIEnv, value: &JString) -> Result<String> {
    Ok(env.get_string(value).context("invalid string")?.into())
}

fn optional_text(env: &mut JNIEnv, value: &JString) -> Result<Option<String>> {
    if value.as_raw().is_null() {
        Ok(None)
    } else {
        text(env, value).map(Some)
    }
}

fn dir(env: &mut JNIEnv, value: &JString) -> Result<PathBuf> {
    Ok(PathBuf::from(text(env, value)?))
}

fn json(env: &mut JNIEnv, value: &impl serde::Serialize) -> Result<jstring> {
    Ok(env.new_string(serde_json::to_string(value)?)?.into_raw())
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_desklink_android_NativeBridge_init(
    mut env: JNIEnv,
    _: JClass,
    path: JString,
    name: JString,
) {
    guard(&mut env, (), |env| {
        agent();
        let (path, name) = (dir(env, &path)?, text(env, &name)?);
        profile::ensure(&path, &name)
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_desklink_android_NativeBridge_info(
    mut env: JNIEnv,
    _: JClass,
    path: JString,
    address: JString,
) -> jstring {
    guard(&mut env, ptr::null_mut(), |env| {
        let (path, address) = (dir(env, &path)?, optional_text(env, &address)?);
        let info = profile::info(&path, address.as_deref())?;
        json(env, &info)
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_desklink_android_NativeBridge_preview(
    mut env: JNIEnv,
    _: JClass,
    code: JString,
) -> jstring {
    guard(&mut env, ptr::null_mut(), |env| {
        let code = text(env, &code)?;
        let preview = profile::preview(&code)?;
        json(env, &preview)
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_desklink_android_NativeBridge_pair(
    mut env: JNIEnv,
    _: JClass,
    path: JString,
    code: JString,
    controller: jboolean,
) -> jstring {
    guard(&mut env, ptr::null_mut(), |env| {
        let (path, code) = (dir(env, &path)?, text(env, &code)?);
        let name = profile::pair(&path, &code, controller == JNI_TRUE)?;
        Ok(env.new_string(name)?.into_raw())
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_desklink_android_NativeBridge_unpair(
    mut env: JNIEnv,
    _: JClass,
    path: JString,
    key: JString,
) {
    guard(&mut env, (), |env| {
        let (path, key) = (dir(env, &path)?, text(env, &key)?);
        profile::unpair(&path, &key)
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_desklink_android_NativeBridge_setName(
    mut env: JNIEnv,
    _: JClass,
    path: JString,
    name: JString,
) {
    guard(&mut env, (), |env| {
        let (path, name) = (dir(env, &path)?, text(env, &name)?);
        profile::set_name(&path, &name)
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_desklink_android_NativeBridge_setClipboard(
    mut env: JNIEnv,
    _: JClass,
    path: JString,
    enabled: jboolean,
) {
    guard(&mut env, (), |env| {
        let path = dir(env, &path)?;
        profile::set_clipboard(&path, enabled == JNI_TRUE)
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_desklink_android_NativeBridge_setGeometry(
    mut env: JNIEnv,
    _: JClass,
    width: jint,
    height: jint,
) {
    guard(&mut env, (), |_| {
        let width = u32::try_from(width).context("negative width")?;
        let height = u32::try_from(height).context("negative height")?;
        agent().bridge().set_geometry(width, height)
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_desklink_android_NativeBridge_start(
    mut env: JNIEnv,
    _: JClass,
    path: JString,
) {
    guard(&mut env, (), |env| {
        let path = dir(env, &path)?;
        agent().start(&path)
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_desklink_android_NativeBridge_stop(mut env: JNIEnv, _: JClass) {
    guard(&mut env, (), |_| {
        agent().stop();
        Ok(())
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_desklink_android_NativeBridge_isRunning(
    mut env: JNIEnv,
    _: JClass,
) -> jboolean {
    guard(&mut env, JNI_FALSE, |_| {
        Ok(if agent().is_running() {
            JNI_TRUE
        } else {
            JNI_FALSE
        })
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_desklink_android_NativeBridge_status(
    mut env: JNIEnv,
    _: JClass,
    path: JString,
) -> jstring {
    guard(&mut env, ptr::null_mut(), |env| {
        let path = dir(env, &path)?;
        let status = agent().status(&path);
        json(env, &status)
    })
}

/// Blocks up to `timeout_ms`. Returns encoded actions, an empty array on timeout, or `null`
/// once the agent has stopped.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_desklink_android_NativeBridge_nextActions(
    mut env: JNIEnv,
    _: JClass,
    timeout_ms: jint,
) -> jbyteArray {
    guard(&mut env, ptr::null_mut(), |env| {
        take_actions(env, agent(), timeout_ms)
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_desklink_android_NativeBridge_localClipboard(
    mut env: JNIEnv,
    _: JClass,
    kind: jint,
    data: JByteArray,
) {
    guard(&mut env, (), |env| {
        let kind = bridge::kind_from_code(kind)?;
        let bytes = env.convert_byte_array(&data)?;
        agent()
            .bridge()
            .local_clipboard(ClipboardContent { kind, bytes })
    })
}

/// The keyboard-and-mouse report descriptor the app registers with Android's HID device profile.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_desklink_android_NativeBridge_hidDescriptor(
    mut env: JNIEnv,
    _: JClass,
) -> jbyteArray {
    guard(&mut env, ptr::null_mut(), |env| {
        Ok(env
            .byte_array_from_slice(desklink_hid::descriptor::DESCRIPTOR)?
            .into_raw())
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_desklink_android_NativeBridge_keyboardEnsure(
    mut env: JNIEnv,
    _: JClass,
    path: JString,
    name: JString,
) {
    guard(&mut env, (), |env| {
        keyboard();
        let (path, name) = (dir(env, &path)?, text(env, &name)?);
        profile::ensure_keyboard(&path, &name)
    })
}

/// Trusts a computer this phone already trusts as the keyboard's controlling computer.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_desklink_android_NativeBridge_keyboardTrust(
    mut env: JNIEnv,
    _: JClass,
    phone: JString,
    path: JString,
    key: JString,
) -> jstring {
    guard(&mut env, ptr::null_mut(), |env| {
        let (phone, path, key) = (dir(env, &phone)?, dir(env, &path)?, text(env, &key)?);
        let name = profile::trust_keyboard_controller(&phone, &path, &key)?;
        Ok(env.new_string(name)?.into_raw())
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_desklink_android_NativeBridge_keyboardPair(
    mut env: JNIEnv,
    _: JClass,
    path: JString,
    code: JString,
) -> jstring {
    guard(&mut env, ptr::null_mut(), |env| {
        let (path, code) = (dir(env, &path)?, text(env, &code)?);
        let name = profile::pair_keyboard(&path, &code)?;
        Ok(env.new_string(name)?.into_raw())
    })
}

/// `width`x`height` is the controlled screen in the units its pointer uses (macOS points).
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_desklink_android_NativeBridge_keyboardStart(
    mut env: JNIEnv,
    _: JClass,
    path: JString,
    width: jint,
    height: jint,
    relative: jboolean,
    swap_control_and_command: jboolean,
) {
    guard(&mut env, (), |env| {
        let path = dir(env, &path)?;
        keyboard().set_receiver(Receiver::Keyboard(desklink_hid::Options {
            width: u32::try_from(width).context("negative width")?,
            height: u32::try_from(height).context("negative height")?,
            pointer: if relative == JNI_TRUE {
                desklink_hid::PointerMode::Relative
            } else {
                desklink_hid::PointerMode::Absolute
            },
            swap_control_and_command: swap_control_and_command == JNI_TRUE,
        }));
        keyboard().start(&path)
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_desklink_android_NativeBridge_keyboardStop(
    mut env: JNIEnv,
    _: JClass,
) {
    guard(&mut env, (), |_| {
        keyboard().stop();
        Ok(())
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_desklink_android_NativeBridge_keyboardStatus(
    mut env: JNIEnv,
    _: JClass,
    path: JString,
) -> jstring {
    guard(&mut env, ptr::null_mut(), |env| {
        let path = dir(env, &path)?;
        let status = keyboard().status(&path);
        json(env, &status)
    })
}

/// Like `nextActions`, for the keyboard-and-mouse agent: Bluetooth reports to send.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_desklink_android_NativeBridge_keyboardNextActions(
    mut env: JNIEnv,
    _: JClass,
    timeout_ms: jint,
) -> jbyteArray {
    guard(&mut env, ptr::null_mut(), |env| {
        take_actions(env, keyboard(), timeout_ms)
    })
}
