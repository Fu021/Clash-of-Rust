package org.clashofrust.android;

import android.content.Context;
import java.io.*;
import java.nio.file.Files;
import java.nio.file.StandardCopyOption;
import org.json.JSONObject;
import org.clashofrust.nativecore.mobile.Mobile;

/** Process-scoped core. Activity recreation must not restart the VPN or core. */
public final class CoreBridge {
    private static boolean started;
    private static String lastConfig = "";
    private CoreBridge() {}

    public static synchronized String start(Context context, String raw) throws Exception {
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
        return home.toString();
    }

    public static synchronized void reload(String raw) throws Exception {
        if (!started) throw new IOException("内核尚未启动");
        Mobile.reload(raw); lastConfig = raw;
    }
    public static synchronized boolean ready() { return started; }
    public static synchronized String config() { return lastConfig; }
    public static JSONObject state() throws Exception {
        return new JSONObject().put("running", ready()).put("vpn", TunnelService.connected())
            .put("error", TunnelService.error());
    }
}
