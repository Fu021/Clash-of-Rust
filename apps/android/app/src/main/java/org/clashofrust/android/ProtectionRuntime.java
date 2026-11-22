package org.clashofrust.android;

import android.content.Context;
import java.io.File;
import java.io.IOException;
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
        sync(owner, TunnelService.connected());
    }
    static synchronized void sync(Context owner, boolean vpnPrepared) throws Exception {
        context = owner.getApplicationContext();
        if (!vpnPrepared) { pause(owner); return; }
        JSONObject settings = CoreBridge.savedSettings(context);
        JSONObject protection = CoreBridge.savedProtection(settings);
        boolean enabled = protection != null;
        configured = protection != null;
        JSONObject config = new JSONObject().put("enabled", enabled);
        if (enabled) config.put("node", protection.getString("node"))
            .put("controller_port", settings.getInt("controller_port"))
            .put("mixed_port", settings.getInt("mixed_port")).put("secret", settings.getString("secret"));
        JSONObject response = new JSONObject(nativeConfigure(config.toString()));
        if (response.has("error")) throw new IOException(response.getString("error"));
    }
    static JSONObject state() throws Exception { return new JSONObject(nativeState()); }
    static boolean guardMatches(boolean enabled) throws Exception {
        JSONObject guard = state();
        JSONObject snapshot = guard.optJSONObject("snapshot");
        return Boolean.valueOf(enabled).equals(guard.opt("enabled"))
            && snapshot != null && "".equals(snapshot.opt("error"))
            && (!enabled || Boolean.TRUE.equals(snapshot.opt("connected")));
    }
    static String notificationLabel(Context context, boolean configured, int health) {
        if (!configured) return "";
        switch (health) {
            case 1: return context.getString(R.string.protection_active);
            case 3: return context.getString(R.string.protection_error);
            default: return context.getString(R.string.protection_unconfirmed);
        }
    }
    static String notificationStatus(Context context) {
        if (TunnelService.recoveryRequired(context) || TunnelService.forwardingHeld()
            || (configured && !TunnelService.error().isEmpty()))
            return context.getString(R.string.protection_unconfirmed);
        return configured ? notificationLabel(context, true, notificationHealth()) : "";
    }
    static String language() {
        return language(context);
    }
    static String language(Context owner) {
        return owner == null ? "en" : AppLanguage.current(owner).getResources()
            .getConfiguration().getLocales().get(0).toLanguageTag();
    }
    // JNI calls this off the UI thread, including when the Activity is closed.
    static String notify(String title, String message) {
        try {
            Context owner = context;
            if (owner == null) throw new IllegalStateException(AppLanguage.current(ClientApplication.instance())
                .getString(R.string.notification_host_unavailable));
            SystemNotifications.show(owner, title, message);
            return "{\"ok\":true}";
        } catch (Exception error) {
            return "{\"error\":" + JSONObject.quote(error.toString()) + "}";
        }
    }
}
