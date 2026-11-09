package org.clashofrust.android;

import android.content.Context;
import android.content.Intent;
import android.content.pm.ApplicationInfo;
import android.content.pm.PackageManager;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.HashSet;
import java.util.Set;
import org.json.JSONArray;
import org.json.JSONObject;

final class ApplicationCatalog {
    static boolean visibleWithoutComponents(ApplicationInfo application, boolean launchable, boolean selected) {
        return launchable || selected || (application.flags & ApplicationInfo.FLAG_SYSTEM) == 0;
    }

    static JSONObject read(Context context) throws Exception {
        PackageManager manager = context.getPackageManager();
        ApplicationRouting saved = ApplicationRouting.load(context);
        Set<String> launchable = new HashSet<>();
        Intent launcher = new Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_LAUNCHER);
        for (android.content.pm.ResolveInfo activity : manager.queryIntentActivities(launcher, 0))
            launchable.add(activity.activityInfo.packageName);
        ArrayList<JSONObject> entries = new ArrayList<>();
        for (ApplicationInfo application : manager.getInstalledApplications(0)) {
            String label;
            try { label = application.loadLabel(manager).toString(); }
            catch (RuntimeException error) { label = application.packageName; }
            entries.add(new JSONObject().put("name", label).put("package", application.packageName)
                .put("system", (application.flags & ApplicationInfo.FLAG_SYSTEM) != 0)
                .put("launchable", launchable.contains(application.packageName)));
        }
        entries.sort(Comparator.comparing((JSONObject item) -> item.optString("name"), String.CASE_INSENSITIVE_ORDER)
            .thenComparing(item -> item.optString("package")));
        ArrayList<String> selected = new ArrayList<>(saved.packages);
        java.util.Collections.sort(selected);
        return new JSONObject().put("applications", new JSONArray(entries))
            .put("mode", saved.mode).put("selected", new JSONArray(selected));
    }

    static JSONObject save(Context context, JSONObject data) throws Exception {
        if (CoreBridge.protectedMode(context))
            throw new IllegalStateException("请先关闭 Claude 保护，再修改应用分流");
        Set<String> selected = new HashSet<>();
        JSONArray packages = data.getJSONArray("selected");
        for (int index = 0; index < packages.length(); index++) selected.add(packages.getString(index));
        ApplicationRouting routing = new ApplicationRouting(data.getString("mode"), selected);
        routing.save(context);
        return new JSONObject().put("summary", routing.summary()).put("connected", TunnelService.connected());
    }
}
