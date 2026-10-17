package org.clashofrust.android;

import android.Manifest;
import android.app.*;
import android.content.Context;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.os.Build;

/** The logo test and protection alerts use this exact channel and sender. */
final class SystemNotifications {
    static final String CHANNEL = "protection";
    private SystemNotifications() {}
    static boolean permitted(Context context) {
        return Build.VERSION.SDK_INT < 33
            || context.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) == PackageManager.PERMISSION_GRANTED;
    }
    @android.annotation.SuppressLint("MissingPermission") // Explicitly checked before notify.
    static void show(Context context, String title, String message) {
        if (!permitted(context)) throw new SecurityException("请允许 Clash of Rust 发送系统通知");
        if (title.length() > 128 || message.length() > 2048 || title.indexOf('\0') >= 0 || message.indexOf('\0') >= 0)
            throw new IllegalArgumentException("系统通知内容无效");
        NotificationManager manager = context.getSystemService(NotificationManager.class);
        if (!manager.areNotificationsEnabled()) throw new SecurityException("系统已关闭应用通知，请在通知设置中允许");
        manager.createNotificationChannel(new NotificationChannel(CHANNEL, "Claude 保护与通知测试", NotificationManager.IMPORTANCE_HIGH));
        if (manager.getNotificationChannel(CHANNEL).getImportance() == NotificationManager.IMPORTANCE_NONE)
            throw new SecurityException("系统已关闭 Claude 保护通知，请在通知设置中允许");
        PendingIntent open = PendingIntent.getActivity(context, 2, new Intent(context, MainActivity.class),
            PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT);
        manager.notify(2, new Notification.Builder(context, CHANNEL).setSmallIcon(R.drawable.ic_app)
            .setContentTitle(title).setContentText(message)
            .setStyle(new Notification.BigTextStyle().bigText(message))
            .setContentIntent(open).setAutoCancel(true).build());
    }
}
