package org.clashofrust.android;

import android.annotation.TargetApi;
import android.app.ActivityManager;
import android.app.Application;
import android.app.ApplicationExitInfo;
import android.app.Instrumentation;
import android.content.Context;
import android.net.ConnectivityManager;
import android.net.NetworkCapabilities;
import android.net.VpnService;
import android.os.Bundle;
import android.os.Process;
import android.os.SystemClock;
import android.system.OsConstants;
import android.view.InputDevice;
import android.view.MotionEvent;
import android.view.inspector.WindowInspector;
import androidx.test.ext.junit.runners.AndroidJUnit4;
import androidx.test.platform.app.InstrumentationRegistry;
import java.io.FileOutputStream;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import org.json.JSONArray;
import org.json.JSONObject;
import org.junit.Test;
import org.junit.runner.RunWith;
import static org.junit.Assert.*;

/** Each method is driven separately by android.py on a disposable API 36 AVD. */
@RunWith(AndroidJUnit4.class)
@TargetApi(36)
public class ProtectionProcessDeathTest {
    private static final String PROFILE = "50a953db-ed60-444c-a63d-30df0c79fd45";
    private static final String NODE = "CI-Process-Death";
    private static final String ROUTE = "COR-Claude-00000000000000000000000000000001";
    private static final String CONFIG = """
        mode: rule
        geo-auto-update: false
        dns:
          enable: false
        proxies:
          - {name: CI-Process-Death, type: http, server: 127.0.0.1, port: 9}
        rules:
          - MATCH,REJECT
        """;

    private static Instrumentation instrumentation() {
        return InstrumentationRegistry.getInstrumentation();
    }

    private static Context context() {
        return instrumentation().getTargetContext();
    }

    private static String token() {
        String token = InstrumentationRegistry.getArguments().getString("death_token", "");
        assertTrue("The host must supply a fresh run token", token.matches("[0-9a-f]{32}"));
        assertEquals("Instrumentation must run in the application process",
            context().getPackageName(), Application.getProcessName());
        return token;
    }

    private static Path client(String name) {
        return context().getFilesDir().toPath().resolve("client").resolve(name);
    }

    private static Path handoff() {
        return context().getCacheDir().toPath().resolve("protection-process-death.json");
    }

    private static JSONObject read(Path path) throws Exception {
        return new JSONObject(readText(path));
    }

    private static String readText(Path path) throws Exception {
        return new String(Files.readAllBytes(path), StandardCharsets.UTF_8);
    }

    private static void persist(Path path, String value) throws Exception {
        try (FileOutputStream file = new FileOutputStream(path.toFile())) {
            file.write(value.getBytes(StandardCharsets.UTF_8));
            file.getFD().sync();
        }
    }

    private static void report(String key, String value) {
        Bundle evidence = new Bundle();
        evidence.putString(key, value);
        instrumentation().sendStatus(2, evidence);
    }

    private static Object appMethod(Class<?> owner, String name, Class<?>[] types, Object... args)
            throws Exception {
        Class<?> target = context().getClassLoader().loadClass(owner.getName());
        var method = target.getDeclaredMethod(name, types);
        method.setAccessible(true);
        return method.invoke(null, args);
    }

    private static JSONObject diagnostics() throws Exception {
        return (JSONObject) appMethod(MainActivity.class, "diagnostics", new Class<?>[0]);
    }

    private static boolean connected() throws Exception {
        return CoreBridge.state(context()).getBoolean("vpn");
    }

    private interface Condition { boolean ready() throws Exception; }

    private static void await(String message, Condition condition) throws Exception {
        long deadline = SystemClock.uptimeMillis() + 30_000;
        do {
            if (condition.ready()) return;
            SystemClock.sleep(100);
        } while (SystemClock.uptimeMillis() < deadline);
        fail(message);
    }

    private static boolean systemVpn() {
        ConnectivityManager connectivity = context().getSystemService(ConnectivityManager.class);
        NetworkCapabilities network = connectivity.getNetworkCapabilities(connectivity.getActiveNetwork());
        return network != null && network.hasTransport(NetworkCapabilities.TRANSPORT_VPN);
    }

    @Test public void prepareAndKill() throws Exception {
        String token = token();
        Context context = context();
        await("The fresh Activity did not initialize its core", () -> {
            JSONObject ui = diagnostics();
            return ui.optBoolean("live") && ui.optBoolean("core_running");
        });
        assertFalse("The disposable emulator must start without an active VPN", connected() || systemVpn());
        Path recoveryRecord = client("protection-recovery.json");
        assertFalse(Files.exists(recoveryRecord));
        assertFalse(Files.exists(handoff()));
        assertNull("The disposable host must grant test VPN consent", VpnService.prepare(context));

        Path settingsPath = client("settings.json");
        JSONObject settings = read(settingsPath);
        Path index = client("profiles.json");
        JSONArray profiles = new JSONArray(readText(index));
        profiles.put(new JSONObject().put("id", PROFILE).put("name", NODE)
            .put("source", "ci-process-death.yaml").put("updated", 0).put("usage", JSONObject.NULL)
            .put("builtin", false));
        persist(client("profiles/" + PROFILE + ".yaml"), CONFIG);
        persist(index, profiles.toString());
        assertTrue(settings.isNull("claude_protection"));
        settings.put("active_profile", PROFILE).put("run_mode", "rule");
        persist(settingsPath, settings.toString());
        CoreBridge.reload(context, CONFIG);
        appMethod(TunnelService.class, "start", new Class<?>[]{Context.class, boolean.class}, context, false);
        await("The original process did not establish a real VPN", () -> {
            JSONObject platform = CoreBridge.state(context);
            return platform.getBoolean("vpn") && systemVpn() && !platform.getBoolean("vpn_transition_pending");
        });

        assertTrue(context.getSharedPreferences("vpn", Context.MODE_PRIVATE)
            .edit().putBoolean("desired", false).commit());
        JSONObject baseline = new JSONObject().put("connected", true).put("desired", false);
        JSONObject record = new JSONObject().put("version", 1).put("token", token)
            .put("profile", PROFILE).put("run_mode", "rule")
            .put("protection", JSONObject.NULL).put("vpn", baseline);
        persist(recoveryRecord, record.toString());
        settings.put("run_mode", "global").put("claude_protection", new JSONObject()
            .put("profile_id", PROFILE).put("node", NODE).put("route", ROUTE)
            .put("previous", new JSONObject().put("mode", "rule")
                .put("proxy_mode", "off").put("selectors", new JSONArray())));
        persist(settingsPath, settings.toString());
        assertTrue(context.getSharedPreferences("vpn", Context.MODE_PRIVATE)
            .edit().putBoolean("desired", true).commit());
        assertTrue(connected() && systemVpn());

        int victim = Process.myPid();
        persist(handoff(), new JSONObject().put("token", token).put("pid", victim)
            .put("timestamp", System.currentTimeMillis()).toString());
        report("cor_death_prepared", token + ":" + victim);
        Process.killProcess(victim);
        fail("SIGKILL returned without killing the application");
    }

    @Test public void recoverAfterDeath() throws Exception {
        String token = token();
        Context context = context();
        JSONObject handoff = read(handoff());
        assertEquals(token, handoff.getString("token"));
        int victim = handoff.getInt("pid");
        int replacement = Process.myPid();
        assertNotEquals("Phase two must run in a new process", victim, replacement);
        assertEquals(InstrumentationRegistry.getArguments().getString("death_pid"),
            Integer.toString(victim));
        ActivityManager manager = context.getSystemService(ActivityManager.class);
        await("Android did not record SIGKILL for the prepared PID", () ->
            manager.getHistoricalProcessExitReasons(null, victim, 8).stream().anyMatch(exit ->
                exit.getPid() == victim && context.getPackageName().equals(exit.getProcessName())
                    && exit.getTimestamp() >= handoff.optLong("timestamp", Long.MAX_VALUE)
                    && exit.getReason() == ApplicationExitInfo.REASON_SIGNALED
                    && exit.getStatus() == OsConstants.SIGKILL));
        report("cor_death_observed", token + ":" + victim + ":" + replacement);

        await("The cold Activity did not show its recovery notice", () -> {
            JSONObject ui = diagnostics();
            return ui.optBoolean("live") && ui.optBoolean("notice_modal") && ui.optBoolean("notice_error")
                && !ui.optBoolean("working");
        });
        await("The killed process's VPN network did not disappear", () -> !systemVpn());
        Path recoveryRecord = client("protection-recovery.json");
        JSONObject original = read(recoveryRecord);
        assertEquals(token, original.getString("token"));
        assertEquals(1, original.getInt("version"));
        assertEquals(PROFILE, original.getString("profile"));
        assertEquals("rule", original.getString("run_mode"));
        assertTrue(original.has("protection") && original.isNull("protection"));
        JSONObject originalVpn = original.getJSONObject("vpn");
        assertTrue(originalVpn.getBoolean("connected"));
        assertFalse(originalVpn.getBoolean("desired"));
        assertFalse("The unfinished configuration started the core", CoreBridge.ready());
        assertFalse("The unfinished configuration reconnected the VPN", connected());
        assertEquals("No core may have started in the replacement process", Long.valueOf(0L),
            appMethod(CoreBridge.class, "generation", new Class<?>[0]));
        assertEquals("No VPN may have started in the replacement process", 0L,
            CoreBridge.state(context).getLong("vpn_generation"));
        assertNull("Recovery must block ordinary VPN starts",
            appMethod(TunnelService.class, "prepareStart", new Class<?>[]{Context.class}, context));
        assertTrue(context.getSharedPreferences("vpn", Context.MODE_PRIVATE).getBoolean("desired", false));
        assertEquals(ROUTE, read(client("settings.json")).getJSONObject("claude_protection").getString("route"));

        await("The recovery retry button could not be tapped", ProtectionProcessDeathTest::tapRetry);
        await("Recovery did not restore the real VPN and clear the record", () ->
            Files.notExists(recoveryRecord) && CoreBridge.ready() && connected() && systemVpn());
        JSONObject restored = read(client("settings.json"));
        assertTrue(restored.isNull("claude_protection"));
        assertEquals("rule", restored.getString("run_mode"));
        assertEquals(PROFILE, restored.getString("active_profile"));
        assertEquals(CONFIG, readText(client("profiles/" + PROFILE + ".yaml")));
        assertTrue("The running core must use the restored profile", CoreBridge.config().contains(NODE));
        assertEquals(readText(client("runtime/config.yaml")), CoreBridge.config());
        JSONObject platform = CoreBridge.state(context);
        assertFalse("The restored VPN should leave recovery hold", platform.getBoolean("vpn_held"));
        assertFalse(platform.getJSONObject("protection").getBoolean("enabled"));
        assertFalse("Recovery must restore the saved plan independently of live TUN",
            platform.getBoolean("vpn_desired"));
        assertEquals("complete", platform.getJSONObject("vpn_rollback").getString("status"));
        appMethod(TunnelService.class, "stop", new Class<?>[]{Context.class}, context);
        await("The disposable VPN did not disconnect", () -> !connected() && !systemVpn());
    }

    private static boolean tapRetry() throws Exception {
        JSONObject ui = diagnostics();
        if (!ui.optBoolean("live") || !ui.optBoolean("notice_modal") || ui.optBoolean("working")) return false;
        JSONObject targets = ui.optJSONObject("targets");
        JSONArray button = targets == null ? null : targets.optJSONArray("notice-retry-core");
        if (button == null || button.length() != 4
                || button.getDouble(2) <= 0 || button.getDouble(3) <= 0) return false;
        double scale = ui.getDouble("scale");
        float x = (float) ((button.getDouble(0) + button.getDouble(2) / 2) * scale);
        float y = (float) ((button.getDouble(1) + button.getDouble(3) / 2) * scale);
        double width = ui.getDouble("width") * scale;
        double height = ui.getDouble("height") * scale;
        if (x < 0 || y < 0 || x >= width || y >= height) return false;
        int[] origin = new int[2];
        boolean[] focused = {false};
        instrumentation().runOnMainSync(() -> {
            for (var view : WindowInspector.getGlobalWindowViews()) {
                if (!view.hasWindowFocus() || !view.isAttachedToWindow() || !view.isShown()
                        || Math.abs(view.getWidth() - width) > 2 || Math.abs(view.getHeight() - height) > 2) continue;
                view.getLocationOnScreen(origin);
                focused[0] = true;
                break;
            }
        });
        if (!focused[0]) return false;
        var automation = instrumentation().getUiAutomation();
        long time = SystemClock.uptimeMillis();
        MotionEvent down = MotionEvent.obtain(time, time, MotionEvent.ACTION_DOWN, origin[0] + x, origin[1] + y, 0);
        try {
            down.setSource(InputDevice.SOURCE_TOUCHSCREEN);
            assertTrue(automation.injectInputEvent(down, true));
        } finally {
            down.recycle();
        }
        SystemClock.sleep(50);
        MotionEvent up = MotionEvent.obtain(time, SystemClock.uptimeMillis(), MotionEvent.ACTION_UP,
            origin[0] + x, origin[1] + y, 0);
        try {
            up.setSource(InputDevice.SOURCE_TOUCHSCREEN);
            assertTrue(automation.injectInputEvent(up, true));
            return true;
        } finally {
            up.recycle();
        }
    }
}
