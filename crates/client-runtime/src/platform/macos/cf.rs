//! Small owned CoreFoundation bridge. Values cross the boundary as canonical JSON.
#![warn(clippy::undocumented_unsafe_blocks)]

use anyhow::{Context, Result, bail, ensure};
use serde_json::{Map, Value};
use std::{ffi::c_void, ptr};

pub type Ref = *const c_void;
#[link(name = "CoreFoundation", kind = "framework")]
// SAFETY: These CoreFoundation declarations use pointer-width CFIndex/CFTypeID,
// u32 encodings, u8 Boolean values and opaque object pointers on supported macOS.
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
        // SAFETY: Callers adopt nonnull Create/Copy outputs with one owned
        // reference; borrowed Get results are never wrapped in this guard.
        unsafe { CFRelease(self.0) };
    }
}
pub fn string(value: &str) -> Result<Owned> {
    let length = isize::try_from(value.len()).context("系统字符串过大")?;
    // SAFETY: Rust supplies valid UTF-8 bytes for the checked length. The API
    // copies them synchronously and returns an owned reference or null.
    Owned::new(unsafe { CFStringCreateWithBytes(ptr::null(), value.as_ptr(), length, UTF8, 0) })
}
pub fn text(value: Ref) -> Result<String> {
    ensure!(
        // SAFETY: Callers borrow a live CF object. Reject null before asking its
        // type; a successful check permits the following CFString-only calls.
        !value.is_null() && unsafe { CFGetTypeID(value) == CFStringGetTypeID() },
        "系统字符串类型无效"
    );
    // SAFETY: The checked string remains live; its length is a UTF-16 unit count.
    let length = unsafe { CFStringGetLength(value) };
    // SAFETY: length came from the live CFString and this query only sizes a copy.
    let size = unsafe { CFStringGetMaximumSizeForEncoding(length, UTF8) }
        .checked_add(1)
        .context("系统字符串过大")?;
    ensure!((1..=1024 * 1024).contains(&size), "系统字符串过大");
    let mut bytes = vec![0; size as usize];
    ensure!(
        // SAFETY: The checked capacity includes a terminator, bytes is writable
        // for that full size, and the borrowed string outlives this copy.
        unsafe { CFStringGetCString(value, bytes.as_mut_ptr(), size, UTF8) } != 0,
        "系统字符串转换失败"
    );
    bytes.truncate(
        bytes
            .iter()
            .position(|b| *b == 0)
            .context("系统字符串缺少结束标记")?,
    );
    let text = String::from_utf8(bytes)?;
    ensure!(
        text.encode_utf16().count() == usize::try_from(length).context("系统字符串长度无效")?,
        "系统字符串转换长度不一致，已停止修改"
    );
    Ok(text)
}
pub fn dictionary_keys(value: Ref) -> Result<Vec<String>> {
    ensure!(
        // SAFETY: Callers retain the CF owner; null is rejected before querying.
        !value.is_null() && unsafe { CFGetTypeID(value) == CFDictionaryGetTypeID() },
        "系统字典类型无效"
    );
    // SAFETY: The checked dictionary is live and is not mutated during the copy.
    let count = unsafe { CFDictionaryGetCount(value) };
    ensure!((0..=4096).contains(&count), "系统字典过大");
    let mut keys = vec![ptr::null(); count as usize];
    // SAFETY: keys has exactly count writable slots. A null values buffer asks
    // for keys only; borrowed keys are copied while the parent owner is alive.
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
    // SAFETY: Callers retain the containing CF owner until recursive conversion
    // finishes; every recursive entry rejects null before inspecting its type.
    let kind = unsafe { CFGetTypeID(value) };
    // SAFETY: Every operation below is guarded by its exact CF type ID. Number
    // kinds 13 (double) and 4 (signed 64-bit) match the typed output buffers.
    // Array indices stay inside the checked count; dictionary buffers have count
    // writable slots. Callers do not mutate the parent during conversion; Get
    // results are copied into JSON, never adopted as independently owned references.
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
    let length = isize::try_from(bytes.len()).context("系统属性数据过大")?;
    // SAFETY: CFDataCreate copies this checked byte range synchronously and
    // returns an owned reference; bytes stays live until the call returns.
    let data = Owned::new(unsafe { CFDataCreate(ptr::null(), bytes.as_ptr(), length) })?;
    let mut error = ptr::null();
    // SAFETY: data stays owned throughout parsing, error is writable, and null
    // format requests no format output. The returned property list is immutable.
    let value = unsafe {
        CFPropertyListCreateWithData(ptr::null(), data.0, 0, ptr::null_mut(), &mut error)
    };
    if !error.is_null() {
        // The API transfers its error reference separately from the result.
        drop(Owned(error));
    }
    Owned::new(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn conversion_rejects_null_wrong_types_and_embedded_string_terminators() {
        assert!(text(ptr::null()).is_err());
        assert!(dictionary_keys(ptr::null()).is_err());
        assert!(super::json(ptr::null()).is_err());
        let number = plist(&json!(42)).unwrap();
        assert!(text(number.0).is_err());
        assert!(dictionary_keys(number.0).is_err());
        let string = string("代理").unwrap();
        assert!(dictionary_keys(string.0).is_err());
        let embedded = super::string("before\0after").unwrap();
        assert!(text(embedded.0).is_err());
        assert!(super::json(embedded.0).is_err());
    }

    #[test]
    fn copied_strings_and_nested_values_outlive_their_cf_owner() {
        let expected = "代理 café 🦀 <PAC> & \"例外\" '列表'";
        let copied = {
            let owner = string(expected).unwrap();
            text(owner.0).unwrap()
        };
        assert_eq!(copied, expected);
        let expected = json!({
            "代理": {
                "enabled": true,
                "port": 7890,
                "ratio": 1.25,
                "empty": [],
                "exceptions": ["*.local", "café.example", "<>&\"'"]
            },
            "empty": {}
        });
        let (mut keys, copied) = {
            let owner = plist(&expected).unwrap();
            (
                dictionary_keys(owner.0).unwrap(),
                super::json(owner.0).unwrap(),
            )
        };
        keys.sort();
        assert_eq!(keys, ["empty", "代理"]);
        assert_eq!(copied, expected);
    }
}
