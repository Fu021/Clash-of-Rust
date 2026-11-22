package org.clashofrust.android;

import android.app.AlertDialog;
import android.content.Intent;
import android.content.pm.PackageInfo;
import android.content.pm.PackageManager;
import android.content.pm.Signature;
import android.net.Uri;
import android.os.Build;
import android.provider.Settings;
import java.io.IOException;
import java.util.Arrays;
import java.util.zip.ZipFile;

/** Hash checks happen in Rust; Android also verifies identity, signer and version. */
final class UpdateInstaller {
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
    static PackageInfo validate(MainActivity activity) throws Exception {
        android.content.Context localized=AppLanguage.current(activity);
        int flags=Build.VERSION.SDK_INT>=28?PackageManager.GET_SIGNING_CERTIFICATES:PackageManager.GET_SIGNATURES;
        PackageManager manager=activity.getPackageManager();
        PackageInfo archive=manager.getPackageArchiveInfo(UpdateProvider.apk(activity).toString(),flags);
        PackageInfo installed=manager.getPackageInfo(activity.getPackageName(),flags);
        if (archive==null || !activity.getPackageName().equals(archive.packageName)) throw new IOException(localized.getString(R.string.update_invalid_package));
        long incoming=Build.VERSION.SDK_INT>=28?archive.getLongVersionCode():archive.versionCode;
        long current=Build.VERSION.SDK_INT>=28?installed.getLongVersionCode():installed.versionCode;
        if (incoming<current) throw new IOException(localized.getString(R.string.update_downgrade));
        if (incoming==current && java.util.Objects.equals(archive.versionName,installed.versionName))
            throw new IOException(localized.getString(R.string.update_same_version));
        Signature[] target=Build.VERSION.SDK_INT>=28?(archive.signingInfo==null?null:archive.signingInfo.getApkContentsSigners()):archive.signatures;
        Signature[] existing=Build.VERSION.SDK_INT>=28?(installed.signingInfo==null?null:installed.signingInfo.getApkContentsSigners()):installed.signatures;
        if (target==null || existing==null || target.length!=existing.length || !Arrays.asList(target).containsAll(Arrays.asList(existing)))
            throw new IOException(localized.getString(R.string.update_wrong_signature));
        try (ZipFile zip=new ZipFile(UpdateProvider.apk(activity))) {
            if (Arrays.stream(Build.SUPPORTED_ABIS).noneMatch(abi->zip.getEntry("lib/"+abi+"/libclash_mobile.so")!=null))
                throw new IOException(localized.getString(R.string.update_wrong_architecture));
        }
        return archive;
    }
    static void request(MainActivity activity) throws Exception {
        PackageInfo archive=validate(activity);
        android.content.Context localized=AppLanguage.current(activity);
        activity.runOnUiThread(()->new AlertDialog.Builder(activity).setTitle(localized.getString(R.string.update_install_title,archive.versionName))
            .setMessage(localized.getString(R.string.update_install_message))
            .setNegativeButton(localized.getString(R.string.cancel),null).setPositiveButton(localized.getString(R.string.update_continue),(dialog,which)->{
                if (!activity.getPackageManager().canRequestPackageInstalls()) {
                    activity.waitingForInstallPermission=true;
                    activity.startActivity(new Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES,Uri.parse("package:"+activity.getPackageName())));
                    MainActivity.emit("notice",localized.getString(R.string.update_allow_install));
                } else launch(activity);
            }).show());
    }
    @SuppressWarnings("deprecation")
    static void launch(MainActivity activity) {
        try {
            // Permission settings can take time: recheck the archive before granting it.
            validate(activity);
            activity.startActivity(new Intent(Intent.ACTION_VIEW).setDataAndType(UpdateProvider.uri(activity),"application/vnd.android.package-archive")
                .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION));
        } catch (Exception error) { MainActivity.emit("error",AppLanguage.current(activity).getString(R.string.update_launch_failed,error)); }
    }
}
