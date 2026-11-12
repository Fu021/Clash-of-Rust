package org.clashofrust.android;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.content.Context;
import android.content.Intent;
import android.content.SharedPreferences;
import android.os.SystemClock;
import org.json.JSONObject;

final class IncidentNotifications {
    static final String RECOVERY_CHANNEL = "runtime-recovery";
    static final String CRITICAL_CHANNEL = "runtime-critical";
    static final int RECOVERY_ID = 3, CRITICAL_ID = 4, APPLICATION_ID = 5;
    private static final long[] lastAttempts = {-1, -1, -1};
    private static long attempted, sent, suppressed, denied;
    private static String delivery = "not_requested";

    private IncidentNotifications() {}

    static boolean eligible(long previous, long now) {
        return previous < 0 || now < previous || now - previous >= 60_000;
    }

    static String message(String code) {
        if ("application-target-missing".equals(code))
            return "有应用连接因指定出口失效被拦截，请在应用规则中更换出口。";
        return "critical".equals(code)
            ? "发生严重错误，请打开应用查看详情并导出诊断。"
            : "发生可恢复错误，请打开应用查看详情并导出诊断。";
    }

    private static synchronized void record(Context context, String code, boolean critical) {
        SharedPreferences preferences = context.getSharedPreferences("runtime-incidents", Context.MODE_PRIVATE);
        long sequence = Math.max(System.currentTimeMillis(), preferences.getLong("sequence", 0) + 1);
        preferences.edit().putLong("sequence", sequence).putString("code", code)
            .putBoolean("critical", critical).apply();
    }

    static void opened(Context context, Intent intent) {
        String code = intent.getStringExtra("runtime_incident");
        if (!"recovery".equals(code) && !"critical".equals(code) && !"application-target-missing".equals(code)) return;
        try { record(context, code, "critical".equals(code)); }
        catch (RuntimeException failure) { android.util.Log.w("ClashOfRust", "Incident reminder unavailable"); }
    }

    @android.annotation.SuppressLint("MissingPermission")
    static synchronized boolean publish(Context context, String code, boolean recordForUi) {
        int index;
        switch (code) {
            case "recovery": index = 0; break;
            case "critical": index = 1; break;
            case "application-target-missing": index = 2; break;
            default: throw new IllegalArgumentException("错误通知类型无效");
        }
        attempted++;
        long now = SystemClock.elapsedRealtime();
        if (!eligible(lastAttempts[index], now)) { suppressed++; delivery = "rate_limited"; return false; }
        lastAttempts[index] = now;
        boolean critical = index == 1;
        try {
            if (recordForUi) {
                record(context, code, critical);
            }
            if (!SystemNotifications.permitted(context)) {
                denied++; delivery = "permission_denied"; return false;
            }
            NotificationManager manager = context.getSystemService(NotificationManager.class);
            if (!manager.areNotificationsEnabled()) {
                denied++; delivery = "notifications_disabled"; return false;
            }
            String channel = critical ? CRITICAL_CHANNEL : RECOVERY_CHANNEL;
            manager.createNotificationChannel(new NotificationChannel(channel,
                critical ? "严重错误" : "可恢复错误", critical
                    ? NotificationManager.IMPORTANCE_HIGH : NotificationManager.IMPORTANCE_DEFAULT));
            if (manager.getNotificationChannel(channel).getImportance() == NotificationManager.IMPORTANCE_NONE) {
                denied++; delivery = "channel_disabled"; return false;
            }
            int identity = critical ? CRITICAL_ID : index == 2 ? APPLICATION_ID : RECOVERY_ID;
            PendingIntent open = PendingIntent.getActivity(context, identity,
                new Intent(context, MainActivity.class).putExtra("runtime_incident", code)
                    .addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
                PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT);
            String title = index == 2 ? "应用出口不可用" : critical ? "Clash of Rust · 严重错误" : "Clash of Rust · 需要处理";
            manager.notify(identity, new Notification.Builder(context, channel)
                .setSmallIcon(R.drawable.ic_app).setContentTitle(title).setContentText(message(code))
                .setStyle(new Notification.BigTextStyle().bigText(message(code)))
                .setVisibility(Notification.VISIBILITY_PRIVATE).setContentIntent(open)
                .setAutoCancel(true).setOnlyAlertOnce(true).build());
            sent++; delivery = "posted";
            return true;
        } catch (RuntimeException failure) {
            if (failure instanceof SecurityException) denied++;
            delivery = failure instanceof SecurityException ? "permission_denied" : "delivery_failed";
            android.util.Log.w("ClashOfRust", "Incident notification unavailable: " + failure.getClass().getSimpleName());
            return false;
        }
    }

    static synchronized JSONObject state(Context context) throws Exception {
        SharedPreferences preferences = context.getSharedPreferences("runtime-incidents", Context.MODE_PRIVATE);
        String code = preferences.getString("code", "");
        NotificationManager manager = context.getSystemService(NotificationManager.class);
        return new JSONObject().put("attempted", attempted).put("sent", sent)
            .put("suppressed", suppressed).put("denied", denied).put("delivery", delivery)
            .put("permitted", SystemNotifications.permitted(context))
            .put("notifications_enabled", manager.areNotificationsEnabled())
            .put("recovery_channel", channelState(manager, RECOVERY_CHANNEL))
            .put("critical_channel", channelState(manager, CRITICAL_CHANNEL))
            .put("sequence", preferences.getLong("sequence", 0) > preferences.getLong("acknowledged", 0)
                ? preferences.getLong("sequence", 0) : 0).put("code", code)
            .put("critical", preferences.getBoolean("critical", false))
            .put("message", code.isEmpty() ? "" : message(code));
    }

    private static String channelState(NotificationManager manager, String id) {
        NotificationChannel channel = manager.getNotificationChannel(id);
        return channel == null ? "not_created" : channel.getImportance() == NotificationManager.IMPORTANCE_NONE
            ? "disabled" : "enabled";
    }

    static synchronized void acknowledge(Context context, long sequence) {
        SharedPreferences preferences = context.getSharedPreferences("runtime-incidents", Context.MODE_PRIVATE);
        if (preferences.getLong("sequence", 0) == sequence)
            preferences.edit().putLong("acknowledged", sequence).apply();
    }
}
