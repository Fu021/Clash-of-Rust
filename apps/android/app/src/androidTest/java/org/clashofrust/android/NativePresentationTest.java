package org.clashofrust.android;

import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.content.Context;
import androidx.test.ext.junit.runners.AndroidJUnit4;
import androidx.test.platform.app.InstrumentationRegistry;
import java.io.IOException;
import java.util.UUID;
import org.junit.Test;
import org.junit.runner.RunWith;
import static org.junit.Assert.*;

@RunWith(AndroidJUnit4.class)
public class NativePresentationTest {
    @Test public void localizedErrorsAndChannelRename() {
        verify(InstrumentationRegistry.getInstrumentation().getTargetContext());
    }

    public static void verify(Context context) {
        renamingAnExistingChannelKeepsItsSettings(context);
        errorSummariesUseTheSelectedLanguageAndKeepTheException(context);
    }

    private static void renamingAnExistingChannelKeepsItsSettings(Context context) {
        Context english = AppLanguage.localized(context, "en");
        Context chinese = AppLanguage.localized(context, "zh-CN");
        NotificationManager manager = context.getSystemService(NotificationManager.class);
        String id = "vpn-name-test-" + UUID.randomUUID();
        String englishName = english.getString(R.string.vpn_channel);
        String chineseName = chinese.getString(R.string.vpn_channel);
        assertNotEquals(englishName, chineseName);

        try {
            SystemNotifications.renameChannel(manager, manager.getNotificationChannel(id), chineseName);
            assertNull(manager.getNotificationChannel(id));

            NotificationChannel channel = new NotificationChannel(id, englishName, NotificationManager.IMPORTANCE_MIN);
            channel.setShowBadge(false);
            channel.enableVibration(true);
            manager.createNotificationChannel(channel);
            NotificationChannel original = manager.getNotificationChannel(id);
            assertNotNull(original);
            assertEquals(englishName, original.getName().toString());

            for (String name : new String[]{chineseName, englishName}) {
                SystemNotifications.renameChannel(manager, manager.getNotificationChannel(id), name);
                NotificationChannel renamed = manager.getNotificationChannel(id);
                assertNotNull(renamed);
                assertEquals(name, renamed.getName().toString());
                assertEquals(original.getImportance(), renamed.getImportance());
                assertEquals(original.canShowBadge(), renamed.canShowBadge());
                assertEquals(original.shouldVibrate(), renamed.shouldVibrate());
                assertEquals(original.getSound(), renamed.getSound());
            }
        } finally {
            manager.deleteNotificationChannel(id);
        }
    }

    private static void errorSummariesUseTheSelectedLanguageAndKeepTheException(Context context) {
        Context english = AppLanguage.localized(context, "en");
        Context chinese = AppLanguage.localized(context, "zh-CN");
        IOException detail = new IOException("Provider returned \"denied\"");
        int[] messages = {R.string.vpn_authorization_open_failed, R.string.import_failed,
            R.string.export_failed, R.string.notification_send_failed};
        String[] englishSummaries = {"Could not open VPN authorization", "Could not import the file",
            "Could not export the file", "Could not send the notification"};
        String[] chineseSummaries = {"无法打开 VPN 授权", "无法导入文件", "无法导出文件", "无法发送系统通知"};

        for (int index = 0; index < messages.length; index++) {
            assertEquals(englishSummaries[index] + ": " + detail, english.getString(messages[index], detail));
            assertEquals(chineseSummaries[index] + "：" + detail, chinese.getString(messages[index], detail));
        }
    }
}
