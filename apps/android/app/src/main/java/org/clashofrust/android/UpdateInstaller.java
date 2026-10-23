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
        int flags=Build.VERSION.SDK_INT>=28?PackageManager.GET_SIGNING_CERTIFICATES:PackageManager.GET_SIGNATURES;
        PackageManager manager=activity.getPackageManager();
        PackageInfo archive=manager.getPackageArchiveInfo(UpdateProvider.apk(activity).toString(),flags);
        PackageInfo installed=manager.getPackageInfo(activity.getPackageName(),flags);
        if (archive==null || !activity.getPackageName().equals(archive.packageName)) throw new IOException("安装包不是 Clash of Rust");
        long incoming=Build.VERSION.SDK_INT>=28?archive.getLongVersionCode():archive.versionCode;
        long current=Build.VERSION.SDK_INT>=28?installed.getLongVersionCode():installed.versionCode;
        if (incoming<current) throw new IOException("Android 不允许覆盖安装较低版本的安装包。如需返回较旧正式版，请先备份配置，再手动卸载并安装正式版；卸载会删除应用数据。");
        if (incoming==current && java.util.Objects.equals(archive.versionName,installed.versionName))
            throw new IOException("已经安装此版本，请重新检查更新");
        Signature[] target=Build.VERSION.SDK_INT>=28?(archive.signingInfo==null?null:archive.signingInfo.getApkContentsSigners()):archive.signatures;
        Signature[] existing=Build.VERSION.SDK_INT>=28?(installed.signingInfo==null?null:installed.signingInfo.getApkContentsSigners()):installed.signatures;
        if (target==null || existing==null || target.length!=existing.length || !Arrays.asList(target).containsAll(Arrays.asList(existing)))
            throw new IOException("更新签名与当前应用不同，已取消安装");
        try (ZipFile zip=new ZipFile(UpdateProvider.apk(activity))) {
            if (Arrays.stream(Build.SUPPORTED_ABIS).noneMatch(abi->zip.getEntry("lib/"+abi+"/libclash_mobile.so")!=null))
                throw new IOException("安装包架构与设备不兼容");
        }
        return archive;
    }
    static void request(MainActivity activity) throws Exception {
        PackageInfo archive=validate(activity);
        activity.runOnUiThread(()->new AlertDialog.Builder(activity).setTitle("安装 "+archive.versionName+"？")
            .setMessage("将打开 Android 安装界面确认更新。安装时 VPN 会短暂断开。")
            .setNegativeButton("取消",null).setPositiveButton("继续安装",(dialog,which)->{
                if (!activity.getPackageManager().canRequestPackageInstalls()) {
                    activity.waitingForInstallPermission=true;
                    activity.startActivity(new Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES,Uri.parse("package:"+activity.getPackageName())));
                    MainActivity.emit("notice","请允许此应用安装更新，然后返回继续");
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
        } catch (Exception error) { MainActivity.emit("error","无法启动更新安装："+error); }
    }
}
