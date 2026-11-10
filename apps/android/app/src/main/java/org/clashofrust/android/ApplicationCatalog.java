package org.clashofrust.android;

import android.content.Context;
import android.content.Intent;
import android.content.pm.ApplicationInfo;
import android.content.pm.PackageManager;
import android.graphics.Bitmap;
import android.graphics.Canvas;
import android.graphics.drawable.Drawable;
import android.net.VpnService;
import android.util.AtomicFile;
import java.io.File;
import java.io.FileOutputStream;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.HashSet;
import java.util.LinkedHashMap;
import java.util.Map;
import java.util.Set;
import org.json.JSONArray;
import org.json.JSONObject;

final class ApplicationCatalog {
    static boolean visibleWithoutComponents(ApplicationInfo application, boolean launchable, boolean selected) {
        return launchable || selected || (application.flags & ApplicationInfo.FLAG_SYSTEM) == 0;
    }

    static String icon(Context context, PackageManager manager, ApplicationInfo application) throws Exception {
        File directory = new File(context.getCacheDir(), "application-icons");
        if (!directory.isDirectory() && !directory.mkdirs())
            throw new java.io.IOException("应用图标缓存目录不可写");
        File destination = new File(directory, application.packageName + ".png");
        long modified = application.sourceDir == null ? 0 : new File(application.sourceDir).lastModified();
        if (destination.isFile() && destination.length() > 0 && destination.lastModified() >= modified)
            return destination.getAbsolutePath();
        Drawable drawable = application.loadIcon(manager);
        Bitmap bitmap = Bitmap.createBitmap(72, 72, Bitmap.Config.ARGB_8888);
        AtomicFile output = new AtomicFile(destination);
        FileOutputStream stream = null;
        try {
            drawable.setBounds(0, 0, bitmap.getWidth(), bitmap.getHeight());
            drawable.draw(new Canvas(bitmap));
            stream = output.startWrite();
            if (!bitmap.compress(Bitmap.CompressFormat.PNG, 100, stream))
                throw new java.io.IOException("应用图标转换失败");
            output.finishWrite(stream);
            return destination.getAbsolutePath();
        } catch (Exception error) {
            if (stream != null) output.failWrite(stream);
            throw error;
        } finally {
            bitmap.recycle();
        }
    }

    static JSONObject read(Context context) throws Exception {
        PackageManager manager = context.getPackageManager();
        ApplicationRouting saved = ApplicationRouting.load(context);
        Set<String> launchable = new HashSet<>();
        Map<String, ApplicationInfo> applications = new LinkedHashMap<>();
        for (ApplicationInfo application : manager.getInstalledApplications(0))
            applications.put(application.packageName, application);
        Intent launcher = new Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_LAUNCHER);
        for (android.content.pm.ResolveInfo activity : manager.queryIntentActivities(launcher, 0)) {
            if (activity.activityInfo == null) continue;
            launchable.add(activity.activityInfo.packageName);
            applications.put(activity.activityInfo.packageName, activity.activityInfo.applicationInfo);
        }
        File[] cached = new File(context.getCacheDir(), "application-icons").listFiles();
        if (cached != null) {
            for (File file : cached) {
                String name = file.getName();
                if (name.endsWith(".png") && !applications.containsKey(name.substring(0, name.length() - 4))) {
                    try {
                        manager.getApplicationInfo(name.substring(0, name.length() - 4), 0);
                    } catch (PackageManager.NameNotFoundException removed) {
                        new AtomicFile(file).delete();
                    }
                }
            }
        }
        ArrayList<JSONObject> entries = new ArrayList<>();
        int other = 0, iconFailures = 0;
        for (ApplicationInfo application : applications.values()) {
            String label;
            try { label = application.loadLabel(manager).toString(); }
            catch (RuntimeException error) { label = application.packageName; }
            String image = "";
            try { image = icon(context, manager, application); }
            catch (Exception error) { iconFailures++; }
            if (!context.getPackageName().equals(application.packageName)
                    && visibleWithoutComponents(application, launchable.contains(application.packageName),
                        saved.packages.contains(application.packageName))) other++;
            entries.add(new JSONObject().put("name", label).put("package", application.packageName)
                .put("icon", image)
                .put("system", (application.flags & ApplicationInfo.FLAG_SYSTEM) != 0)
                .put("launchable", launchable.contains(application.packageName)));
        }
        entries.sort(Comparator.comparing((JSONObject item) -> item.optString("name"), String.CASE_INSENSITIVE_ORDER)
            .thenComparing(item -> item.optString("package")));
        ArrayList<String> selected = new ArrayList<>(saved.packages);
        java.util.Collections.sort(selected);
        return new JSONObject().put("applications", new JSONArray(entries))
            .put("mode", saved.mode).put("selected", new JSONArray(selected))
            .put("summary", saved.summary()).put("visibility_limited", other == 0)
            .put("query_permission", android.os.Build.VERSION.SDK_INT < 30
                || manager.checkPermission("android.permission.QUERY_ALL_PACKAGES", context.getPackageName())
                    == PackageManager.PERMISSION_GRANTED)
            .put("icon_failures", iconFailures);
    }

    static JSONObject save(Context context, JSONObject data) throws Exception {
        if (CoreBridge.protectedMode(context))
            throw new IllegalStateException("请先关闭 Claude 保护，再修改应用分流");
        Set<String> selected = new HashSet<>();
        JSONArray packages = data.getJSONArray("selected");
        for (int index = 0; index < packages.length(); index++) selected.add(packages.getString(index));
        ApplicationRouting routing = new ApplicationRouting(data.getString("mode"), selected);
        if (TunnelService.pending()) throw new IllegalStateException("VPN 正在连接，请稍后保存");
        routing.save(context);
        boolean reconnect = TunnelService.connected();
        String reconnectError = "";
        if (reconnect) {
            try {
                if (VpnService.prepare(context) != null)
                    throw new IllegalStateException("VPN 授权已失效，请重新连接");
                TunnelService.start(context, true);
            } catch (RuntimeException error) { reconnectError = error.toString(); }
        }
        return new JSONObject().put("summary", routing.summary()).put("connected", reconnect)
            .put("reconnecting", reconnect && reconnectError.isEmpty()).put("reconnect_error", reconnectError);
    }
}
