//! Minimal safe wrappers around native GIO settings and D-Bus APIs.
use anyhow::{Result, bail, ensure};
use std::ffi::{CStr, CString, c_char, c_int, c_void};
type Pointer = *mut c_void;

#[link(name = "gio-2.0")]
unsafe extern "C" {
    fn g_file_new_for_path(path: *const c_char) -> Pointer;
    fn g_file_get_uri(file: Pointer) -> *mut c_char;
    fn g_app_info_launch_default_for_uri(
        uri: *const c_char,
        context: Pointer,
        error: *mut Pointer,
    ) -> c_int;
    fn g_settings_schema_source_get_default() -> Pointer;
    fn g_settings_schema_source_lookup(
        source: Pointer,
        schema: *const c_char,
        recursive: c_int,
    ) -> Pointer;
    fn g_settings_schema_has_key(schema: Pointer, key: *const c_char) -> c_int;
    fn g_settings_schema_unref(schema: Pointer);
    fn g_settings_new_full(schema: Pointer, backend: Pointer, path: *const c_char) -> Pointer;
    fn g_settings_is_writable(settings: Pointer, key: *const c_char) -> c_int;
    fn g_settings_get_value(settings: Pointer, key: *const c_char) -> Pointer;
    fn g_settings_set_value(settings: Pointer, key: *const c_char, value: Pointer) -> c_int;
    fn g_settings_sync();
    fn g_bus_get_sync(kind: c_int, cancellable: Pointer, error: *mut Pointer) -> Pointer;
    fn g_dbus_connection_call_sync(
        connection: Pointer,
        destination: *const c_char,
        path: *const c_char,
        interface: *const c_char,
        method: *const c_char,
        parameters: Pointer,
        reply_type: Pointer,
        flags: c_int,
        timeout_ms: c_int,
        cancellable: Pointer,
        error: *mut Pointer,
    ) -> Pointer;
    fn g_dbus_connection_emit_signal(
        connection: Pointer,
        destination: *const c_char,
        path: *const c_char,
        interface: *const c_char,
        signal: *const c_char,
        parameters: Pointer,
        error: *mut Pointer,
    ) -> c_int;
    fn g_dbus_connection_flush_sync(
        connection: Pointer,
        cancellable: Pointer,
        error: *mut Pointer,
    ) -> c_int;
}
#[link(name = "glib-2.0")]
unsafe extern "C" {
    fn g_variant_print(value: Pointer, annotate: c_int) -> *mut c_char;
    fn g_variant_get_type(value: Pointer) -> Pointer;
    fn g_variant_get_type_string(value: Pointer) -> *const c_char;
    fn g_variant_get_child_value(value: Pointer, index: usize) -> Pointer;
    fn g_variant_get_variant(value: Pointer) -> Pointer;
    fn g_variant_get_boolean(value: Pointer) -> c_int;
    fn g_variant_parse(
        kind: Pointer,
        text: *const c_char,
        limit: *const c_char,
        end: *mut *const c_char,
        error: *mut Pointer,
    ) -> Pointer;
    fn g_variant_ref_sink(value: Pointer) -> Pointer;
    fn g_variant_unref(value: Pointer);
    fn g_free(pointer: Pointer);
}
#[link(name = "gobject-2.0")]
unsafe extern "C" {
    fn g_object_unref(pointer: Pointer);
}

struct Object(Pointer);
impl Drop for Object {
    fn drop(&mut self) {
        unsafe {
            g_object_unref(self.0);
        }
    }
}
struct Variant(Pointer);
impl Drop for Variant {
    fn drop(&mut self) {
        unsafe {
            g_variant_unref(self.0);
        }
    }
}

pub(super) fn ensure_tray_available() -> Result<()> {
    unsafe {
        let bus = g_bus_get_sync(2, std::ptr::null_mut(), std::ptr::null_mut());
        ensure!(!bus.is_null(), "无法连接桌面 D-Bus 会话");
        let bus = Object(bus);
        let parameters = g_variant_parse(
            std::ptr::null_mut(),
            c"('org.kde.StatusNotifierWatcher', 'IsStatusNotifierHostRegistered')".as_ptr(),
            std::ptr::null(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        );
        ensure!(!parameters.is_null(), "无法构建托盘查询");
        let parameters = Variant(g_variant_ref_sink(parameters));
        // The tray library assumes a host exists. Verify the actual host so
        // closing the window cannot hide the app in a nonexistent WSLg tray.
        let reply = g_dbus_connection_call_sync(
            bus.0,
            c"org.kde.StatusNotifierWatcher".as_ptr(),
            c"/StatusNotifierWatcher".as_ptr(),
            c"org.freedesktop.DBus.Properties".as_ptr(),
            c"Get".as_ptr(),
            parameters.0,
            std::ptr::null_mut(),
            1, // G_DBUS_CALL_FLAGS_NO_AUTO_START
            1000,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        );
        ensure!(!reply.is_null(), "当前桌面没有可用托盘宿主");
        let reply = Variant(reply);
        ensure!(
            CStr::from_ptr(g_variant_get_type_string(reply.0)).to_bytes() == b"(v)",
            "托盘宿主返回无效状态"
        );
        let wrapped = Variant(g_variant_get_child_value(reply.0, 0));
        let registered = Variant(g_variant_get_variant(wrapped.0));
        ensure!(
            CStr::from_ptr(g_variant_get_type_string(registered.0)).to_bytes() == b"b"
                && g_variant_get_boolean(registered.0) != 0,
            "当前桌面没有可用托盘宿主"
        );
    }
    Ok(())
}

pub(super) fn setting(args: &[&str]) -> Result<String> {
    ensure!((3..=4).contains(&args.len()), "无效 GSettings 操作");
    let schema_name = CString::new(args[1])?;
    let key = CString::new(args[2])?;
    unsafe {
        let source = g_settings_schema_source_get_default();
        ensure!(!source.is_null(), "找不到 GNOME GSettings schemas");
        let schema = g_settings_schema_source_lookup(source, schema_name.as_ptr(), 1);
        ensure!(
            !schema.is_null(),
            "当前桌面缺少代理设置 schema：{}",
            args[1]
        );
        if g_settings_schema_has_key(schema, key.as_ptr()) == 0 {
            g_settings_schema_unref(schema);
            bail!("GNOME 代理设置键不存在");
        }
        let settings = g_settings_new_full(schema, std::ptr::null_mut(), std::ptr::null());
        g_settings_schema_unref(schema);
        ensure!(!settings.is_null(), "无法打开 GNOME 代理设置");
        let settings = Object(settings);
        match args[0] {
            "writable" => Ok((g_settings_is_writable(settings.0, key.as_ptr()) != 0).to_string()),
            "get" | "set" => {
                let old = g_settings_get_value(settings.0, key.as_ptr());
                ensure!(!old.is_null(), "无法读取 GNOME 代理设置");
                let old = Variant(old);
                if args[0] == "get" {
                    let printed = g_variant_print(old.0, 0);
                    ensure!(!printed.is_null(), "无法格式化 GNOME 代理设置");
                    let result = CStr::from_ptr(printed).to_string_lossy().into_owned();
                    g_free(printed.cast());
                    Ok(result)
                } else {
                    ensure!(args.len() == 4, "缺少 GNOME 代理设置值");
                    let text = CString::new(args[3])?;
                    let value = g_variant_parse(
                        g_variant_get_type(old.0),
                        text.as_ptr(),
                        std::ptr::null(),
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                    );
                    ensure!(!value.is_null(), "GNOME 代理设置值格式无效");
                    let value = Variant(g_variant_ref_sink(value));
                    ensure!(
                        g_settings_set_value(settings.0, key.as_ptr(), value.0) != 0,
                        "GNOME 代理设置不可写"
                    );
                    Ok(String::new())
                }
            }
            _ => bail!("无效 GSettings 操作"),
        }
    }
}

pub(super) fn sync() {
    unsafe {
        g_settings_sync();
    }
}

pub(super) fn open_uri(uri: &str) -> Result<()> {
    let uri = CString::new(uri)?;
    unsafe {
        ensure!(
            g_app_info_launch_default_for_uri(
                uri.as_ptr(),
                std::ptr::null_mut(),
                std::ptr::null_mut()
            ) != 0,
            "无法打开默认应用"
        );
    }
    Ok(())
}

pub(super) fn open_directory(path: &std::path::Path) -> Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let absolute = std::path::absolute(path)?;
    let path = CString::new(absolute.as_os_str().as_bytes())?;
    unsafe {
        let file = g_file_new_for_path(path.as_ptr());
        ensure!(!file.is_null(), "无法打开目录");
        let file = Object(file);
        let uri = g_file_get_uri(file.0);
        ensure!(!uri.is_null(), "无法生成目录地址");
        let text = CStr::from_ptr(uri).to_string_lossy().into_owned();
        g_free(uri.cast());
        open_uri(&text)
    }
}

pub(super) fn notify_kde() -> Result<()> {
    unsafe {
        let bus = g_bus_get_sync(2, std::ptr::null_mut(), std::ptr::null_mut());
        ensure!(!bus.is_null(), "无法连接 KDE 桌面 D-Bus 会话");
        let bus = Object(bus);
        let parameters = g_variant_parse(
            std::ptr::null_mut(),
            c"('',)".as_ptr(),
            std::ptr::null(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        );
        ensure!(!parameters.is_null(), "无法构建 KDE 配置刷新消息");
        let parameters = Variant(g_variant_ref_sink(parameters));
        ensure!(
            g_dbus_connection_emit_signal(
                bus.0,
                std::ptr::null(),
                c"/KIO/Scheduler".as_ptr(),
                c"org.kde.KIO.Scheduler".as_ptr(),
                c"reparseSlaveConfiguration".as_ptr(),
                parameters.0,
                std::ptr::null_mut()
            ) != 0,
            "无法通知 KDE 代理配置更新"
        );
        ensure!(
            g_dbus_connection_flush_sync(bus.0, std::ptr::null_mut(), std::ptr::null_mut()) != 0,
            "KDE 代理配置通知失败"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "Run with GSETTINGS_BACKEND=memory to isolate desktop settings"]
    fn native_gsettings_round_trip_on_isolated_memory_backend() {
        assert_eq!(std::env::var("GSETTINGS_BACKEND").unwrap(), "memory");
        let schema = "org.gnome.system.proxy.http";
        assert_eq!(setting(&["writable", schema, "port"]).unwrap(), "true");
        setting(&["set", schema, "port", "7897"]).unwrap();
        setting(&["set", schema, "host", "'127.0.0.1'"]).unwrap();
        sync();
        assert_eq!(setting(&["get", schema, "port"]).unwrap(), "7897");
        assert_eq!(setting(&["get", schema, "host"]).unwrap(), "'127.0.0.1'");
        assert!(setting(&["get", "missing.proxy.schema", "port"]).is_err());
    }
    #[test]
    #[ignore = "Run under dbus-run-session to isolate the desktop D-Bus bus"]
    fn native_kde_signal_on_isolated_bus() {
        notify_kde().unwrap();
    }

    #[test]
    #[ignore = "Run under dbus-run-session to isolate the desktop D-Bus bus"]
    fn no_tray_host_on_isolated_bus() {
        assert!(ensure_tray_available().is_err());
    }
}
