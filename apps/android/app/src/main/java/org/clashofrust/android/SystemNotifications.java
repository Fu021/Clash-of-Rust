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
        Context localized = AppLanguage.current(context);
        if (!permitted(context)) throw new SecurityException(localized.getString(R.string.notification_permission));
        if (title.length() > 128 || message.length() > 2048 || title.indexOf('\0') >= 0 || message.indexOf('\0') >= 0)
            throw new IllegalArgumentException(localized.getString(R.string.notification_invalid_content));
        NotificationManager manager = context.getSystemService(NotificationManager.class);
        if (!manager.areNotificationsEnabled()) throw new SecurityException(localized.getString(R.string.notification_disabled));
        manager.createNotificationChannel(new NotificationChannel(CHANNEL, localized.getString(R.string.notification_protection_channel), NotificationManager.IMPORTANCE_HIGH));
        if (manager.getNotificationChannel(CHANNEL).getImportance() == NotificationManager.IMPORTANCE_NONE)
            throw new SecurityException(localized.getString(R.string.notification_protection_disabled));
        PendingIntent open = PendingIntent.getActivity(context, 2, new Intent(context, MainActivity.class),
            PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT);
        manager.notify(2, new Notification.Builder(context, CHANNEL).setSmallIcon(R.drawable.ic_app)
            .setContentTitle(title).setContentText(message)
            .setStyle(new Notification.BigTextStyle().bigText(message))
            .setContentIntent(open).setAutoCancel(true).build());
    }
}
