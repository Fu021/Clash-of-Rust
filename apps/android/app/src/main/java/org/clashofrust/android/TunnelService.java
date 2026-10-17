package org.clashofrust.android;

import android.app.*;
import android.content.*;
import android.net.VpnService;
import android.os.*;
import java.util.Collections;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import org.clashofrust.nativecore.mobile.Mobile;
import org.json.JSONObject;

/** The foreground service owns the TUN; closing the window leaves it connected. */
public final class TunnelService extends VpnService {
    static final String RECONFIGURE = "org.clashofrust.android.RECONFIGURE";
    private static volatile TunnelService instance;
    private static volatile boolean connected;
    private static volatile String lastError = "";
    private static volatile long generation;
    // One process-wide queue also orders cleanup against a newly created service.
    private static final ExecutorService serial = Executors.newSingleThreadExecutor();
    private ParcelFileDescriptor descriptor;
    private String interfaceName;
    private boolean stopping;
    static boolean connected() { return connected; }
    static String error() { return lastError; }
    static long generation() { return generation; }
    static void stop(Context context) {
        context.getSharedPreferences("vpn", MODE_PRIVATE).edit().putBoolean("desired", false).apply();
        TunnelService active = instance;
        if (active != null) active.serial.execute(active::disconnect);
    }
    @Override public void onCreate() {
        super.onCreate(); instance = this;
        NotificationManager manager = getSystemService(NotificationManager.class);
        manager.createNotificationChannel(new NotificationChannel("vpn", "VPN 连接", NotificationManager.IMPORTANCE_LOW));
    }
    @Override public int onStartCommand(Intent intent, int flags, int startId) {
        boolean restore = intent == null;
        boolean alwaysOn = Build.VERSION.SDK_INT >= 29 && isAlwaysOn();
        if (restore && !alwaysOn && !getSharedPreferences("vpn", MODE_PRIVATE).getBoolean("desired", false)) {
            stopSelf(); return START_NOT_STICKY;
        }
        getSharedPreferences("vpn", MODE_PRIVATE).edit().putBoolean("desired", true).apply();
        startForeground(1, notification("正在连接…"));
        boolean reconfigure = intent != null && RECONFIGURE.equals(intent.getAction());
        serial.execute(() -> connect(reconfigure));
        return START_STICKY;
    }
    private void connect(boolean reconfigure) {
        if (stopping || (descriptor != null && !reconfigure)) return;
        ParcelFileDescriptor previous = descriptor;
        ParcelFileDescriptor replacement = null;
        boolean established = false;
        try {
            CoreBridge.restore(this);
            if (VpnService.prepare(this) != null) throw new IllegalStateException("VPN 授权已失效，请打开应用重新连接");
            boolean ipv6 = CoreBridge.ipv6();
            Builder builder = new Builder().setSession("Clash of Rust").setMtu(1500)
                .addAddress("198.18.0.1", 30).addRoute("0.0.0.0", 0).addDnsServer("198.18.0.2");
            if (ipv6) builder.addAddress("fdfe:dcba:9876::1", 126).addRoute("::", 0).addDnsServer("fdfe:dcba:9876::2");
            // Do not allow IPv6 to escape the VPN when disabled: omitting its family blocks it.
            java.util.Set<String> exclusions=getSharedPreferences("vpn", MODE_PRIVATE).getStringSet("excluded",Collections.emptySet());
            java.util.Set<String> valid=new java.util.HashSet<>();
            // Broad protection must cover excluded apps too; keep the user's
            // preferences intact so disconnecting protection restores them.
            for (String name : CoreBridge.protectedMode(this) ? Collections.<String>emptySet() : exclusions) {
                try {builder.addDisallowedApplication(name);valid.add(name);}
                catch (android.content.pm.PackageManager.NameNotFoundException removed) {
                    android.util.Log.i("ClashOfRust","Removed an uninstalled application from VPN exclusions");
                }
            }
            if (!CoreBridge.protectedMode(this) && !valid.equals(exclusions))
                getSharedPreferences("vpn",MODE_PRIVATE).edit().putStringSet("excluded",valid).apply();
            if (Build.VERSION.SDK_INT >= 29) builder.setMetered(false);
            Intent open = new Intent(this, MainActivity.class);
            builder.setConfigureIntent(PendingIntent.getActivity(this, 0, open, PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT));
            // Establish before closing the old interface. Android deactivates
            // the old VPN atomically when the replacement succeeds; closing it
            // first exposes the underlying network during every reconfigure.
            try (VpnRouting routes=new VpnRouting(this,previous==null ? null : interfaceName,ipv6)) {
                ProtectionRuntime.pause(this);
                if (previous == null) Mobile.setProtector(fd -> protect(fd));
                replacement = builder.establish();
                if (replacement == null) throw new IllegalStateException("系统拒绝建立 VPN");
                established = true;
                connected = false;
                if (previous != null) Mobile.stopTun();
                android.util.Log.i("ClashOfRust", "VPN established; starting embedded TUN");
                Mobile.startTun(replacement.getFd(), ipv6);
                descriptor = replacement;
                replacement = null;
                // The VPN Network object can remain unchanged while netd is
                // still moving its routes. Publish readiness only after the
                // new interface arrives through the ordered network callback.
                interfaceName=routes.await();
                closeDescriptor(previous);
                previous = null;
                android.util.Log.i("ClashOfRust", "VPN routing ready on "+interfaceName);
            }
            connected = true; lastError = "";
            generation++;
            ProtectionRuntime.sync(this);
            VpnTileService.refresh(this);
            android.util.Log.i("ClashOfRust", "Embedded TUN started");
            getSystemService(NotificationManager.class).notify(1, notification("VPN 已连接"));
            MainActivity.emit("vpn", true);
        } catch (Exception error) {
            closeDescriptor(replacement);
            lastError = error.toString();
            android.util.Log.e("ClashOfRust", "VPN startup failed", error);
            boolean keepPrevious = false;
            if (previous != null && !established) {
                try { keepPrevious = VpnService.prepare(this) == null; }
                catch (RuntimeException revoked) { android.util.Log.w("ClashOfRust", "VPN permission is unavailable", revoked); }
            }
            if (keepPrevious) {
                // A rejected replacement leaves the old descriptor valid.
                // Resume its guard and keep the working connection available.
                connected = true;
                try { ProtectionRuntime.sync(this); }
                catch (Exception guard) {
                    lastError += "; 保护恢复失败：" + guard;
                    android.util.Log.e("ClashOfRust", "Restore protection after rejected VPN replacement", guard);
                }
                try {
                    VpnTileService.refresh(this);
                    getSystemService(NotificationManager.class).notify(1, notification("VPN 重建失败，原连接已保留"));
                } catch (RuntimeException status) { android.util.Log.w("ClashOfRust", "Update preserved VPN status", status); }
                MainActivity.emit("error", "VPN 重建失败，原连接已保留：" + lastError);
                return;
            }
            if (previous != null) {
                closeDescriptor(previous);
                if (descriptor == previous) descriptor = null;
            }
            MainActivity.emit("error", "VPN 启动失败：" + lastError);
            disconnect();
        }
    }
    private static void closeDescriptor(ParcelFileDescriptor value) {
        if (value == null) return;
        try { value.close(); }
        catch (Exception error) { android.util.Log.e("ClashOfRust", "Close TUN descriptor", error); }
    }
    private void cleanup() {
        connected = false;
        VpnTileService.refresh(this);
        try { Mobile.stopTun(); } catch (Exception error) { android.util.Log.e("ClashOfRust", "Stop TUN", error); }
        closeDescriptor(descriptor);
        descriptor = null;
        interfaceName = null;
        Mobile.setProtector(null);
        try { ProtectionRuntime.sync(this); }
        catch (Exception error) { android.util.Log.e("ClashOfRust", "Stop protection", error); }
        MainActivity.emit("vpn", false);
    }
    private void disconnect() {
        stopping = true;
        getSharedPreferences("vpn", MODE_PRIVATE).edit().putBoolean("desired", false).apply();
        cleanup();
        stopForeground(STOP_FOREGROUND_REMOVE);
        stopSelf();
    }
    @Override public void onRevoke() {
        lastError = "VPN 授权已被系统撤回";
        serial.execute(this::disconnect);
    }
    @Override public void onDestroy() {
        instance = null;
        serial.execute(this::cleanup);
        super.onDestroy();
    }
    private Notification notification(String text) {
        PendingIntent open = PendingIntent.getActivity(this, 0, new Intent(this, MainActivity.class),
            PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT);
        return new Notification.Builder(this, "vpn").setContentTitle("Clash of Rust")
            .setContentText(text).setSmallIcon(R.drawable.ic_app).setOngoing(true).setContentIntent(open).build();
    }
    static void updateTraffic(Context context, String down, String up) {
        TunnelService active = instance;
        if (active != null && connected && context.getSharedPreferences("appearance", MODE_PRIVATE)
            .getBoolean("notification-speeds", true)
            && (Build.VERSION.SDK_INT < 33 || context.checkSelfPermission(android.Manifest.permission.POST_NOTIFICATIONS)
                == android.content.pm.PackageManager.PERMISSION_GRANTED))
            active.getSystemService(NotificationManager.class).notify(1, active.notification("↓ " + down + "  ↑ " + up));
    }
}
