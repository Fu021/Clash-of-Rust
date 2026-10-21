package org.clashofrust.android;

import android.app.UiModeManager;
import android.content.Context;
import android.content.res.Configuration;
import android.content.res.Resources;
import android.os.Build;
import java.io.File;
import java.nio.file.Files;
import org.json.JSONObject;

/** Saved appearance drives the starting window, system bars and first Slint frame. */
final class Appearance {
    private static int appliedMode = -1;
    private static JSONObject settings(Context context) {
        File file = new File(context.getFilesDir(), "client/settings.json");
        if (file.isFile() && file.length() <= 1024 * 1024) {
            try {
                return new JSONObject(new String(Files.readAllBytes(file.toPath()),
                    java.nio.charset.StandardCharsets.UTF_8));
            } catch (Exception error) {
                android.util.Log.w("ClashOfRust", "Could not read startup appearance", error);
            }
        }
        return new JSONObject();
    }
    private static int mode(JSONObject saved) {
        String value = saved.optString("theme_mode");
        return value.equals("dark") ? 0 : value.equals("light") ? 1 : 2;
    }
    static int mode(Context context) { return mode(settings(context)); }
    static JSONObject startup(Context context) throws org.json.JSONException {
        JSONObject saved = settings(context);
        return new JSONObject().put("mode", mode(saved))
            .put("dark", systemDark(context))
            // Rust decodes its own enum; no second palette/order in Java.
            .put("accent_color", saved.optString("accent_color", "teal"));
    }
    static boolean systemDark(Context context) {
        // The Activity's resources can reflect our per-app override on API 31+.
        // Following the system must read the system configuration instead.
        int mode = context.getSystemService(UiModeManager.class).getNightMode();
        if (mode == UiModeManager.MODE_NIGHT_YES) return true;
        if (mode == UiModeManager.MODE_NIGHT_NO) return false;
        return (Resources.getSystem().getConfiguration().uiMode & Configuration.UI_MODE_NIGHT_MASK)
            == Configuration.UI_MODE_NIGHT_YES;
    }
    static boolean dark(Context context, int mode) { return mode == 0 || (mode == 2 && systemDark(context)); }
    static void persistSystemMode(Context context, int mode) {
        if (Build.VERSION.SDK_INT >= 31 && mode != appliedMode) {
            appliedMode = mode;
            context.getSystemService(UiModeManager.class).setApplicationNightMode(
                mode == 0 ? UiModeManager.MODE_NIGHT_YES : mode == 1 ? UiModeManager.MODE_NIGHT_NO
                    // AUTO clears the per-package override (UI_MODE_NIGHT_UNDEFINED)
                    // in UiModeManagerService; the global system mode then applies.
                    : UiModeManager.MODE_NIGHT_AUTO);
        }
    }
}
