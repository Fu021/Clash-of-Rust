//! Small owned CoreFoundation bridge. Values cross the boundary as canonical JSON.
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Map, Value};
use std::{ffi::c_void, ptr};

pub type Ref = *const c_void;
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(value: Ref);
    fn CFGetTypeID(value: Ref) -> usize;
    fn CFStringGetTypeID() -> usize;
    fn CFNumberGetTypeID() -> usize;
    fn CFBooleanGetTypeID() -> usize;
    fn CFArrayGetTypeID() -> usize;
    fn CFDictionaryGetTypeID() -> usize;
    fn CFStringCreateWithBytes(
        a: Ref,
        bytes: *const u8,
        len: isize,
        encoding: u32,
        external: u8,
    ) -> Ref;
    fn CFStringGetLength(value: Ref) -> isize;
    fn CFStringGetMaximumSizeForEncoding(len: isize, encoding: u32) -> isize;
    fn CFStringGetCString(value: Ref, buffer: *mut u8, size: isize, encoding: u32) -> u8;
    fn CFNumberGetValue(value: Ref, kind: isize, result: *mut c_void) -> u8;
    fn CFNumberIsFloatType(value: Ref) -> u8;
    fn CFBooleanGetValue(value: Ref) -> u8;
    fn CFArrayGetCount(value: Ref) -> isize;
    fn CFArrayGetValueAtIndex(value: Ref, index: isize) -> Ref;
    fn CFDictionaryGetCount(value: Ref) -> isize;
    fn CFDictionaryGetKeysAndValues(value: Ref, keys: *mut Ref, values: *mut Ref);
    fn CFDataCreate(a: Ref, bytes: *const u8, length: isize) -> Ref;
    fn CFPropertyListCreateWithData(
        a: Ref,
        data: Ref,
        options: usize,
        format: *mut isize,
        error: *mut Ref,
    ) -> Ref;
}
const UTF8: u32 = 0x08000100;
pub struct Owned(pub Ref);
impl Owned {
    pub fn new(value: Ref) -> Result<Self> {
        ensure!(!value.is_null(), "macOS 系统接口返回空对象");
        Ok(Self(value))
    }
}
impl Drop for Owned {
    fn drop(&mut self) {
        unsafe { CFRelease(self.0) };
    }
}
pub fn string(value: &str) -> Result<Owned> {
    Owned::new(unsafe {
        CFStringCreateWithBytes(ptr::null(), value.as_ptr(), value.len() as isize, UTF8, 0)
    })
}
pub fn text(value: Ref) -> Result<String> {
    ensure!(
        unsafe { CFGetTypeID(value) == CFStringGetTypeID() },
        "系统字符串类型无效"
    );
    let size = unsafe { CFStringGetMaximumSizeForEncoding(CFStringGetLength(value), UTF8) } + 1;
    ensure!((1..=1024 * 1024).contains(&size), "系统字符串过大");
    let mut bytes = vec![0; size as usize];
    ensure!(
        unsafe { CFStringGetCString(value, bytes.as_mut_ptr(), size, UTF8) } != 0,
        "系统字符串转换失败"
    );
    bytes.truncate(
        bytes
            .iter()
            .position(|b| *b == 0)
            .context("系统字符串缺少结束标记")?,
    );
    Ok(String::from_utf8(bytes)?)
}
pub fn dictionary_keys(value: Ref) -> Result<Vec<String>> {
    ensure!(
        !value.is_null() && unsafe { CFGetTypeID(value) == CFDictionaryGetTypeID() },
        "系统字典类型无效"
    );
    let count = unsafe { CFDictionaryGetCount(value) };
    ensure!((0..=4096).contains(&count), "系统字典过大");
    let mut keys = vec![ptr::null(); count as usize];
    unsafe {
        CFDictionaryGetKeysAndValues(value, keys.as_mut_ptr(), ptr::null_mut());
    }
    keys.into_iter().map(text).collect()
}
pub fn json(value: Ref) -> Result<Value> {
    to_json(value, 0)
}
fn to_json(value: Ref, depth: usize) -> Result<Value> {
    ensure!(!value.is_null() && depth < 32, "系统属性结构无效");
    let kind = unsafe { CFGetTypeID(value) };
    unsafe {
        if kind == CFStringGetTypeID() {
            return Ok(Value::String(text(value)?));
        }
        if kind == CFBooleanGetTypeID() {
            return Ok(Value::Bool(CFBooleanGetValue(value) != 0));
        }
        if kind == CFNumberGetTypeID() {
            if CFNumberIsFloatType(value) != 0 {
                let mut number = 0_f64;
                ensure!(
                    CFNumberGetValue(value, 13, (&mut number as *mut f64).cast()) != 0,
                    "系统数值转换失败"
                );
                return serde_json::Number::from_f64(number)
                    .map(Value::Number)
                    .context("系统数值无效");
            }
            let mut number = 0_i64;
            ensure!(
                CFNumberGetValue(value, 4, (&mut number as *mut i64).cast()) != 0,
                "系统整数转换失败"
            );
            return Ok(number.into());
        }
        if kind == CFArrayGetTypeID() {
            let count = CFArrayGetCount(value);
            ensure!((0..=4096).contains(&count), "系统数组过大");
            return (0..count)
                .map(|i| to_json(CFArrayGetValueAtIndex(value, i), depth + 1))
                .collect::<Result<Vec<_>>>()
                .map(Value::Array);
        }
        if kind == CFDictionaryGetTypeID() {
            let count = CFDictionaryGetCount(value);
            ensure!((0..=4096).contains(&count), "系统字典过大");
            let mut keys = vec![ptr::null(); count as usize];
            let mut values = keys.clone();
            CFDictionaryGetKeysAndValues(value, keys.as_mut_ptr(), values.as_mut_ptr());
            let mut result = Map::new();
            for (key, value) in keys.into_iter().zip(values) {
                result.insert(text(key)?, to_json(value, depth + 1)?);
            }
            return Ok(Value::Object(result));
        }
    }
    bail!("系统代理中存在不支持的属性类型，已停止修改")
}
pub fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
pub fn plist(value: &Value) -> Result<Owned> {
    fn encode(value: &Value) -> Result<String> {
        Ok(match value {
            Value::Object(map) => format!(
                "<dict>{}</dict>",
                map.iter()
                    .map(|(key, value)| Ok(format!("<key>{}</key>{}", escape(key), encode(value)?)))
                    .collect::<Result<Vec<_>>>()?
                    .join("")
            ),
            Value::Array(values) => format!(
                "<array>{}</array>",
                values
                    .iter()
                    .map(encode)
                    .collect::<Result<Vec<_>>>()?
                    .join("")
            ),
            Value::String(value) => format!("<string>{}</string>", escape(value)),
            Value::Bool(true) => "<true/>".into(),
            Value::Bool(false) => "<false/>".into(),
            Value::Number(value) => format!(
                "<{kind}>{value}</{kind}>",
                kind = if value.is_f64() { "real" } else { "integer" }
            ),
            Value::Null => bail!("plist 不能包含 null"),
        })
    }
    let bytes = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><plist version=\"1.0\">{}</plist>",
        encode(value)?
    );
    let data =
        Owned::new(unsafe { CFDataCreate(ptr::null(), bytes.as_ptr(), bytes.len() as isize) })?;
    let mut error = ptr::null();
    let value = unsafe {
        CFPropertyListCreateWithData(ptr::null(), data.0, 0, ptr::null_mut(), &mut error)
    };
    if !error.is_null() {
        drop(Owned(error));
    }
    Owned::new(value)
}
