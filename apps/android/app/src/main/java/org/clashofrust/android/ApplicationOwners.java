package org.clashofrust.android;

import android.content.Context;
import android.content.pm.PackageManager;
import android.net.ConnectivityManager;
import android.os.Build;
import java.net.InetAddress;
import java.net.InetSocketAddress;
import org.clashofrust.nativecore.mobile.ConnectionOwner;
import org.clashofrust.nativecore.mobile.Mobile;
import org.json.JSONObject;

final class ApplicationOwners implements ConnectionOwner {
    private final Context context;

    private ApplicationOwners(Context context) {
        this.context = context.getApplicationContext();
    }

    static boolean supported() {
        return Build.VERSION.SDK_INT >= 29;
    }

    static JSONObject capability() throws Exception {
        return new JSONObject().put("supported", supported())
            .put("reason", supported() ? "" : "按应用指定出口需要 Android 10 或更新版本；当前仍可使用全部、仅选中和排除模式。");
    }

    static void install(Context context) {
        Mobile.setConnectionOwner(supported() ? new ApplicationOwners(context) : null);
    }

    @Override public int packageUID(String packageName) {
        try {
            return context.getPackageManager().getApplicationInfo(packageName, 0).uid;
        } catch (PackageManager.NameNotFoundException removed) {
            return -1;
        }
    }

    @android.annotation.TargetApi(29)
    @Override public int owner(int protocol, String source, int sourcePort,
            String destination, int destinationPort) throws Exception {
        if (!supported() || (protocol != 6 && protocol != 17)) return -1;
        ConnectivityManager manager = context.getSystemService(ConnectivityManager.class);
        if (manager == null) return -1;
        try {
            return manager.getConnectionOwnerUid(protocol,
                new InetSocketAddress(InetAddress.getByName(source), sourcePort),
                new InetSocketAddress(InetAddress.getByName(destination), destinationPort));
        } catch (SecurityException denied) {
            return -1;
        }
    }
}
