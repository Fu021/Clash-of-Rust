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
        serial.execute(() -> {
            if (reconfigure) cleanup();
            connect();
        });
        return START_STICKY;
    }
    private void connect() {
        if (descriptor != null || stopping) return;
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
            Mobile.setProtector(fd -> protect(fd));
            descriptor = builder.establish();
            if (descriptor == null) throw new IllegalStateException("系统拒绝建立 VPN");
            android.util.Log.i("ClashOfRust", "VPN established; starting embedded TUN");
            Mobile.startTun(descriptor.getFd(), ipv6);
            connected = true; lastError = "";
            generation++;
            ProtectionRuntime.sync(this);
            VpnTileService.refresh(this);
            android.util.Log.i("ClashOfRust", "Embedded TUN started");
            getSystemService(NotificationManager.class).notify(1, notification("VPN 已连接"));
            MainActivity.emit("vpn", true);
        } catch (Exception error) {
            lastError = error.toString();
            android.util.Log.e("ClashOfRust", "VPN startup failed", error);
            MainActivity.emit("error", "VPN 启动失败：" + lastError);
            disconnect();
        }
    }
    private void cleanup() {
        connected = false;
        VpnTileService.refresh(this);
        try { Mobile.stopTun(); } catch (Exception error) { android.util.Log.e("ClashOfRust", "Stop TUN", error); }
        if (descriptor != null) {
            try { descriptor.close(); } catch (Exception error) { android.util.Log.e("ClashOfRust", "Close TUN descriptor", error); }
            descriptor = null;
        }
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
