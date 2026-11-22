package org.clashofrust.android;

import android.content.Context;
import java.io.*;
import java.nio.file.Files;
import java.nio.file.LinkOption;
import java.nio.file.StandardCopyOption;
import java.nio.file.attribute.BasicFileAttributes;
import java.nio.charset.StandardCharsets;
import org.json.JSONObject;
import org.clashofrust.nativecore.mobile.Mobile;

/** Process-scoped core. Activity recreation must not restart the VPN or core. */
public final class CoreBridge {
    private static boolean started;
    private static String lastConfig = "";
    private static boolean ipv6Enabled = true;
    private static long generation;
    private CoreBridge() {}
    private static native String nativeColdRuntime(String root, boolean applicationRulesSupported);

    public static synchronized String start(Context context, String raw, boolean ipv6) throws Exception {
        ApplicationOwners.install(context);
        File home = new File(context.getFilesDir(), "client/runtime");
        if (!home.isDirectory() && !home.mkdirs()) throw new IOException(AppLanguage.current(context).getString(R.string.core_directory_failed));
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
        generation++;
        lastConfig = raw;
        ipv6Enabled = ipv6;
        return home.toString();
    }

    public static synchronized void reload(Context context, String raw) throws Exception {
        reload(context, raw, ipv6Enabled);
    }
    public static synchronized void reload(Context context, String raw, boolean ipv6) throws Exception {
        if (!started) throw new IOException(AppLanguage.current(context).getString(R.string.core_not_started));
        Mobile.reload(raw); generation++; lastConfig = raw;
        ipv6Enabled = ipv6;
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
        JSONObject runtime = checkedRuntime(context);
        start(context, runtime.getString("config"), runtime.getBoolean("ipv6"));
    }
    private static JSONObject checkedRuntime(Context context) throws Exception {
        JSONObject response = new JSONObject(nativeColdRuntime(
            new File(context.getFilesDir(), "client").getAbsolutePath(), ApplicationOwners.supported()));
        if (response.has("error")) throw new IOException(response.getString("error"));
        return response;
    }
    static synchronized boolean vpnProtectedMode(Context context) throws Exception {
        if (!protectedMode(context)) return false;
        JSONObject runtime = checkedRuntime(context);
        if (!runtime.getBoolean("protected") || !matchesRuntime(runtime.getString("config"), runtime.getBoolean("ipv6"))) {
            throw new IOException(AppLanguage.current(context).getString(R.string.protection_unconfirmed));
        }
        return true;
    }
    static JSONObject savedSettings(Context context) throws Exception {
        var path = new File(context.getFilesDir(), "client/settings.json").toPath();
        BasicFileAttributes file = Files.readAttributes(path, BasicFileAttributes.class, LinkOption.NOFOLLOW_LINKS);
        int limit = 10 * 1024 * 1024;
        if (!file.isRegularFile() || file.size() > limit) {
            throw new IOException(AppLanguage.current(context).getString(R.string.protection_settings_large));
        }
        byte[] bytes = Files.readAllBytes(path);
        if (bytes.length > limit) throw new IOException(AppLanguage.current(context).getString(R.string.protection_settings_large));
        return new JSONObject(StandardCharsets.UTF_8.newDecoder().decode(java.nio.ByteBuffer.wrap(bytes)).toString());
    }
    static JSONObject savedProtection(JSONObject settings) throws Exception {
        if (settings.isNull("claude_protection")) return null;
        return settings.getJSONObject("claude_protection");
    }
    static boolean protectedMode(Context context) throws Exception {
        return savedProtection(savedSettings(context)) != null;
    }
    public static synchronized String config() { return lastConfig; }
    static synchronized long generation() { return generation; }
    public static synchronized boolean ipv6() { return ipv6Enabled; }
    static synchronized boolean matchesRuntime(String raw, boolean ipv6) {
        return started && ipv6Enabled == ipv6 && lastConfig.equals(raw);
    }
    public static JSONObject state(Context context) throws Exception {
        JSONObject state = new JSONObject().put("running", ready());
        TunnelService.writeState(context, state);
        return state.put("protection", ProtectionRuntime.state())
            .put("application_routing", new JSONObject(Mobile.applicationRoutingDiagnostics()));
    }
}
