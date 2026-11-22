package org.clashofrust.android;

import android.app.*;
import android.content.*;
import android.content.pm.PackageManager;
import android.net.Uri;
import android.net.VpnService;
import android.os.*;
import android.provider.Settings;
import android.view.WindowInsets;
import android.view.inputmethod.InputMethodManager;
import java.io.*;
import java.nio.charset.StandardCharsets;
import java.util.*;
import java.util.concurrent.Executors;
import org.json.JSONObject;

public final class MainActivity extends NativeActivity {
    static { System.loadLibrary("clash_mobile"); }
    private static final int VPN = 10, OPEN = 11, SAVE = 12, NOTIFICATIONS = 13;
    private static final String VPN_STATE = "org.clashofrust.android.vpn-state";
    private static final Handler recoveryConsentHandler = new Handler(Looper.getMainLooper());
    private static MainActivity resumedVpnActivity;
    private volatile boolean vpnPrompt;
    private boolean vpnReconfigure;
    private String vpnStartRequest;
    private boolean vpnAuthorizationOnly;
    private long vpnAuthorizationRequest;
    private static volatile String vpnAuthorizationState = "{}";
    static JSONObject authorizationState() throws Exception { return new JSONObject(vpnAuthorizationState); }
    private boolean tileConnect;
    boolean waitingForInstallPermission;
    private String exportContent;
    private volatile String documentFailure;
    private volatile NetworkOverview networkOverview;
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
        Bundle vpn = state == null ? null : state.getBundle(VPN_STATE);
        if (vpn != null) {
            vpnPrompt = vpn.getBoolean("prompt");
            vpnReconfigure = vpn.getBoolean("reconfigure");
            vpnStartRequest = vpn.getString("start-request");
            vpnAuthorizationOnly = vpn.getBoolean("authorization-only");
            vpnAuthorizationRequest = vpn.getLong("authorization-request");
            if (vpnPrompt && !vpnAuthorizationOnly) TunnelService.resumeRequest(this, vpnStartRequest);
        }
        super.onCreate(state);
        IncidentNotifications.opened(this, getIntent());
        try { networkOverview = new NetworkOverview(this); }
        catch (RuntimeException error) { android.util.Log.w("ClashOfRust", "Network overview unavailable", error); }
        UpdateInstaller.pruneInstalled(this);
        configureSystemBars(dark);
        // Slint's Android helper also registers a PRIORITY_DEFAULT back callback.
        // Stay above that callback but below system overlays so a single app path
        // owns navigation and dialog dismissal.
        if (Build.VERSION.SDK_INT >= 33) getOnBackInvokedDispatcher().registerOnBackInvokedCallback(
            android.window.OnBackInvokedDispatcher.PRIORITY_DEFAULT + 1, this::handleBack);
    }
    @Override protected void onSaveInstanceState(Bundle state) {
        super.onSaveInstanceState(state);
        Bundle vpn = new Bundle();
        vpn.putBoolean("prompt", vpnPrompt);
        vpn.putBoolean("reconfigure", vpnReconfigure);
        vpn.putString("start-request", vpnStartRequest);
        vpn.putBoolean("authorization-only", vpnAuthorizationOnly);
        vpn.putLong("authorization-request", vpnAuthorizationRequest);
        state.putBundle(VPN_STATE, vpn);
    }
    @Override protected void onNewIntent(Intent intent) {
        super.onNewIntent(intent); setIntent(intent);
        IncidentNotifications.opened(this, intent);
        if (VpnTileService.CONNECT.equals(intent.getAction())) { tileConnect=true; connectFromTile(); }
    }
    private void connectFromTile() {
        if (tileConnect && CoreBridge.ready()) { tileConnect=false; prepareVpn(); }
    }
    // NativeActivity cannot use ComponentActivity. API 33+ dispatches through
    // OnBackInvokedDispatcher above; this entry is the legacy path for API 26–32.
    // Keep forwarding instead of calling super so Android 16 cannot finish the
    // Activity before Slint dismisses a dialog or navigates back.
    @android.annotation.SuppressLint("GestureBackNavigation")
    @SuppressWarnings("deprecation")
    @Override public void onBackPressed() {
        handleBack();
    }
    private void handleBack() {
        if (hideImeIfVisible()) return;
        emit("back",true);
    }
    @SuppressWarnings("deprecation")
    private boolean hideImeIfVisible() {
        InputMethodManager imm=(InputMethodManager)getSystemService(INPUT_METHOD_SERVICE);
        if (imm==null) return false;
        android.view.View root=getWindow().getDecorView().getRootView();
        WindowInsets insets=root.getRootWindowInsets();
        boolean visible;
        if (Build.VERSION.SDK_INT >= 30) {
            visible=insets!=null && insets.isVisible(WindowInsets.Type.ime());
        } else {
            android.graphics.Rect frame=new android.graphics.Rect();
            root.getWindowVisibleDisplayFrame(frame);
            int[] origin=new int[2];
            root.getLocationOnScreen(origin);
            int obscured=Math.max(0,origin[1]+root.getHeight()-frame.bottom);
            int navigation=insets!=null ? insets.getStableInsetBottom() : 0;
            visible=obscured>navigation;
        }
        if (!visible) return false;
        imm.hideSoftInputFromWindow(getWindow().getDecorView().getWindowToken(),0);
        return true;
    }
    @Override protected void onResume() {
        super.onResume(); emit("visible", true);
        if (waitingForInstallPermission) {
            waitingForInstallPermission=false;
            if (getPackageManager().canRequestPackageInstalls()) UpdateInstaller.launch(this);
            else emit("notice",AppLanguage.current(this).getString(R.string.update_permission_missing));
        }
    }
    @Override protected void onPostResume() {
        super.onPostResume();
        resumedVpnActivity = this;
        preparePendingRecoveryConsent();
    }
    @Override protected void onPause() {
        if (resumedVpnActivity == this) resumedVpnActivity = null;
        emit("visible", false);
        super.onPause();
    }
    @Override protected void onDestroy() {
        if (isFinishing()) TunnelService.cancelStart(this, vpnStartRequest);
        NetworkOverview current = networkOverview;
        networkOverview = null;
        if (current != null) current.close();
        super.onDestroy();
    }
    private long networkRevision() {
        NetworkOverview current = networkOverview;
        return current == null ? 0 : current.revision();
    }

    /** Invoked by the Rust worker, never performs blocking work on the UI thread. */
    public String request(String input) {
        try {
            JSONObject data = new JSONObject(input);
            String op = data.getString("op");
            switch (op) {
                case "initialize":
                    CoreBridge.start(this, data.getString("config"), data.getBoolean("ipv6"));
                    if (data.optBoolean("connect_from_tile", true)) runOnUiThread(this::connectFromTile);
                    break;
                case "reload": CoreBridge.reload(this, data.getString("config"), data.getBoolean("ipv6")); break;
                case "finish-protection-recovery": {
                    Boolean desired = data.isNull("desired") ? null : data.getBoolean("desired");
                    return TunnelService.finishRecovery(this, data.getString("request"),
                        data.getBoolean("connected"), desired, data.getLong("generation"),
                        data.getString("config"), data.getBoolean("ipv6"), data.getBoolean("protected"),
                        data.getLong("deadline_elapsed_ms")).toString();
                }
                case "protection-recovery-receipt": return new JSONObject().put(
                    "committed", TunnelService.recoveryCommitted(this, data.getString("request"))).toString();
                case "reserve-vpn-recovery": {
                    Boolean desired = data.isNull("desired") ? null : data.getBoolean("desired");
                    String token = data.getString("token");
                    TunnelService.reserveRecovery(this, token, data.getBoolean("connected"),
                        desired, data.getLong("generation"));
                    return new JSONObject().put("token", token).toString();
                }
                case "release-vpn-recovery": TunnelService.releaseRecovery(data.getString("token")); break;
                case "hold-vpn-recovery": TunnelService.holdRecovery(this); break;
                case "state": {
                    TunnelService.refreshNotificationChannel(AppLanguage.current(this));
                    return CoreBridge.state(this)
                        .put("network_revision",networkRevision())
                        .put("font_sizes",systemFontSizes())
                        .put("dark", Appearance.systemDark(this))
                        .put("notification_speeds",getSharedPreferences("appearance",MODE_PRIVATE).getBoolean("notification-speeds",true))
                        .put("incident_notifications", IncidentNotifications.state(this))
                        .put("application_scope",ApplicationRouting.load(this).summary(this)).toString();
                }
                case "network-info": {
                    NetworkOverview current = networkOverview;
                    if (current == null) throw new IOException(AppLanguage.current(this).getString(R.string.network_info_pending));
                    return current.read().toString();
                }
                case "connect":
                case "reconnect": {
                    boolean reconfigure = op.equals("reconnect");
                    if (vpnPrompt) {
                        if (reconfigure) throw new IOException(AppLanguage.current(this).getString(R.string.vpn_authorization_pending));
                        break;
                    }
                    String request = TunnelService.prepareStart(this);
                    if (request == null)
                        throw new IOException(AppLanguage.current(this).getString(R.string.protection_unconfirmed));
                    TunnelService.clearError();
                    runOnUiThread(() -> prepareVpn(reconfigure, request));
                    break;
                }
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
                case "restore-vpn": {
                    Boolean live = data.isNull("connected") ? null : data.getBoolean("connected");
                    Boolean desired = data.isNull("desired") ? null : data.getBoolean("desired");
                    return TunnelService.restoreNetwork(this, data.getString("request"), live, desired,
                        data.getLong("deadline_elapsed_ms")).toString();
                }
                case "expire-vpn-start": return TunnelService.expireVpnStart(this, data.getString("request")).toString();
                case "protection-sync": ProtectionRuntime.sync(this); break;
                case "protection-pause": ProtectionRuntime.pause(this); break;
                case "appearance": return Appearance.startup(this).put("font_sizes",systemFontSizes()).toString();
                case "page-views": {
                    SharedPreferences views = getSharedPreferences("page-views", MODE_PRIVATE);
                    return new JSONObject().put("nodes_list", views.getBoolean("nodes_list", false))
                        .put("profiles_list", views.getBoolean("profiles_list", false)).toString();
                }
                case "save-page-views":
                    getSharedPreferences("page-views", MODE_PRIVATE).edit()
                        .putBoolean("nodes_list", data.getBoolean("nodes_list"))
                        .putBoolean("profiles_list", data.getBoolean("profiles_list")).apply();
                    break;
                case "system-languages": {
                    android.os.LocaleList locales = android.content.res.Resources.getSystem().getConfiguration().getLocales();
                    org.json.JSONArray languages = new org.json.JSONArray();
                    for (int index = 0; index < locales.size(); index++) languages.put(locales.get(index).toLanguageTag());
                    return new JSONObject().put("languages", languages).toString();
                }
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
                        if (exportContent != null) { emit("export-error", AppLanguage.current(this).getString(R.string.export_in_progress)); return; }
                        exportContent = content;
                        documentFailure = null;
                        try {
                            startActivityForResult(new Intent(Intent.ACTION_CREATE_DOCUMENT)
                                .setType(name.endsWith(".json") ? "application/json" : "text/plain").addCategory(Intent.CATEGORY_OPENABLE)
                                .putExtra(Intent.EXTRA_TITLE, name), SAVE);
                        } catch (Exception error) {
                            exportContent = null;
                            documentFailure = error.toString();
                            android.util.Log.e("ClashOfRust",
                                "DOCUMENT_PICKER_FAILED (" + error.getClass().getSimpleName() + ")");
                            emit("export-error", AppLanguage.current(this).getString(R.string.export_open_failed, error));
                        }
                    });
                    break;
                }
                case "diagnostics": return new JSONObject()
                    .put("sdk", android.os.Build.VERSION.SDK_INT)
                    .put("release", android.os.Build.VERSION.RELEASE)
                    .put("manufacturer", android.os.Build.MANUFACTURER)
                    .put("model", android.os.Build.MODEL)
                    .put("network", networkOverview == null
                        ? new JSONObject().put("state", "not_initialized")
                        : networkOverview.diagnostics())
                    .put("socket_protection", new JSONObject(
                        org.clashofrust.nativecore.mobile.Mobile.socketDiagnostics()))
                    .put("vpn", new JSONObject().put("connected", TunnelService.connected())
                        .put("pending", TunnelService.pending()).put("generation", TunnelService.generation())
                        .put("error", TunnelService.error())).toString();
                case "background": runOnUiThread(() -> moveTaskToBack(true)); break;
                case "vpn-settings": runOnUiThread(() -> startActivity(new Intent(Settings.ACTION_VPN_SETTINGS))); break;
                case "battery-settings": runOnUiThread(() -> startActivity(new Intent(Settings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS))); break;
                case "applications-list": return ApplicationCatalog.read(this).toString();
                case "applications-save": return ApplicationCatalog.save(this, data).toString();
                case "application-policy-capability": return ApplicationOwners.capability(this).toString();
                case "applications-reconnect":
                    if (CoreBridge.protectedMode(this))
                        throw new IllegalStateException(AppLanguage.current(this).getString(R.string.applications_guard_rules));
                    if (TunnelService.connected()) TunnelService.start(this, true);
                    break;
                case "applications-permissions":
                    runOnUiThread(() -> {
                        try {
                            startActivity(new Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS,
                                Uri.parse("package:" + getPackageName())));
                        } catch (RuntimeException error) { emit("error", AppLanguage.current(this).getString(R.string.applications_permission_settings, error)); }
                    });
                    break;
                case "notification-speeds":
                    getSharedPreferences("appearance", MODE_PRIVATE).edit().putBoolean("notification-speeds", data.getBoolean("value")).apply(); break;
                case "notify-incident":
                    return new JSONObject().put("posted", IncidentNotifications.publish(this,
                        data.getBoolean("critical") ? "critical" : "recovery", false)).toString();
                case "report-error":
                    IncidentNotifications.publish(this, "recovery", false);
                    emit("error", data.getString("message")); break;
                case "acknowledge-incident":
                    IncidentNotifications.acknowledge(this, data.getLong("sequence")); break;
                case "notify": {
                    String title = data.optString("title", "Clash of Rust"), message = data.optString("message", AppLanguage.current(this).getString(R.string.notification_test_message));
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
                        throw new IOException(AppLanguage.current(this).getString(R.string.link_scheme));
                    runOnUiThread(() -> {
                        try { startActivity(new Intent(Intent.ACTION_VIEW, url)); }
                        catch (RuntimeException error) { emit("link-error", AppLanguage.current(this).getString(R.string.link_open_failed, error)); }
                    });
                    break;
                }
                default: throw new IOException(AppLanguage.current(this).getString(R.string.android_operation_unknown, op));
            }
            return new JSONObject().put("ok", true).toString();
        } catch (Exception error) {
            return "{\"error\":" + JSONObject.quote(error.toString()) + "}";
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
        if (vpnPrompt) return;
        prepareVpn(false, TunnelService.prepareStart(this));
    }
    static void dispatchRecoveryConsent() {
        recoveryConsentHandler.post(() -> {
            MainActivity activity = resumedVpnActivity;
            if (activity != null) activity.preparePendingRecoveryConsent();
        });
    }
    private void preparePendingRecoveryConsent() {
        if (vpnPrompt || isFinishing() || isDestroyed()) return;
        String request = TunnelService.takeRecoveryConsent(this);
        if (request != null) prepareVpn(true, request);
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
            authorizationResult(request, false, AppLanguage.current(this).getString(R.string.vpn_authorization_pending));
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
            authorizationResult(request, false, AppLanguage.current(this).getString(R.string.vpn_authorization_open_failed, error));
        }
    }
    void prepareVpn(boolean reconfigure, String request) {
        if (!TunnelService.startAllowed(this, request)) return;
        if (isFinishing() || isDestroyed() || vpnPrompt || (TunnelService.connected() && !reconfigure)) {
            TunnelService.cancelStart(this, request);
            return;
        }
        try {
            if (!CoreBridge.ready()) throw new IllegalStateException(AppLanguage.current(this).getString(R.string.core_start_first));
            Intent permission = VpnService.prepare(this);
            vpnReconfigure = reconfigure;
            vpnStartRequest = request;
            if (permission == null) startVpn(); else {
                vpnPrompt = true;
                startActivityForResult(permission, VPN);
            }
        } catch (RuntimeException error) {
            vpnPrompt = false;
            vpnReconfigure = false;
            vpnStartRequest = null;
            TunnelService.cancelStart(this, request);
            emit("error", error.toString());
        }
    }
    private void startVpn() {
        String request = vpnStartRequest;
        vpnStartRequest = null;
        boolean reconfigure = vpnReconfigure;
        vpnReconfigure = false;
        try { if (!TunnelService.start(this, reconfigure, request)) return; }
        catch (RuntimeException error) { emit("error", error.toString()); return; }
        if (Build.VERSION.SDK_INT >= 33 && checkSelfPermission(android.Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED)
            requestPermissions(new String[]{android.Manifest.permission.POST_NOTIFICATIONS}, 14);
    }
    @Override protected void onActivityResult(int request, int result, Intent data) {
        super.onActivityResult(request, result, data);
        if (request == VPN) {
            if (!vpnPrompt) return;
            vpnPrompt = false;
            dispatchRecoveryConsent();
            if (vpnAuthorizationOnly) {
                long pending = vpnAuthorizationRequest;
                vpnAuthorizationOnly = false;
                vpnAuthorizationRequest = 0;
                if (pending != 0) {
                    boolean granted = result == RESULT_OK && VpnService.prepare(this) == null;
                    authorizationResult(pending, granted, granted ? "" : AppLanguage.current(this).getString(R.string.vpn_authorization_cancelled));
                }
                return;
            }
            if (result == RESULT_OK) startVpn(); else {
                String pending = vpnStartRequest;
                vpnReconfigure = false;
                vpnStartRequest = null;
                if (!TunnelService.cancelStart(this, pending)) return;
                emit("vpn-cancelled", true);
                emit("error", AppLanguage.current(this).getString(R.string.vpn_authorization_cancelled_short));
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
                    if (content == null) throw new IOException(AppLanguage.current(this).getString(R.string.export_expired));
                    try (OutputStream out = getContentResolver().openOutputStream(uri, "wt")) {
                        if (out == null) throw new IOException(AppLanguage.current(this).getString(R.string.export_write_failed));
                        out.write(content.getBytes(StandardCharsets.UTF_8));
                    }
                    emit("export-notice", AppLanguage.current(this).getString(R.string.export_complete));
                } else {
                    try (InputStream in = getContentResolver().openInputStream(uri)) {
                        if (in == null) throw new IOException(AppLanguage.current(this).getString(R.string.import_read_failed));
                        ByteArrayOutputStream out = new ByteArrayOutputStream();
                        byte[] buffer = new byte[8192];
                        int count;
                        while ((count = in.read(buffer)) != -1) {
                            if (out.size() + count > 10 * 1024 * 1024) throw new IOException(AppLanguage.current(this).getString(R.string.import_too_large));
                            out.write(buffer, 0, count);
                        }
                        String name=AppLanguage.current(this).getString(R.string.import_default_name);
                        try (android.database.Cursor cursor=getContentResolver().query(uri,new String[]{android.provider.OpenableColumns.DISPLAY_NAME},null,null,null)) {
                            if (cursor!=null && cursor.moveToFirst()) name=cursor.getString(0);
                        }
                        JSONObject file = new JSONObject().put("name", name)
                            .put("content", out.toString(StandardCharsets.UTF_8.name()));
                        emit("file", file);
                    }
                }
            } catch (Exception error) {
                documentFailure = error.toString();
                android.util.Log.e("ClashOfRust",
                    (request == SAVE ? "DOCUMENT_EXPORT_FAILED" : "DOCUMENT_IMPORT_FAILED")
                        + " (" + error.getClass().getSimpleName() + ")");
                emit(request == SAVE ? "export-error" : "error", AppLanguage.current(this).getString(
                    request == SAVE ? R.string.export_failed : R.string.import_failed, error));
            }
        });
        worker.shutdown();
    }
    @Override public void onRequestPermissionsResult(int request, String[] permissions, int[] results) {
        super.onRequestPermissionsResult(request, permissions, results);
        if (request == NOTIFICATIONS) {
            String title = notificationTitle, message = notificationMessage;
            notificationTitle = null; notificationMessage = null;
            if (results.length > 0 && results[0] == PackageManager.PERMISSION_GRANTED && title != null && message != null) {
                try { SystemNotifications.show(this, title, message); }
                catch (Exception error) { emit("error", AppLanguage.current(this).getString(R.string.notification_send_failed, error)); }
            } else emit("notice", AppLanguage.current(this).getString(R.string.notification_permission_denied));
        }
    }
}
