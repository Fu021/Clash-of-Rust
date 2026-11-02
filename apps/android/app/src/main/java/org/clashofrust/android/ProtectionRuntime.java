package org.clashofrust.android;

import android.content.Context;
import java.io.File;
import java.io.IOException;
import java.nio.file.Files;
import java.nio.charset.StandardCharsets;
import org.json.JSONObject;

/** Foreground VPN service owns enforcement; no Activity reference is retained. */
final class ProtectionRuntime {
    static { System.loadLibrary("clash_mobile"); }
    private static volatile Context context;
    private static volatile boolean configured;
    private static native String nativeConfigure(String json);
    private static native String nativeState();
    private static native String nativeRecover(String root);
    static native int notificationHealth();
    private ProtectionRuntime() {}
    static synchronized void recover(Context owner) throws Exception {
        JSONObject response = new JSONObject(nativeRecover(new File(owner.getFilesDir(),"client").getAbsolutePath()));
        if (response.has("error")) throw new IOException(response.getString("error"));
    }
    static synchronized void pause(Context owner) throws Exception {
        context = owner.getApplicationContext();
        JSONObject response = new JSONObject(nativeConfigure("{\"enabled\":false}"));
        if (response.has("error")) throw new IOException(response.getString("error"));
    }
    static synchronized void sync(Context owner) throws Exception {
        context = owner.getApplicationContext();
        if (!TunnelService.connected()) { pause(owner); return; }
        File path = new File(context.getFilesDir(), "client/settings.json");
        if (path.length() > 10 * 1024 * 1024) throw new IOException("保护配置超过大小限制");
        JSONObject settings = path.isFile() ? new JSONObject(new String(Files.readAllBytes(path.toPath()), StandardCharsets.UTF_8)) : new JSONObject();
        JSONObject protection = settings.optJSONObject("claude_protection");
        boolean enabled = protection != null && TunnelService.connected();
        configured = protection != null;
        JSONObject config = new JSONObject().put("enabled", enabled);
        if (enabled) config.put("node", protection.getString("node"))
            .put("controller_port", settings.getInt("controller_port"))
            .put("mixed_port", settings.getInt("mixed_port")).put("secret", settings.getString("secret"));
        JSONObject response = new JSONObject(nativeConfigure(config.toString()));
        if (response.has("error")) throw new IOException(response.getString("error"));
    }
    static JSONObject state() throws Exception { return new JSONObject(nativeState()); }
    static String notificationLabel(boolean configured, int health) {
        if (!configured) return "";
        switch (health) {
            case 1: return "Claude 保护已启用";
            case 3: return "Claude 保护异常";
            default: return "Claude 保护待确认";
        }
    }
    static String notificationStatus() {
        return configured ? notificationLabel(true, notificationHealth()) : "";
    }
    // JNI calls this off the UI thread, including when the Activity is closed.
    static String notify(String title, String message) {
        try {
            if (context == null) throw new IllegalStateException("通知宿主尚未就绪");
            SystemNotifications.show(context, title, message);
            return "{\"ok\":true}";
        } catch (Exception error) {
            try { return new JSONObject().put("error", error.toString()).toString(); }
            catch (Exception ignored) { return "{\"error\":\"系统通知失败\"}"; }
        }
    }
}
