package org.clashofrust.android;

import android.content.Context;
import android.content.SharedPreferences;
import android.content.pm.PackageManager;
import android.net.VpnService;
import java.io.IOException;
import java.util.Collections;
import java.util.HashSet;
import java.util.Set;

final class ApplicationRouting {
    static final String ALL = "all", ONLY = "only", EXCLUDE = "exclude";
    final String mode;
    final Set<String> packages;

    ApplicationRouting(String mode, Set<String> packages) {
        if (!ALL.equals(mode) && !ONLY.equals(mode) && !EXCLUDE.equals(mode))
            throw new IllegalArgumentException("无法识别应用分流模式");
        this.mode = mode;
        this.packages = Collections.unmodifiableSet(new HashSet<>(packages));
    }

    static ApplicationRouting load(SharedPreferences preferences) {
        Set<String> legacy = preferences.getStringSet("excluded", Collections.emptySet());
        String mode = preferences.getString("application_mode", legacy.isEmpty() ? ALL : EXCLUDE);
        Set<String> packages = preferences.getStringSet("selected_apps", legacy);
        return new ApplicationRouting(mode, packages);
    }

    static ApplicationRouting load(Context context) {
        try {
            return load(context.getSharedPreferences("vpn", Context.MODE_PRIVATE));
        } catch (IllegalArgumentException error) {
            throw new IllegalArgumentException(AppLanguage.current(context).getString(R.string.applications_invalid_mode), error);
        }
    }

    static ApplicationRouting create(Context context, String mode, Set<String> packages) {
        try {
            return new ApplicationRouting(mode, packages);
        } catch (IllegalArgumentException error) {
            throw new IllegalArgumentException(AppLanguage.current(context).getString(R.string.applications_invalid_mode), error);
        }
    }

    String summary(Context context) {
        Context localized = AppLanguage.current(context);
        if (ALL.equals(mode)) return localized.getString(R.string.applications_all);
        return localized.getString(ONLY.equals(mode) ? R.string.applications_only : R.string.applications_excluded, packages.size());
    }

    void persist(Context context, SharedPreferences preferences) throws IOException {
        if (!preferences.edit().putString("application_mode", mode)
                .putStringSet("selected_apps", packages)
                .putStringSet("excluded", EXCLUDE.equals(mode) ? packages : Collections.emptySet()).commit())
            throw new IOException(AppLanguage.current(context).getString(R.string.applications_save_failed));
    }

    void save(Context context) throws Exception {
        if (CoreBridge.protectedMode(context))
            throw new IllegalStateException(AppLanguage.current(context).getString(R.string.applications_guard_routing));
        if (ONLY.equals(mode) && packages.isEmpty())
            throw new IllegalArgumentException(AppLanguage.current(context).getString(R.string.applications_select_one));
        persist(context, context.getSharedPreferences("vpn", Context.MODE_PRIVATE));
    }

    static ApplicationRouting effective(ApplicationRouting saved, boolean protectedMode) {
        return protectedMode ? new ApplicationRouting(ALL, Collections.emptySet()) : saved;
    }

    static void apply(Context context, VpnService.Builder builder, boolean protectedMode) throws Exception {
        ApplicationRouting saved = load(context);
        ApplicationRouting routing = effective(saved, protectedMode);
        if (ALL.equals(routing.mode)) return;
        Set<String> valid = new HashSet<>();
        for (String name : routing.packages) {
            try {
                context.getPackageManager().getApplicationInfo(name, 0);
                if (ONLY.equals(routing.mode)) builder.addAllowedApplication(name);
                else builder.addDisallowedApplication(name);
                valid.add(name);
            } catch (PackageManager.NameNotFoundException removed) {
                android.util.Log.i("ClashOfRust", "Removed an uninstalled application from VPN routing");
            }
        }
        if (!valid.equals(saved.packages))
            new ApplicationRouting(saved.mode, valid).persist(context, context.getSharedPreferences("vpn", Context.MODE_PRIVATE));
        if (ONLY.equals(routing.mode) && valid.isEmpty())
            throw new IOException(AppLanguage.current(context).getString(R.string.applications_removed));
    }
}
