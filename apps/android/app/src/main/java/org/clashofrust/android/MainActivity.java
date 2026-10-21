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
    private boolean vpnReconfigure;
    private boolean vpnAuthorizationOnly;
    private long vpnAuthorizationRequest;
    private static volatile String vpnAuthorizationState = "{}";
    static JSONObject authorizationState() throws Exception { return new JSONObject(vpnAuthorizationState); }
    private boolean tileConnect;
    boolean waitingForInstallPermission;
    private String exportContent;
    private String notificationTitle, notificationMessage;
    private static native void nativeEvent(String value);
    private static native String nativeDiagnostics();
    static JSONObject diagnostics() throws Exception { return new JSONObject(nativeDiagnostics()); }

    static void emit(String event, Object value) {
        try { nativeEvent(new JSONObject().put("event", event).put("value", value).toString()); }
        catch (Exception | UnsatisfiedLinkError error) {
            android.util.Log.e("ClashOfRust", "Native event delivery failed", error);
        }
    }
    @Override public void onCreate(Bundle state) {
        boolean dark = Appearance.dark(this, Appearance.mode(this));
        setTheme(dark ? R.style.ClientDark : R.style.ClientLight);
        tileConnect=VpnTileService.CONNECT.equals(getIntent().getAction());
        super.onCreate(state);
        UpdateInstaller.pruneInstalled(this);
        // Let Slint handle native insets; do not install a second padded content view.
        getWindow().setSoftInputMode(android.view.WindowManager.LayoutParams.SOFT_INPUT_ADJUST_RESIZE);
        configureSystemBars(dark);
        if (Build.VERSION.SDK_INT >= 33) getOnBackInvokedDispatcher().registerOnBackInvokedCallback(
            android.window.OnBackInvokedDispatcher.PRIORITY_DEFAULT, () -> emit("back",true));
    }
    @Override protected void onNewIntent(Intent intent) {
        super.onNewIntent(intent); setIntent(intent);
        if (VpnTileService.CONNECT.equals(intent.getAction())) { tileConnect=true; connectFromTile(); }
    }
    private void connectFromTile() {
        if (tileConnect && CoreBridge.ready()) { tileConnect=false; prepareVpn(); }
    }
    // NativeActivity cannot use ComponentActivity. API 33+ dispatches through
    // OnBackInvokedDispatcher above; this entry is solely for API 26–32.
    @android.annotation.SuppressLint("GestureBackNavigation")
    @Override public void onBackPressed() {
        if (Build.VERSION.SDK_INT < 33) emit("back",true);
        else super.onBackPressed();
    }
    @Override protected void onResume() {
        super.onResume(); emit("visible", true);
        if (waitingForInstallPermission) {
            waitingForInstallPermission=false;
            if (getPackageManager().canRequestPackageInstalls()) UpdateInstaller.launch(this);
            else emit("notice","尚未允许安装更新，可在设置页点击安装更新重试");
        }
    }
    @Override protected void onPause() { emit("visible", false); super.onPause(); }

    /** Invoked by the Rust worker, never performs blocking work on the UI thread. */
    public String request(String input) {
        try {
            JSONObject data = new JSONObject(input);
            String op = data.getString("op");
            switch (op) {
                case "initialize": CoreBridge.start(this, data.getString("config"), data.getBoolean("ipv6")); runOnUiThread(this::connectFromTile); break;
                case "reload": CoreBridge.reload(data.getString("config")); CoreBridge.setIpv6(data.getBoolean("ipv6")); break;
                case "state": return CoreBridge.state()
                    .put("font_sizes",systemFontSizes())
                    .put("dark", Appearance.systemDark(this))
                    .put("notification_speeds",getSharedPreferences("appearance",MODE_PRIVATE).getBoolean("notification-speeds",true))
                    .put("excluded_apps",getSharedPreferences("vpn",MODE_PRIVATE).getStringSet("excluded",Collections.emptySet()).size()).toString();
                case "connect": TunnelService.clearError(); runOnUiThread(this::prepareVpn); break;
                case "reconnect": TunnelService.clearError(); runOnUiThread(() -> prepareVpn(true)); break;
                case "authorize-vpn": {
                    long request = data.getLong("request");
                    runOnUiThread(() -> authorizeVpn(request));
                    break;
                }
                case "cancel-vpn-authorization": {
                    long request = data.getLong("request");
                    runOnUiThread(() -> {
                        if (vpnAuthorizationOnly && vpnAuthorizationRequest == request)
                            try {
                                vpnAuthorizationRequest = 0;
                                vpnAuthorizationState = new JSONObject().put("request",request).put("status","cancelled").toString();
                            } catch (Exception error) { emit("error",error.toString()); }
                    });
                    break;
                }
                case "disconnect": TunnelService.stop(this); break;
                case "protection-sync": ProtectionRuntime.sync(this); break;
                case "protection-pause": ProtectionRuntime.pause(this); break;
                case "vpn-plan":
                    getSharedPreferences("vpn", MODE_PRIVATE).edit().putBoolean("desired", data.getBoolean("enabled")).commit();
                    break;
                case "traffic": TunnelService.updateTraffic(this, data.optString("down"), data.optString("up")); break;
                case "appearance": return Appearance.startup(this).put("font_sizes",systemFontSizes()).toString();
                case "system-bars": runOnUiThread(() -> {
                    Appearance.persistSystemMode(this,data.optInt("mode",2));
                    configureSystemBars(data.optBoolean("dark"));
                }); break;
                case "install-update": UpdateInstaller.request(this); break;
                case "copy": {
                    String text = data.getString("text");
                    java.util.concurrent.FutureTask<Void> copied = new java.util.concurrent.FutureTask<>(() -> {
                        ((android.content.ClipboardManager)getSystemService(CLIPBOARD_SERVICE))
                            .setPrimaryClip(ClipData.newPlainText("Clash of Rust", text));
                        return null;
                    });
                    if (Looper.myLooper() == Looper.getMainLooper()) copied.run();
                    else runOnUiThread(copied);
                    // A queued runnable is not a clipboard acknowledgement.
                    try { copied.get(3, java.util.concurrent.TimeUnit.SECONDS); }
                    catch (Exception error) { copied.cancel(false); throw error; }
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
                case "background": runOnUiThread(() -> moveTaskToBack(true)); break;
                case "vpn-settings": runOnUiThread(() -> startActivity(new Intent(Settings.ACTION_VPN_SETTINGS))); break;
                case "battery-settings": runOnUiThread(() -> startActivity(new Intent(Settings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS))); break;
                case "application-routing": runOnUiThread(this::applicationRouting); break;
                case "notification-speeds":
                    getSharedPreferences("appearance", MODE_PRIVATE).edit().putBoolean("notification-speeds", data.getBoolean("value")).apply(); break;
                case "notify": {
                    String title = data.optString("title", "Clash of Rust"), message = data.optString("message", "喵!");
                    if (!SystemNotifications.permitted(this) && data.optBoolean("request_permission")) {
                        runOnUiThread(() -> {
                            notificationTitle = title; notificationMessage = message;
                            requestPermissions(new String[]{android.Manifest.permission.POST_NOTIFICATIONS}, NOTIFICATIONS);
                        });
                    } else SystemNotifications.show(this, title, message);
                    break;
                }
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

    private org.json.JSONArray systemFontSizes() throws Exception {
        android.util.DisplayMetrics metrics=getResources().getDisplayMetrics();
        org.json.JSONArray sizes=new org.json.JSONArray();
        for (int sp=10;sp<=40;sp++) sizes.put(android.util.TypedValue.applyDimension(
            android.util.TypedValue.COMPLEX_UNIT_SP,sp,metrics)/metrics.density);
        return sizes;
    }
    @SuppressWarnings("deprecation")
    private void configureSystemBars(boolean dark) {
        android.view.Window window=getWindow();
        window.setStatusBarColor(android.graphics.Color.TRANSPARENT);
        window.setNavigationBarColor(android.graphics.Color.TRANSPARENT);
        if (Build.VERSION.SDK_INT>=30) {
            window.setDecorFitsSystemWindows(false);
            android.view.WindowInsetsController controller=window.getInsetsController();
            if (controller!=null) {
                int light=android.view.WindowInsetsController.APPEARANCE_LIGHT_STATUS_BARS
                    |android.view.WindowInsetsController.APPEARANCE_LIGHT_NAVIGATION_BARS;
                controller.setSystemBarsAppearance(dark?0:light,light);
            }
        } else {
            int flags=android.view.View.SYSTEM_UI_FLAG_LAYOUT_STABLE|android.view.View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN
                |android.view.View.SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION;
            if (!dark) flags|=android.view.View.SYSTEM_UI_FLAG_LIGHT_STATUS_BAR|android.view.View.SYSTEM_UI_FLAG_LIGHT_NAVIGATION_BAR;
            window.getDecorView().setSystemUiVisibility(flags);
        }
        if (Build.VERSION.SDK_INT>=29) window.setNavigationBarContrastEnforced(false);
    }
    private void prepareVpn() {
        prepareVpn(false);
    }
    private void authorizationResult(long request, boolean granted, String error) {
        try {
            JSONObject result = new JSONObject().put("request", request).put("status", granted ? "granted" : "denied")
                .put("granted", granted).put("error", error);
            vpnAuthorizationState = result.toString();
            emit("vpn-authorized", result);
        } catch (Exception failure) { emit("error", failure.toString()); }
    }
    private void authorizeVpn(long request) {
        if (vpnPrompt) {
            authorizationResult(request, false, "已有 VPN 授权正在进行");
            return;
        }
        try {
            Intent permission = VpnService.prepare(this);
            if (permission == null) {
                authorizationResult(request, true, "");
                return;
            }
            // Consent alone must not start a VPN or change the active DNS.
            // Rust applies protection only after this matching result returns.
            vpnAuthorizationOnly = true;
            vpnAuthorizationRequest = request;
            vpnPrompt = true;
            vpnAuthorizationState = new JSONObject().put("request",request).put("status","pending").toString();
            startActivityForResult(permission, VPN);
        } catch (Exception error) {
            vpnPrompt = false;
            vpnAuthorizationOnly = false;
            vpnAuthorizationRequest = 0;
            authorizationResult(request, false, error.toString());
        }
    }
    private void prepareVpn(boolean reconfigure) {
        if (vpnPrompt || (TunnelService.connected() && !reconfigure)) return;
        if (!CoreBridge.ready()) { emit("error", "请先启动内核"); return; }
        Intent permission = VpnService.prepare(this);
        vpnReconfigure = reconfigure;
        if (permission == null) startVpn(); else {
            vpnPrompt = true;
            startActivityForResult(permission, VPN);
        }
    }
    private void startVpn() {
        Intent start = new Intent(this, TunnelService.class);
        if (vpnReconfigure) start.setAction(TunnelService.RECONFIGURE);
        vpnReconfigure = false;
        startForegroundService(start);
        if (Build.VERSION.SDK_INT >= 33 && checkSelfPermission(android.Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED)
            requestPermissions(new String[]{android.Manifest.permission.POST_NOTIFICATIONS}, 14);
    }
    @Override protected void onActivityResult(int request, int result, Intent data) {
        super.onActivityResult(request, result, data);
        if (request == VPN) {
            vpnPrompt = false;
            if (vpnAuthorizationOnly) {
                long pending = vpnAuthorizationRequest;
                vpnAuthorizationOnly = false;
                vpnAuthorizationRequest = 0;
                if (pending != 0) {
                    boolean granted = result == RESULT_OK && VpnService.prepare(this) == null;
                    authorizationResult(pending, granted, granted ? "" : "VPN 授权已取消或失效");
                }
                return;
            }
            if (result == RESULT_OK) startVpn(); else {
                vpnReconfigure = false;
                emit("vpn-cancelled", true);
                emit("error", "VPN 授权已取消");
            }
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
                        String name="导入配置";
                        try (android.database.Cursor cursor=getContentResolver().query(uri,new String[]{android.provider.OpenableColumns.DISPLAY_NAME},null,null,null)) {
                            if (cursor!=null && cursor.moveToFirst()) name=cursor.getString(0);
                        }
                        JSONObject file = new JSONObject().put("name", name)
                            .put("content", out.toString(StandardCharsets.UTF_8.name()));
                        emit("file", file);
                    }
                }
            } catch (Exception error) { emit("error", error.toString()); }
        });
        worker.shutdown();
    }
    private void applicationRouting() {
        try {
            if (CoreBridge.protectedMode(this)) {
                emit("notice", "请先关闭 Claude 保护，再修改应用分流"); return;
            }
        } catch (Exception error) { emit("error", error.toString()); return; }
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
    @Override public void onRequestPermissionsResult(int request, String[] permissions, int[] results) {
        super.onRequestPermissionsResult(request, permissions, results);
        if (request == NOTIFICATIONS) {
            String title = notificationTitle, message = notificationMessage;
            notificationTitle = null; notificationMessage = null;
            if (results.length > 0 && results[0] == PackageManager.PERMISSION_GRANTED && title != null && message != null) {
                try { SystemNotifications.show(this, title, message); }
                catch (Exception error) { emit("error", error.toString()); }
            } else emit("notice", "未获得系统通知权限，Claude 保护通知将无法显示");
        }
    }
}
