package org.clashofrust.android;

import android.app.*;
import android.content.*;
import android.net.VpnService;
import android.os.*;
import java.util.Collections;
import java.util.concurrent.ScheduledExecutorService;
import java.util.concurrent.ScheduledFuture;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.Executors;
import org.clashofrust.nativecore.mobile.Mobile;
import org.json.JSONObject;

/** The foreground service owns the TUN; closing the window leaves it connected. */
public final class TunnelService extends VpnService {
    static final String RECONFIGURE = "org.clashofrust.android.RECONFIGURE";
    static final String REQUEST = "vpn_request";
    private static final Object requests = new Object();
    private static volatile TunnelService instance;
    private static volatile boolean connected;
    private static volatile String lastError = "";
    private static volatile long generation;
    // One process-wide queue also orders cleanup against a newly created service.
    private static final ScheduledExecutorService serial = Executors.newSingleThreadScheduledExecutor();
    private ScheduledFuture<?> notificationTask;
    private ParcelFileDescriptor descriptor;
    private String interfaceName;
    private volatile boolean stopping;
    private volatile String lastStatus = "";
    private volatile int latestStartId;
    private volatile String acceptedRequest;
    static String prepareStart(Context context) {
        synchronized (requests) {
            String request = java.util.UUID.randomUUID().toString();
            context.getSharedPreferences("vpn", MODE_PRIVATE).edit().putString(REQUEST, request).apply();
            return request;
        }
    }
    static boolean current(Context context, String request) {
        return request != null && request.equals(context.getSharedPreferences("vpn", MODE_PRIVATE).getString(REQUEST, ""));
    }
    static void start(Context context, boolean reconfigure) { start(context, reconfigure, prepareStart(context)); }
    static void start(Context context, boolean reconfigure, String request) {
        synchronized (requests) {
            if (!current(context, request)) return;
            Intent intent = new Intent(context, TunnelService.class).putExtra(REQUEST, request);
            if (reconfigure) intent.setAction(RECONFIGURE);
            context.startForegroundService(intent);
        }
    }
    static boolean connected() { return connected; }
    static boolean pending() {
        TunnelService active = instance;
        return active != null && !connected && !active.stopping && current(active, active.acceptedRequest);
    }
    static String error() { return lastError; }
    static void clearError() { lastError = ""; }
    static long generation() { return generation; }
    static void stop(Context context) {
        synchronized (requests) {
            String request = prepareStart(context);
            context.getSharedPreferences("vpn", MODE_PRIVATE).edit().putBoolean("desired", false).apply();
            TunnelService active = instance;
            if (active != null) serial.execute(() -> active.disconnect(request));
        }
    }
    @Override public void onCreate() {
        super.onCreate(); instance = this;
        NotificationManager manager = getSystemService(NotificationManager.class);
        manager.createNotificationChannel(new NotificationChannel("vpn", "VPN 连接", NotificationManager.IMPORTANCE_LOW));
        notificationTask = serial.scheduleWithFixedDelay(this::updateStatus, 1, 1, TimeUnit.SECONDS);
    }
    @Override public int onStartCommand(Intent intent, int flags, int startId) {
        synchronized (requests) {
            latestStartId = startId;
            String request = intent == null ? null : intent.getStringExtra(REQUEST);
            if (request != null && !current(this, request)) {
                // A previously queued foreground start must not undo a later stop.
                // Android's foreground deadline belongs to the service, even
                // when this request is stale and a newer start prevents stopSelf.
                startForeground(1, notification(connected ? "VPN 已连接" : "VPN 连接已取消"));
                if (current(this, acceptedRequest)) return START_STICKY;
                stopForeground(STOP_FOREGROUND_REMOVE);
                stopSelfResult(startId);
                return START_NOT_STICKY;
            }
            boolean restore = intent == null;
            boolean alwaysOn = Build.VERSION.SDK_INT >= 29 && isAlwaysOn();
            if (restore && !alwaysOn && !getSharedPreferences("vpn", MODE_PRIVATE).getBoolean("desired", false)) {
                startForeground(1, notification("VPN 连接已取消"));
                stopForeground(STOP_FOREGROUND_REMOVE);
                stopSelfResult(startId); return START_NOT_STICKY;
            }
            if (request == null) request = prepareStart(this);
            acceptedRequest = request;
            stopping = false;
            getSharedPreferences("vpn", MODE_PRIVATE).edit().putBoolean("desired", true).apply();
            clearError();
            startForeground(1, notification("正在连接…"));
            boolean reconfigure = intent != null && RECONFIGURE.equals(intent.getAction());
            String accepted = request;
            serial.execute(() -> connect(reconfigure, accepted));
            VpnTileService.refresh(this);
            return START_STICKY;
        }
    }
    private void connect(boolean reconfigure, String request) {
        if (stopping || !current(this, request) || (descriptor != null && !reconfigure)) return;
        ParcelFileDescriptor previous = descriptor;
        ParcelFileDescriptor replacement = null;
        boolean established = false;
        try {
            CoreBridge.restore(this);
            if (stopping || !current(this, request)) return;
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
                if (stopping || !current(this, request)) return;
                ProtectionRuntime.pause(this);
                if (previous == null) Mobile.setProtector(fd -> protect(fd));
                replacement = builder.establish();
                if (replacement == null) throw new IllegalStateException("系统拒绝建立 VPN");
                established = true;
                connected = false;
                // establish() returns before ConnectivityService/netd finishes
                // moving the existing VPN Network to the new interface. Do
                // not start its packet reader against that transitional device.
                // gVisor stops its reader after a device read failure; publishing
                // routes afterwards does not restart that reader.
                String readyInterface=routes.await(() -> stopping || !current(this, request));
                if (previous != null) Mobile.stopTun();
                android.util.Log.i("ClashOfRust", "VPN established; starting embedded TUN");
                Mobile.startTun(replacement.getFd(), ipv6);
                descriptor = replacement;
                replacement = null;
                interfaceName=readyInterface;
                closeDescriptor(previous);
                previous = null;
                android.util.Log.i("ClashOfRust", "VPN routing ready on "+interfaceName);
            }
            if (stopping || !current(this, request)) { cleanup(); return; }
            connected = true; lastError = "";
            generation++;
            ProtectionRuntime.sync(this);
            VpnTileService.refresh(this);
            android.util.Log.i("ClashOfRust", "Embedded TUN started");
            getSystemService(NotificationManager.class).notify(1, notification("VPN 已连接"));
            MainActivity.emit("vpn", true);
        } catch (Exception error) {
            closeDescriptor(replacement);
            if (stopping || !current(this, request)) {
                if (previous != descriptor) closeDescriptor(previous);
                cleanup(); return;
            }
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
            disconnect(request);
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
    private void disconnect(String request) {
        synchronized (requests) {
            if (!current(this, request)) return;
            stopping = true;
            getSharedPreferences("vpn", MODE_PRIVATE).edit().putBoolean("desired", false).apply();
        }
        cleanup();
        synchronized (requests) {
            // A new explicit connect can arrive while cleanup is in progress.
            if (!current(this, request)) { stopping = false; return; }
            stopForeground(STOP_FOREGROUND_REMOVE);
            if (!stopSelfResult(latestStartId)) stopping = false;
        }
    }
    @Override public void onRevoke() {
        lastError = "VPN 授权已被系统撤回";
        stop(this);
    }
    @Override public void onDestroy() {
        stopping = true;
        if (notificationTask != null) notificationTask.cancel(false);
        if (instance == this) instance = null;
        serial.execute(this::cleanup);
        super.onDestroy();
    }
    private Notification notification(String text) {
        lastStatus = text;
        String[] lines = text.split("\n", 2);
        String title = lines.length > 1 ? lines[0] : "Clash of Rust";
        String content = lines.length > 1 ? lines[1] : text;
        PendingIntent open = PendingIntent.getActivity(this, 0, new Intent(this, MainActivity.class),
            PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT);
        return new Notification.Builder(this, "vpn").setContentTitle(title)
            .setContentText(content).setStyle(new Notification.BigTextStyle().bigText(content))
            .setOnlyAlertOnce(true).setShowWhen(false)
            .setSmallIcon(R.drawable.ic_app).setOngoing(true).setContentIntent(open).build();
    }
    static String statusText(String down, String up, String node, String mode, boolean speeds) {
        String label;
        switch (mode) {
            case "global": label = "全局"; break;
            case "rule": label = "规则"; break;
            case "direct": label = "直连"; break;
            default: label = "未知模式"; break;
        }
        String text = label + " · " + (node == null || node.isEmpty() ? "节点待确认" : node);
        return speeds ? text + "\n↓ " + down + "  ↑ " + up : text;
    }
    private static String speed(long bytes) {
        if (bytes < 1024) return bytes + " B/s";
        double value = bytes;
        String[] units = {"KiB/s", "MiB/s", "GiB/s", "TiB/s"};
        int unit = 0;
        value /= 1024;
        while (value >= 1024 && unit < units.length - 1) { value /= 1024; unit++; }
        return String.format(java.util.Locale.ROOT, "%.1f %s", value, units[unit]);
    }
    private void updateStatus() {
        if (instance != this || !connected || stopping) return;
        if (Build.VERSION.SDK_INT >= 33 && checkSelfPermission(android.Manifest.permission.POST_NOTIFICATIONS)
            != android.content.pm.PackageManager.PERMISSION_GRANTED) return;
        try {
            String raw = Mobile.notificationStatus();
            if (raw.isEmpty()) return;
            JSONObject state = new JSONObject(raw);
            String text = statusText(speed(state.optLong("down")), speed(state.optLong("up")),
                state.optString("node"), state.optString("mode"),
                getSharedPreferences("appearance", MODE_PRIVATE).getBoolean("notification-speeds", true));
            synchronized (requests) {
                if (instance == this && connected && !stopping && !text.equals(lastStatus))
                    getSystemService(NotificationManager.class).notify(1, notification(text));
            }
        } catch (Exception error) {
            android.util.Log.w("ClashOfRust", "VPN notification status unavailable", error);
        }
    }
}
