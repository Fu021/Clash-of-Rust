package org.clashofrust.android;

import android.app.AlertDialog;
import android.content.Context;
import android.content.Intent;
import android.content.SharedPreferences;
import android.content.pm.PackageInfo;
import android.content.pm.PackageManager;
import android.content.pm.Signature;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.provider.Settings;
import android.system.Os;
import android.system.StructStat;
import android.util.Base64;
import java.io.File;
import java.io.FileInputStream;
import java.io.IOException;
import java.io.InputStream;
import java.security.MessageDigest;
import java.security.NoSuchAlgorithmException;
import java.util.Arrays;
import java.util.zip.ZipFile;

/** Hash checks happen in Rust; Android also verifies identity, signer and version. */
final class UpdateInstaller {
    private static Verification verification;

    static final class PermissionRequest {
        private static final String SAVED_CODE = "org.clashofrust.android.update-permission";
        private static final String LAST_CODE = "last-code";
        private static final String SOURCE = "source";
        private static final String READY = "ready";
        private static final int FIRST_CODE = 0x1000, LAST_ALLOWED_CODE = 0x3fff;
        private final SharedPreferences store;
        private int code;

        PermissionRequest(SharedPreferences store, Bundle state) {
            this.store = store;
            if (state != null) {
                int saved = state.getInt(SAVED_CODE);
                if (handles(saved) && saved == store.getInt(LAST_CODE, 0) && store.contains(SOURCE)) {
                    code = saved;
                }
            }
        }

        static boolean handles(int code) {
            return code >= FIRST_CODE && code <= LAST_ALLOWED_CODE;
        }

        boolean pending() { return matches(code); }

        int begin(String source, boolean ready) throws IOException {
            if (pending()) return 0;
            int next = store.getInt(LAST_CODE, FIRST_CODE - 1) + 1;
            if (!handles(next)) throw new IOException("Update permission request codes exhausted");
            if (!store.edit().putInt(LAST_CODE, next).putString(SOURCE, source).putBoolean(READY, ready).commit()) {
                throw new IOException("Could not save the update permission request");
            }
            code = next;
            return code;
        }

        void save(Bundle state) { state.putInt(SAVED_CODE, code); }

        private boolean matches(int expectedCode) {
            return code != 0 && code == expectedCode && code == store.getInt(LAST_CODE, 0)
                && store.contains(SOURCE);
        }

        void returned(int returnedCode) throws IOException {
            if (matches(returnedCode) && !store.getBoolean(READY, false)
                    && !store.edit().putBoolean(READY, true).commit()) {
                IOException error = new IOException("Could not save the returned update permission request");
                try { finish(returnedCode); }
                catch (IOException cleanup) { error.addSuppressed(cleanup); }
                throw error;
            }
        }

        int readyCode() { return matches(code) && store.getBoolean(READY, false) ? code : 0; }

        String source() { return matches(code) ? store.getString(SOURCE, null) : null; }

        boolean finish(int expectedCode) throws IOException {
            if (!matches(expectedCode)) return false;
            code = 0;
            if (!store.edit().remove(SOURCE).remove(READY).commit()) {
                throw new IOException("Could not clear the update permission request");
            }
            return true;
        }
    }

    private static final class Verification {
        final int code;
        final String source;
        boolean complete;
        Exception error;

        Verification(int code, String source) {
            this.code = code;
            this.source = source;
        }
    }

    @SuppressWarnings("deprecation")
    static void pruneInstalled(MainActivity activity) {
        java.io.File file=UpdateProvider.apk(activity);
        if (!file.isFile()) return;
        try {
            PackageManager manager=activity.getPackageManager();
            PackageInfo archive=manager.getPackageArchiveInfo(file.toString(),0);
            PackageInfo current=manager.getPackageInfo(activity.getPackageName(),0);
            long installed=Build.VERSION.SDK_INT>=28?current.getLongVersionCode():current.versionCode;
            long cached=archive==null?0:Build.VERSION.SDK_INT>=28?archive.getLongVersionCode():archive.versionCode;
            if (cached<=installed && !file.delete()) android.util.Log.w("ClashOfRust","Could not clear installed update cache");
        } catch (Exception error) {android.util.Log.w("ClashOfRust","Read update cache",error);}
    }
    @SuppressWarnings("deprecation")
    private static PackageInfo validate(Context context, File apk) throws Exception {
        Context localized=AppLanguage.current(context);
        int flags=Build.VERSION.SDK_INT>=28?PackageManager.GET_SIGNING_CERTIFICATES:PackageManager.GET_SIGNATURES;
        PackageManager manager=context.getPackageManager();
        PackageInfo archive=manager.getPackageArchiveInfo(apk.toString(),flags);
        PackageInfo installed=manager.getPackageInfo(context.getPackageName(),flags);
        if (archive==null || !context.getPackageName().equals(archive.packageName)) throw new IOException(localized.getString(R.string.update_invalid_package));
        long incoming=Build.VERSION.SDK_INT>=28?archive.getLongVersionCode():archive.versionCode;
        long current=Build.VERSION.SDK_INT>=28?installed.getLongVersionCode():installed.versionCode;
        if (incoming<current) throw new IOException(localized.getString(R.string.update_downgrade));
        if (incoming==current && java.util.Objects.equals(archive.versionName,installed.versionName))
            throw new IOException(localized.getString(R.string.update_same_version));
        Signature[] target=Build.VERSION.SDK_INT>=28?(archive.signingInfo==null?null:archive.signingInfo.getApkContentsSigners()):archive.signatures;
        Signature[] existing=Build.VERSION.SDK_INT>=28?(installed.signingInfo==null?null:installed.signingInfo.getApkContentsSigners()):installed.signatures;
        if (target==null || existing==null || target.length!=existing.length || !Arrays.asList(target).containsAll(Arrays.asList(existing)))
            throw new IOException(localized.getString(R.string.update_wrong_signature));
        try (ZipFile zip=new ZipFile(apk)) {
            if (Arrays.stream(Build.SUPPORTED_ABIS).noneMatch(abi->zip.getEntry("lib/"+abi+"/libclash_mobile.so")!=null))
                throw new IOException(localized.getString(R.string.update_wrong_architecture));
        }
        return archive;
    }

    private record CheckedApk(String version, String source) {}

    private static CheckedApk inspect(Context context) throws Exception {
        File apk = UpdateProvider.apk(context);
        try (FileInputStream input = new FileInputStream(apk)) {
            StructStat opened = Os.fstat(input.getFD());
            PackageInfo archive = validate(context, apk);
            StructStat current = Os.stat(apk.toString());
            if (opened.st_dev != current.st_dev || opened.st_ino != current.st_ino) {
                throw new IOException("Update APK changed during verification");
            }
            return new CheckedApk(archive.versionName, fingerprint(input));
        }
    }

    static void request(MainActivity activity) throws Exception {
        CheckedApk checked = inspect(activity);
        Context localized=AppLanguage.current(activity);
        activity.runOnUiThread(() -> {
            if (activity.isFinishing() || activity.isDestroyed() || activity.installPermission.pending()) return;
            new AlertDialog.Builder(activity).setTitle(localized.getString(R.string.update_install_title,checked.version()))
                .setMessage(localized.getString(R.string.update_install_message))
                .setNegativeButton(localized.getString(R.string.cancel),null)
                .setPositiveButton(localized.getString(R.string.update_continue),
                    (dialog, which) -> confirm(activity, checked.source()))
                .show();
        });
    }

    @SuppressWarnings("deprecation")
    private static void confirm(MainActivity activity, String source) {
        if (activity.installPermission.pending()) return;
        int code = 0;
        try {
            boolean authorized = activity.getPackageManager().canRequestPackageInstalls();
            code = activity.installPermission.begin(source, authorized);
            if (code == 0) return;
            if (authorized) {
                resume(activity);
                return;
            }
            activity.startActivityForResult(new Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES,
                Uri.fromParts("package", activity.getPackageName(), null)), code);
        } catch (Exception error) {
            try { activity.installPermission.finish(code); }
            catch (IOException cleanup) { error.addSuppressed(cleanup); }
            reportFailure(activity, error);
            return;
        }
        MainActivity.emit("notice", AppLanguage.current(activity).getString(R.string.update_allow_install));
    }

    static void permissionReturned(MainActivity activity, int code) {
        try {
            activity.installPermission.returned(code);
        } catch (Exception error) {
            reportFailure(activity, error);
        }
    }

    static String fingerprint(InputStream input) throws IOException, NoSuchAlgorithmException {
        MessageDigest digest = MessageDigest.getInstance("SHA-256");
        byte[] buffer = new byte[64 * 1024];
        int length;
        while ((length = input.read(buffer)) != -1) digest.update(buffer, 0, length);
        return Base64.encodeToString(digest.digest(), Base64.URL_SAFE | Base64.NO_WRAP | Base64.NO_PADDING);
    }

    static void resume(MainActivity activity) {
        if (activity == null || MainActivity.resumedActivity() != activity
                || activity.isFinishing() || activity.isDestroyed()) return;
        PermissionRequest request = activity.installPermission;
        int code = request.readyCode();
        if (code == 0) return;
        try {
            if (!activity.getPackageManager().canRequestPackageInstalls()) {
                if (request.finish(code)) {
                    MainActivity.emit("notice", AppLanguage.current(activity).getString(R.string.update_permission_missing));
                }
                return;
            }
            if (verification != null && !verification.complete) return;
            if (verification == null || verification.code != code) {
                Verification check = new Verification(code, request.source());
                verify(activity.getApplicationContext(), check);
                verification = check;
                return;
            }
            Verification checked = verification;
            verification = null;
            if (!request.finish(code)) return;
            if (checked.error != null) throw checked.error;
            activity.startActivity(new Intent(Intent.ACTION_VIEW)
                .setDataAndType(UpdateProvider.uri(activity, checked.source), "application/vnd.android.package-archive")
                .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION));
        } catch (Exception error) {
            try { request.finish(code); }
            catch (IOException cleanup) { error.addSuppressed(cleanup); }
            reportFailure(activity, error);
        }
    }

    private static void verify(Context context, Verification check) {
        Handler main = new Handler(Looper.getMainLooper());
        new Thread(() -> {
            Exception failure = null;
            try {
                if (!check.source.equals(inspect(context).source())) throw new IOException("APK SHA-256 mismatch");
            } catch (Exception error) {
                failure = error;
            }
            Exception result = failure;
            main.post(() -> {
                check.error = result;
                check.complete = true;
                resume(MainActivity.resumedActivity());
            });
        }, "apk-update-verify").start();
    }

    private static void reportFailure(MainActivity activity, Exception error) {
        MainActivity.emit("error", AppLanguage.current(activity).getString(R.string.update_launch_failed, error));
    }
}
