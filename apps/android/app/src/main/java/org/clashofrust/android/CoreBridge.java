package org.clashofrust.android;

import android.content.Context;
import java.io.*;
import java.nio.file.Files;
import java.nio.file.StandardCopyOption;
import java.nio.charset.StandardCharsets;
import org.json.JSONObject;
import org.clashofrust.nativecore.mobile.Mobile;

/** Process-scoped core. Activity recreation must not restart the VPN or core. */
public final class CoreBridge {
    private static boolean started;
    private static String lastConfig = "";
    private static boolean ipv6Enabled = true;
    private CoreBridge() {}

    public static synchronized String start(Context context, String raw, boolean ipv6) throws Exception {
        File home = new File(context.getFilesDir(), "client/runtime");
        if (!home.isDirectory() && !home.mkdirs()) throw new IOException("无法创建内核目录");
        // Seed only absent files; keep complete user updates across app upgrades.
        for (String name : new String[]{"GeoIP.dat", "GeoSite.dat", "Country.mmdb", "ASN.mmdb", "geodata.json"}) {
            File target = new File(home, name);
            if (!target.isFile()) {
                File partial = new File(home, name + ".partial");
                try (InputStream in = context.getAssets().open(name);
                     FileOutputStream out = new FileOutputStream(partial)) {
                    byte[] buffer = new byte[8192];
                    int count;
                    while ((count = in.read(buffer)) != -1) out.write(buffer, 0, count);
                    out.getFD().sync();
                }
                Files.move(partial.toPath(), target.toPath(), StandardCopyOption.REPLACE_EXISTING);
            }
        }
        if (started) Mobile.reload(raw); else { Mobile.start(home.toString(), raw); started = true; }
        lastConfig = raw;
        ipv6Enabled = ipv6;
        return home.toString();
    }

    public static synchronized void reload(String raw) throws Exception {
        if (!started) throw new IOException("内核尚未启动");
        Mobile.reload(raw); lastConfig = raw;
    }
    public static synchronized boolean ready() { return started; }
    static boolean restorable(Context context) {
        File runtime = new File(context.getFilesDir(), "client/runtime/config.yaml");
        File settings = new File(context.getFilesDir(), "client/settings.json");
        return runtime.isFile() && settings.isFile() && runtime.length() <= 20 * 1024 * 1024
            && settings.length() <= 10 * 1024 * 1024;
    }
    public static synchronized void restore(Context context) throws Exception {
        if (started) return;
        ProtectionRuntime.recover(context);
        File runtime = new File(context.getFilesDir(), "client/runtime/config.yaml");
        File settings = new File(context.getFilesDir(), "client/settings.json");
        if (!runtime.isFile() || !settings.isFile() || runtime.length() > 20 * 1024 * 1024 || settings.length() > 10 * 1024 * 1024)
            throw new IOException("没有可恢复的内核配置，请打开应用连接");
        JSONObject saved = new JSONObject(new String(Files.readAllBytes(settings.toPath()), StandardCharsets.UTF_8));
        boolean ipv6 = saved.optBoolean("ipv6", true) || saved.optJSONObject("claude_protection") != null;
        start(context, new String(Files.readAllBytes(runtime.toPath()), StandardCharsets.UTF_8), ipv6);
    }
    static boolean protectedMode(Context context) throws Exception {
        File settings = new File(context.getFilesDir(), "client/settings.json");
        if (!settings.isFile()) return false;
        if (settings.length() > 10 * 1024 * 1024) throw new IOException("保护配置超过大小限制");
        return new JSONObject(new String(Files.readAllBytes(settings.toPath()), StandardCharsets.UTF_8))
            .optJSONObject("claude_protection") != null;
    }
    public static synchronized String config() { return lastConfig; }
    public static synchronized boolean ipv6() { return ipv6Enabled; }
    public static synchronized void setIpv6(boolean enabled) { ipv6Enabled = enabled; }
    public static JSONObject state() throws Exception {
        return new JSONObject().put("running", ready()).put("vpn", TunnelService.connected())
            .put("vpn_pending", TunnelService.pending())
            .put("vpn_generation", TunnelService.generation())
            .put("protection", ProtectionRuntime.state())
            .put("vpn_error", TunnelService.error());
    }
}
