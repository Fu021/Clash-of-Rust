package org.clashofrust.android;

import android.app.*;
import android.content.*;
import android.content.pm.PackageManager;
import android.net.Uri;
import android.net.VpnService;
import android.os.*;
import android.provider.Settings;
import java.io.*;
import java.nio.charset.StandardCharsets;
import java.util.*;
import java.util.concurrent.Executors;
import org.json.JSONObject;

public final class MainActivity extends NativeActivity {
    static { System.loadLibrary("clash_mobile"); }
    private static final int VPN = 10, OPEN = 11, SAVE = 12, NOTIFICATIONS = 13;
    private boolean vpnPrompt;
    private String exportContent;
    private static native void nativeEvent(String value);

    static void emit(String event, Object value) {
        try { nativeEvent(new JSONObject().put("event", event).put("value", value).toString()); }
        catch (Exception | UnsatisfiedLinkError error) {
            android.util.Log.e("ClashOfRust", "Native event delivery failed", error);
        }
    }
    @Override public void onCreate(Bundle state) {
        super.onCreate(state);
        // Let Slint handle native insets; do not install a second padded content view.
        getWindow().setSoftInputMode(android.view.WindowManager.LayoutParams.SOFT_INPUT_ADJUST_RESIZE);
    }
    @Override protected void onResume() { super.onResume(); emit("visible", true); }
    @Override protected void onPause() { emit("visible", false); super.onPause(); }

    /** Invoked by the Rust worker, never performs blocking work on the UI thread. */
    public String request(String input) {
        try {
            JSONObject data = new JSONObject(input);
            String op = data.getString("op");
            switch (op) {
                case "initialize": CoreBridge.start(this, data.getString("config"), data.getBoolean("ipv6")); break;
                case "reload": CoreBridge.reload(data.getString("config")); CoreBridge.setIpv6(data.getBoolean("ipv6")); break;
                case "state": return CoreBridge.state()
                    .put("dark", (getResources().getConfiguration().uiMode & android.content.res.Configuration.UI_MODE_NIGHT_MASK)
                        == android.content.res.Configuration.UI_MODE_NIGHT_YES)
                    .put("notification_speeds",getSharedPreferences("appearance",MODE_PRIVATE).getBoolean("notification-speeds",true))
                    .put("excluded_apps",getSharedPreferences("vpn",MODE_PRIVATE).getStringSet("excluded",Collections.emptySet()).size()).toString();
                case "connect": runOnUiThread(this::prepareVpn); break;
                case "disconnect": TunnelService.stop(this); break;
                case "traffic": TunnelService.updateTraffic(this, data.optString("down"), data.optString("up")); break;
                case "copy": {
                    String text = data.getString("text");
                    runOnUiThread(() -> ((android.content.ClipboardManager)getSystemService(CLIPBOARD_SERVICE))
                        .setPrimaryClip(ClipData.newPlainText("Clash of Rust", text)));
                    break;
                }
                case "pick-file": runOnUiThread(() -> startActivityForResult(
                    new Intent(Intent.ACTION_OPEN_DOCUMENT).setType("*/*")
                        .addCategory(Intent.CATEGORY_OPENABLE), OPEN)); break;
                case "export": {
                    String name = data.getString("name"), content = data.getString("content");
                    runOnUiThread(() -> {
                        if (exportContent != null) { emit("error", "已有导出操作正在进行"); return; }
                        exportContent = content;
                        startActivityForResult(new Intent(Intent.ACTION_CREATE_DOCUMENT)
                            .setType("text/plain").addCategory(Intent.CATEGORY_OPENABLE)
                            .putExtra(Intent.EXTRA_TITLE, name), SAVE);
                    });
                    break;
                }
                case "vpn-settings": runOnUiThread(() -> startActivity(new Intent(Settings.ACTION_VPN_SETTINGS))); break;
                case "battery-settings": runOnUiThread(() -> startActivity(new Intent(Settings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS))); break;
                case "application-routing": runOnUiThread(this::applicationRouting); break;
                case "notification-speeds":
                    getSharedPreferences("appearance", MODE_PRIVATE).edit().putBoolean("notification-speeds", data.getBoolean("value")).apply(); break;
                case "notify": runOnUiThread(this::notifyMeow); break;
                case "open-url": {
                    Uri url = Uri.parse(data.getString("url"));
                    if (!"https".equals(url.getScheme()) && !"http".equals(url.getScheme()))
                        throw new IOException("仅支持 HTTP/HTTPS 地址");
                    runOnUiThread(() -> startActivity(new Intent(Intent.ACTION_VIEW, url)));
                    break;
                }
                default: throw new IOException("未知 Android 操作：" + op);
            }
            return new JSONObject().put("ok", true).toString();
        } catch (Exception error) {
            try { return new JSONObject().put("error", error.toString()).toString(); }
            catch (Exception impossible) { return "{\"error\":\"Android request failed\"}"; }
        }
    }
    private void prepareVpn() {
        if (vpnPrompt || TunnelService.connected()) return;
        if (!CoreBridge.ready()) { emit("error", "请先启动内核"); return; }
        Intent permission = VpnService.prepare(this);
        if (permission == null) startVpn(); else {
            vpnPrompt = true;
            startActivityForResult(permission, VPN);
        }
    }
    private void startVpn() {
        startForegroundService(new Intent(this, TunnelService.class));
    }
    @Override protected void onActivityResult(int request, int result, Intent data) {
        super.onActivityResult(request, result, data);
        if (request == VPN) {
            vpnPrompt = false;
            if (result == RESULT_OK) startVpn(); else emit("error", "VPN 授权已取消");
            return;
        }
        if (request != OPEN && request != SAVE) return;
        String content = exportContent;
        if (request == SAVE) exportContent = null;
        if (result != RESULT_OK || data == null || data.getData() == null) return;
        Uri uri = data.getData();
        var worker = Executors.newSingleThreadExecutor();
        worker.execute(() -> {
            try {
                if (request == SAVE) {
                    if (content == null) throw new IOException("导出内容已失效");
                    try (OutputStream out = getContentResolver().openOutputStream(uri, "wt")) {
                        if (out == null) throw new IOException("无法写入所选文件");
                        out.write(content.getBytes(StandardCharsets.UTF_8));
                    }
                    emit("notice", "文件已导出");
                } else {
                    try (InputStream in = getContentResolver().openInputStream(uri)) {
                        if (in == null) throw new IOException("无法读取所选文件");
                        ByteArrayOutputStream out = new ByteArrayOutputStream();
                        byte[] buffer = new byte[8192];
                        int count;
                        while ((count = in.read(buffer)) != -1) {
                            if (out.size() + count > 10 * 1024 * 1024) throw new IOException("订阅超过 10 MiB");
                            out.write(buffer, 0, count);
                        }
                        JSONObject file = new JSONObject().put("name", "导入配置")
                            .put("content", out.toString(StandardCharsets.UTF_8.name()));
                        emit("file", file);
                    }
                }
            } catch (Exception error) { emit("error", error.toString()); }
        });
        worker.shutdown();
    }
    private void applicationRouting() {
        if (TunnelService.connected()) {
            new AlertDialog.Builder(this).setMessage("请先断开 VPN，再修改应用分流。")
                .setPositiveButton("知道了", null).show(); return;
        }
        List<android.content.pm.ResolveInfo> apps = getPackageManager().queryIntentActivities(
            new Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_LAUNCHER), 0);
        apps.sort(Comparator.comparing(item -> item.loadLabel(getPackageManager()).toString()));
        Set<String> exclusions = new HashSet<>(getSharedPreferences("vpn", MODE_PRIVATE)
            .getStringSet("excluded", Collections.emptySet()));
        String[] labels = new String[apps.size()];
        boolean[] checked = new boolean[apps.size()];
        for (int i=0; i<apps.size(); i++) {
            labels[i] = apps.get(i).loadLabel(getPackageManager()).toString();
            checked[i] = exclusions.contains(apps.get(i).activityInfo.packageName);
        }
        new AlertDialog.Builder(this).setTitle("选择不经过 VPN 的应用")
            .setMultiChoiceItems(labels, checked, (dialog, which, selected) -> {
                String id = apps.get(which).activityInfo.packageName;
                if (selected) exclusions.add(id); else exclusions.remove(id);
            }).setNegativeButton("取消", null).setPositiveButton("保存", (dialog, which) -> {
                getSharedPreferences("vpn", MODE_PRIVATE).edit().putStringSet("excluded", exclusions).apply();
                emit("notice", "应用分流已保存，下次连接时生效");
            }).show();
    }
    private void notifyMeow() {
        if (Build.VERSION.SDK_INT >= 33 && checkSelfPermission(android.Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
            requestPermissions(new String[]{android.Manifest.permission.POST_NOTIFICATIONS}, NOTIFICATIONS);
            return;
        }
        NotificationManager manager = getSystemService(NotificationManager.class);
        manager.createNotificationChannel(new NotificationChannel("messages", "消息", NotificationManager.IMPORTANCE_DEFAULT));
        manager.notify(2, new Notification.Builder(this, "messages").setSmallIcon(R.drawable.ic_app)
            .setContentTitle("Clash of Rust").setContentText("喵!").setAutoCancel(true).build());
    }
    @Override public void onRequestPermissionsResult(int request, String[] permissions, int[] results) {
        super.onRequestPermissionsResult(request, permissions, results);
        if (request == NOTIFICATIONS && results.length > 0 && results[0] == PackageManager.PERMISSION_GRANTED) notifyMeow();
    }
}
