package org.clashofrust.android;

import android.content.Context;
import android.content.res.Configuration;
import android.content.res.Resources;
import android.os.LocaleList;
import java.io.File;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.attribute.BasicFileAttributes;
import java.nio.file.attribute.FileTime;
import java.util.Locale;
import java.util.Objects;
import org.json.JSONObject;

final class AppLanguage {
    private static File cachedFile;
    private static Object cachedKey;
    private static FileTime cachedModified;
    private static long cachedSize = -1;
    private static String cachedPreference = "system";

    private AppLanguage() {}

    static String resolve(String preference, LocaleList languages) {
        if ("en".equals(preference) || "zh-CN".equals(preference)) return preference;
        for (int index = 0; index < languages.size(); index++) {
            Locale locale = languages.get(index);
            if ("en".equalsIgnoreCase(locale.getLanguage())) return "en";
            if ("zh".equalsIgnoreCase(locale.getLanguage())) {
                String script = locale.getScript();
                String country = locale.getCountry();
                if ("Hant".equalsIgnoreCase(script)) continue;
                if (script.isEmpty() && ("TW".equalsIgnoreCase(country)
                    || "HK".equalsIgnoreCase(country) || "MO".equalsIgnoreCase(country))) continue;
                return "zh-CN";
            }
        }
        return "en";
    }

    private static synchronized String preference(Context context) {
        File file = new File(context.getFilesDir(), "client/settings.json");
        try {
            BasicFileAttributes attributes = Files.readAttributes(file.toPath(), BasicFileAttributes.class);
            if (!attributes.isRegularFile() || attributes.size() > 10 * 1024 * 1024) return "system";
            if (file.equals(cachedFile) && Objects.equals(attributes.fileKey(), cachedKey)
                && attributes.lastModifiedTime().equals(cachedModified) && attributes.size() == cachedSize)
                return cachedPreference;
            JSONObject settings = new JSONObject(new String(Files.readAllBytes(file.toPath()), StandardCharsets.UTF_8));
            String value = settings.optString("language", "system");
            cachedFile = file;
            cachedKey = attributes.fileKey();
            cachedModified = attributes.lastModifiedTime();
            cachedSize = attributes.size();
            cachedPreference = value;
            return value;
        } catch (Exception ignored) {
            return "system";
        }
    }

    static Context localized(Context context, String tag) {
        Configuration configuration = new Configuration(context.getResources().getConfiguration());
        configuration.setLocale("zh-CN".equals(tag) ? Locale.SIMPLIFIED_CHINESE : Locale.ENGLISH);
        return context.createConfigurationContext(configuration);
    }

    static Context current(Context context) {
        return localized(context, resolve(preference(context), Resources.getSystem().getConfiguration().getLocales()));
    }
}
