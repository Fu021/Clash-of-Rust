package org.clashofrust.android;

import android.content.*;
import android.net.*;
import android.os.Build;
import android.os.SystemClock;
import android.view.MotionEvent;
import androidx.test.ext.junit.runners.AndroidJUnit4;
import androidx.test.platform.app.InstrumentationRegistry;
import java.io.*;
import java.net.*;
import java.nio.charset.StandardCharsets;
import org.json.*;
import org.junit.Test;
import org.junit.runner.RunWith;
import static org.junit.Assert.*;

/** Uses the packaged Slint Activity, embedded core and the real Android VPN. */
@RunWith(AndroidJUnit4.class)
public class ProxyTunTest {
    private static void persistNetworkProfile(Context context) throws Exception {
        assertFalse("Prepare the network profile before starting the core", CoreBridge.ready());
        File directory = new File(context.getFilesDir(), "client");
        File profiles = new File(directory, "profiles");
        assertTrue(profiles.isDirectory() || profiles.mkdirs());
        String profileId = "b4f7a20e-402d-4739-90dd-4b517591e653";
        File settingsFile = new File(directory, "settings.json");
        JSONObject settings = settingsFile.isFile()
            ? new JSONObject(new String(java.nio.file.Files.readAllBytes(settingsFile.toPath()), StandardCharsets.UTF_8))
            : new JSONObject();
        File indexFile = new File(directory, "profiles.json");
        JSONArray index = indexFile.isFile()
            ? new JSONArray(new String(java.nio.file.Files.readAllBytes(indexFile.toPath()), StandardCharsets.UTF_8))
            : new JSONArray();
        JSONArray updated = new JSONArray();
        for (int position = 0; position < index.length(); position++) {
            JSONObject profile = index.getJSONObject(position);
            if (!profileId.equals(profile.getString("id"))) updated.put(profile);
        }
        updated.put(new JSONObject().put("id", profileId).put("name", "CI TUN route")
            .put("source", "ci-tun.yaml").put("updated", 0).put("usage", JSONObject.NULL));
        String raw = "dns:\n  enable: true\n  enhanced-mode: fake-ip\n"
            + "  nameserver: [https://1.1.1.1/dns-query]\n"
            + "hosts:\n  android-tun.test: 10.0.2.2\n"
            + "proxies:\n  - {name: CI-TUN-origin, type: http, server: 10.0.2.2, port: 18081}\n"
            + "rules:\n  - IP-CIDR,198.51.100.1/32,CI-TUN-origin,no-resolve\n  - MATCH,DIRECT\n";
        settings.put("active_profile", profileId).put("run_mode", "rule").put("ipv6", true);
        java.nio.file.Files.write(new File(profiles, profileId + ".yaml").toPath(), raw.getBytes(StandardCharsets.UTF_8));
        java.nio.file.Files.write(indexFile.toPath(), updated.toString().getBytes(StandardCharsets.UTF_8));
        java.nio.file.Files.write(settingsFile.toPath(), settings.toString().getBytes(StandardCharsets.UTF_8));
    }

    private static void assertNetworkProfile(String raw) throws Exception {
        assertTrue("The active profile lost its TUN route", raw.contains("198.51.100.1/32"));
        assertTrue("The active profile lost its DNS host", raw.contains("android-tun.test"));
        JSONObject proxies = new JSONObject(read(
            new URL("http://" + value(raw, "external-controller") + "/proxies"),
            java.net.Proxy.NO_PROXY, value(raw, "secret")));
        assertTrue("The running core did not load the persisted test outbound",
            proxies.getJSONObject("proxies").has("CI-TUN-origin"));
    }

    @Test public void notificationShowsModeAndNodeWithOptionalSpeeds() {
        String text = TunnelService.statusText("1 MiB/s", "2 KiB/s", "Current node", "global", true, "");
        assertTrue(text.contains("全局 · Current node"));
        assertTrue(text.contains("↓ 1 MiB/s"));
        text = TunnelService.statusText("1 MiB/s", "2 KiB/s", "Other node", "rule", false, "");
        assertEquals("规则 · Other node", text);
        assertEquals("直连 · DIRECT", TunnelService.statusText("", "", "DIRECT", "direct", false, ""));
        assertEquals("", ProtectionRuntime.notificationLabel(false, 3));
        assertEquals("Claude 保护已启用", ProtectionRuntime.notificationLabel(true, 1));
        assertEquals("Claude 保护异常", ProtectionRuntime.notificationLabel(true, 3));
        assertEquals("Claude 保护待确认", ProtectionRuntime.notificationLabel(true, 2));
        assertEquals("Claude 保护待确认", ProtectionRuntime.notificationLabel(true, 0));
        assertEquals("Claude 保护待确认", ProtectionRuntime.notificationLabel(true, -1));
        int health = ProtectionRuntime.notificationHealth();
        assertTrue("Native protection status must use the shared readiness states", health >= 1 && health <= 3);
        for (int value : new int[]{1, 2, 3}) {
            String protection = ProtectionRuntime.notificationLabel(true, value);
            for (boolean speeds : new boolean[]{true, false}) {
                text = TunnelService.statusText("1 MiB/s", "2 KiB/s", "Current node", "global", speeds, protection);
                assertTrue("Protection must remain visible in the notification title", text.startsWith(protection + "\n"));
                assertTrue(text.contains("全局 · Current node"));
                assertEquals(speeds, text.contains("↓ 1 MiB/s"));
            }
        }
    }
    private static boolean injectPointer(int action,float x,float y,long down,long now) {
        MotionEvent event=MotionEvent.obtain(down,now,action,x,y,0);
        event.setSource(android.view.InputDevice.SOURCE_TOUCHSCREEN);
        try { return InstrumentationRegistry.getInstrumentation().getUiAutomation().injectInputEvent(event,true); }
        finally {event.recycle();}
    }
    private static void pointer(int action,float x,float y,long down,long now) {
        assertTrue("Android rejected touch input",injectPointer(action,x,y,down,now));
    }
    private static boolean tryTap(float x,float y) {
        long start=SystemClock.uptimeMillis();
        boolean pressed=injectPointer(MotionEvent.ACTION_DOWN,x,y,start,start);
        SystemClock.sleep(50);
        boolean released=injectPointer(MotionEvent.ACTION_UP,x,y,start,SystemClock.uptimeMillis());
        return pressed && released;
    }
    private static void tap(float x,float y) {
        assertTrue("Android rejected touch input",tryTap(x,y));
    }
    private static String shell(String command) throws Exception {
        try (InputStream in=new android.os.ParcelFileDescriptor.AutoCloseInputStream(
            InstrumentationRegistry.getInstrumentation().getUiAutomation().executeShellCommand(command))) {
            ByteArrayOutputStream output=new ByteArrayOutputStream();
            byte[] bytes=new byte[1024];int count;
            while ((count=in.read(bytes))!=-1) output.write(bytes,0,count);
            return output.toString(StandardCharsets.UTF_8.name());
        }
    }
    private static void tab(int index,int page) throws Exception {
        // Activity focus and accessibility readiness precede the compositor's
        // removal of ActivityRecordInputSink on some Android 15 emulator boots.
        // A globally injected event may be accepted yet discarded by that sink.
        // Navigation is idempotent: keep using real touches until the actual
        // Slint page changes, and fail if the UI remains unresponsive.
        await("UI tab did not navigate to "+page,()-> {
            JSONObject ui=MainActivity.diagnostics();
            if (ui.optInt("page",-1)==page) return true;
            if (ui.optDouble("scale",0)<=0 || ui.optDouble("width",0)<=0
                    || ui.optDouble("height",0)<=0) return false;
            float scale=(float)ui.getDouble("scale");
            tap((float)ui.getDouble("width")*(index+0.5f)/5*scale,
                ((float)ui.getDouble("height")-(float)ui.getDouble("safe_bottom")-38)*scale);
            SystemClock.sleep(500);
            return MainActivity.diagnostics().optInt("page",-1)==page;
        });
    }
    @Test public void presentationUsesSavedAppearanceAndHandlesNavigation() throws Exception {
        Context context = InstrumentationRegistry.getInstrumentation().getTargetContext();
        shell("cmd uimode night no");
        File settings=new File(context.getFilesDir(),"client/settings.json");
        settings.getParentFile().mkdirs();
        JSONObject saved=settings.isFile()?new JSONObject(new String(java.nio.file.Files.readAllBytes(settings.toPath()),StandardCharsets.UTF_8)):new JSONObject();
        saved.put("theme_mode","dark");
        saved.put("accent_color","purple");
        java.nio.file.Files.write(settings.toPath(),saved.toString().getBytes(StandardCharsets.UTF_8));
        context.startActivity(new Intent(context, MainActivity.class).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK));
        await("Native UI did not start its core; inspect logcat", CoreBridge::ready);
        await("Activity did not receive focus",()->inStage(androidx.test.runner.lifecycle.Stage.RESUMED,true));
        waitForAppWindow();
        verifyPresentation();
    }
    @Test public void incidentNotificationsKeepSeverityPermissionAndRateLimitsSeparate() throws Exception {
        assertTrue(IncidentNotifications.eligible(-1, 0));
        assertFalse(IncidentNotifications.eligible(10, 60_009));
        assertTrue(IncidentNotifications.eligible(10, 60_010));
        assertTrue(IncidentNotifications.eligible(100, 50));
        assertTrue(IncidentNotifications.message("application-target-missing").contains("被拦截"));
        Context context = InstrumentationRegistry.getInstrumentation().getTargetContext();
        if (Build.VERSION.SDK_INT >= 33)
            shell("pm grant " + context.getPackageName() + " android.permission.POST_NOTIFICATIONS");
        boolean posted = IncidentNotifications.publish(context, "recovery", false);
        assertTrue("Recovery notification was not accepted: " + IncidentNotifications.state(context), posted);
        long sent = IncidentNotifications.state(context).getLong("sent");
        assertFalse(IncidentNotifications.publish(context, "recovery", false));
        assertEquals(sent, IncidentNotifications.state(context).getLong("sent"));
        if (posted) {
            android.app.NotificationManager manager = context.getSystemService(android.app.NotificationManager.class);
            android.app.Notification[] delivered = new android.app.Notification[1];
            try {
                await("The accepted recovery notification was not posted", () -> {
                    for (android.service.notification.StatusBarNotification entry : manager.getActiveNotifications()) {
                        if (entry.getId() != IncidentNotifications.RECOVERY_ID) continue;
                        delivered[0] = entry.getNotification();
                        return true;
                    }
                    return false;
                });
                android.app.Notification notification = delivered[0];
                assertEquals(IncidentNotifications.RECOVERY_CHANNEL, notification.getChannelId());
                assertEquals(android.app.Notification.VISIBILITY_PRIVATE, notification.visibility);
                assertNotNull(notification.contentIntent);
                assertEquals(0, notification.flags & android.app.Notification.FLAG_ONGOING_EVENT);
            } finally {
                manager.cancel(IncidentNotifications.RECOVERY_ID);
            }
        }
        assertNotEquals(IncidentNotifications.RECOVERY_CHANNEL, IncidentNotifications.CRITICAL_CHANNEL);
        assertNotEquals(IncidentNotifications.RECOVERY_ID, IncidentNotifications.APPLICATION_ID);
        if (Build.VERSION.SDK_INT >= 33) {
            Context denied = new ContextWrapper(context) {
                @Override public int checkSelfPermission(String permission) {
                    return android.Manifest.permission.POST_NOTIFICATIONS.equals(permission)
                        ? android.content.pm.PackageManager.PERMISSION_DENIED : super.checkSelfPermission(permission);
                }
                @Override public SharedPreferences getSharedPreferences(String name, int mode) {
                    return context.getSharedPreferences("incident-regression-" + name, mode);
                }
            };
            assertFalse(IncidentNotifications.publish(denied, "critical", true));
            JSONObject state = IncidentNotifications.state(denied);
            assertEquals("permission_denied", state.getString("delivery"));
            assertTrue(state.getBoolean("critical"));
            assertTrue("Denied notification lost its in-app reminder", state.getLong("sequence") > 0);
            IncidentNotifications.acknowledge(denied, state.getLong("sequence"));
            assertEquals(0, IncidentNotifications.state(denied).getLong("sequence"));
        }
    }

    private static void verifyPresentation() throws Exception {
        new ProxyTunTest().notificationShowsModeAndNodeWithOptionalSpeeds();
        new ProxyTunTest().heldResponseClassifiesTruncationAsTransportFailure();
        await("Core is running but Slint still reports it as stopped",()->MainActivity.diagnostics().optBoolean("core_running"));
        JSONObject initial=MainActivity.diagnostics();
        Context context=InstrumentationRegistry.getInstrumentation().getTargetContext();
        assertEquals("The first Slint frame used a different theme from saved appearance",
            Appearance.dark(context,Appearance.mode(context)),
            initial.getBoolean("startup_dark"));
        assertEquals("The first Slint frame must use the saved purple accent instead of default green",
            2, initial.getInt("startup_accent"));
        assertEquals("The settings worker must preserve the startup accent",
            initial.getInt("startup_accent"), initial.getInt("accent"));
        // Assert the real Slint state follows system SP conversion, including
        // Android 14+ nonlinear scaling and a live font-size change.
        try {
            for (String value:new String[]{"0.85","2.0","1.0"}) {
                shell("settings put system font_scale "+value);
                await("Slint font size did not follow Android "+value,()-> {
                    android.util.DisplayMetrics metrics=InstrumentationRegistry.getInstrumentation()
                        .getTargetContext().getResources().getDisplayMetrics();
                    float expected=android.util.TypedValue.applyDimension(android.util.TypedValue.COMPLEX_UNIT_SP,16,metrics)/metrics.density;
                    double actual=MainActivity.diagnostics().optDouble("body_size",-1);
                    return Math.abs(expected-actual)<0.1 && (value.equals("1.0")?Math.abs(actual-16)<0.1:
                        value.equals("0.85")?actual<15:actual>20);
                });
            }
        } finally {shell("settings put system font_scale 1.0");}
        // Large page models are presented lazily. Inspect the real proxy page
        // after navigation instead of requiring its off-screen rows on Home.
        tab(1,1);
        await("Proxies page has no groups",()->MainActivity.diagnostics().optInt("groups")>0);
        tab(2,2);
        await("Subscription page is empty",()->MainActivity.diagnostics().optInt("profiles")>0);
        tab(4,9);
        await("Settings does not expose scrollable content",()->{
            JSONObject s=MainActivity.diagnostics();
            return s.optDouble("settings_content_height")>s.optDouble("settings_viewport_height")+100;
        });
        JSONObject ui=MainActivity.diagnostics();float scale=(float)ui.getDouble("scale");
        float x=(float)ui.getDouble("width")*0.55f*scale;
        float start=((float)ui.getDouble("height")-(float)ui.getDouble("safe_bottom")-120)*scale;
        float end=((float)ui.getDouble("safe_top")+160)*scale;
        long down=SystemClock.uptimeMillis();pointer(MotionEvent.ACTION_DOWN,x,start,down,down);
        for (int i=1;i<=15;i++) {
            SystemClock.sleep(25);pointer(MotionEvent.ACTION_MOVE,x,start+(end-start)*i/15,down,SystemClock.uptimeMillis());
        }
        pointer(MotionEvent.ACTION_UP,x,end,down,SystemClock.uptimeMillis());
        await("Touch swipe did not scroll Android settings",()->MainActivity.diagnostics().optDouble("settings_scroll_y") < -40);
        tab(0,0);
        // Wait for the launcher transition to finish before reopening. Issuing
        // HOME and am-start back-to-back can let the late HOME pause our Activity.
        shell("am start -W -a android.intent.action.MAIN -c android.intent.category.HOME");
        await("Activity did not reach the background",()->inStage(androidx.test.runner.lifecycle.Stage.STOPPED,false));
        shell("am start -W -n org.clashofrust.android/.MainActivity");
        await("Activity did not resume with window focus",()->inStage(androidx.test.runner.lifecycle.Stage.RESUMED,true));
        waitForAppWindow();
        // Actual pointer navigation proves the resumed UI is processing new events.
        tab(1,1);tab(0,0);
        android.app.UiAutomation automation=InstrumentationRegistry.getInstrumentation().getUiAutomation();
        try {
            assertTrue("Window manager rejected landscape rotation",automation.setRotation(android.app.UiAutomation.ROTATION_FREEZE_90));
            await("System display did not enter landscape",()->InstrumentationRegistry.getInstrumentation().getTargetContext()
                .getResources().getConfiguration().orientation==android.content.res.Configuration.ORIENTATION_LANDSCAPE);
            await("Landscape surface did not resize",()-> {JSONObject s=MainActivity.diagnostics();return s.optDouble("width")>s.optDouble("height");});
        } finally {
            automation.setRotation(android.app.UiAutomation.ROTATION_FREEZE_0);
        }
        try {
            await("Portrait surface did not recover",()-> {JSONObject s=MainActivity.diagnostics();return s.optDouble("height")>s.optDouble("width");});
        } finally {
            automation.setRotation(android.app.UiAutomation.ROTATION_UNFREEZE);
        }
        assertTrue(MainActivity.diagnostics().optBoolean("core_running"));
    }
    private static void waitForAppWindow() throws Exception {
        android.app.UiAutomation automation=InstrumentationRegistry.getInstrumentation().getUiAutomation();
        automation.waitForIdle(500,10000);
        await("Clash window is not visible and focused",()-> {
            MainActivity activity=resumedActivity();
            if (activity==null) return false;
            java.util.concurrent.atomic.AtomicBoolean visible=new java.util.concurrent.atomic.AtomicBoolean();
            java.util.concurrent.atomic.AtomicInteger width=new java.util.concurrent.atomic.AtomicInteger();
            java.util.concurrent.atomic.AtomicInteger height=new java.util.concurrent.atomic.AtomicInteger();
            InstrumentationRegistry.getInstrumentation().runOnMainSync(()-> {
                android.view.View decor=activity.getWindow().getDecorView();
                visible.set(decor.isAttachedToWindow() && decor.isShown()
                    && decor.hasWindowFocus() && decor.isLaidOut() && !decor.isLayoutRequested());
                width.set(decor.getWidth());
                height.set(decor.getHeight());
            });
            JSONObject ui=MainActivity.diagnostics();
            double scale=ui.optDouble("scale");
            return visible.get() && ui.optBoolean("live") && scale>0
                && ui.optDouble("width")>0 && ui.optDouble("height")>0
                && Math.abs(width.get()-ui.optDouble("width")*scale)<=2
                && Math.abs(height.get()-ui.optDouble("height")*scale)<=2;
        });
    }
    private static boolean inStage(androidx.test.runner.lifecycle.Stage stage,boolean focused) {
        java.util.concurrent.atomic.AtomicBoolean matched=new java.util.concurrent.atomic.AtomicBoolean();
        InstrumentationRegistry.getInstrumentation().runOnMainSync(()-> {
            for (android.app.Activity activity:androidx.test.runner.lifecycle.ActivityLifecycleMonitorRegistry.getInstance().getActivitiesInStage(stage)) {
                if (activity instanceof MainActivity && (!focused || activity.hasWindowFocus())) matched.set(true);
            }
        });
        return matched.get();
    }
    private static MainActivity resumedActivity() {
        java.util.concurrent.atomic.AtomicReference<MainActivity> result=new java.util.concurrent.atomic.AtomicReference<>();
        InstrumentationRegistry.getInstrumentation().runOnMainSync(()-> {
            for (android.app.Activity activity:androidx.test.runner.lifecycle.ActivityLifecycleMonitorRegistry.getInstance()
                    .getActivitiesInStage(androidx.test.runner.lifecycle.Stage.RESUMED))
                if (activity instanceof MainActivity && activity.hasWindowFocus()) result.set((MainActivity)activity);
        });
        return result.get();
    }
    private static int[] windowOrigin() throws Exception {
        java.util.concurrent.atomic.AtomicReference<int[]> origin=new java.util.concurrent.atomic.AtomicReference<>();
        await("Slint activity did not regain focus",()-> {
            MainActivity activity=resumedActivity();
            if (activity==null) return false;
            InstrumentationRegistry.getInstrumentation().runOnMainSync(()-> {
                android.view.View decor=activity.getWindow().getDecorView();
                if (activity.hasWindowFocus() && decor.isAttachedToWindow() && decor.isShown()) {
                    int[] position=new int[2];
                    decor.getLocationOnScreen(position);
                    origin.set(position);
                }
            });
            return origin.get()!=null;
        });
        return origin.get();
    }
    private static boolean tryTapTarget(String name) throws Exception {
        JSONObject ui=MainActivity.diagnostics();
        assertTrue("Refusing to touch a stale Slint snapshot",ui.optBoolean("live"));
        JSONArray bounds=ui.getJSONArray(name);
        assertEquals("Expected the real Slint control bounds",4,bounds.length());
        assertTrue("Control has no width",bounds.getDouble(2)>0);
        assertTrue("Control has no height",bounds.getDouble(3)>0);
        float scale=(float)ui.getDouble("scale");
        int[] origin=windowOrigin();
        return tryTap(origin[0]+(float)(bounds.getDouble(0)+bounds.getDouble(2)/2)*scale,
            origin[1]+(float)(bounds.getDouble(1)+bounds.getDouble(3)/2)*scale);
    }
    private static void tapTarget(String name) throws Exception {
        java.util.concurrent.atomic.AtomicReference<String> previous=new java.util.concurrent.atomic.AtomicReference<>();
        java.util.concurrent.atomic.AtomicLong stableSince=new java.util.concurrent.atomic.AtomicLong();
        await("Control "+name+" layout did not settle before touch",()-> {
            JSONObject ui=MainActivity.diagnostics();
            if (!ui.optBoolean("live")) return false;
            String geometry=ui.getJSONArray(name).toString()+ui.getDouble("keyboard_inset");
            long now=SystemClock.uptimeMillis();
            if (!geometry.equals(previous.getAndSet(geometry))) stableSince.set(now);
            return now-stableSince.get()>=250;
        });
        assertTrue("Android rejected touch input",tryTapTarget(name));
    }
    private static void dismissErrorDialogIfPresent() throws Exception {
        dismissErrorDialogIfPresent(true);
    }
    private static void dismissErrorDialogIfPresent(boolean imeEvidence) throws Exception {
        JSONObject ui=MainActivity.diagnostics();
        if (!ui.optBoolean("notice_active") || !ui.optBoolean("notice_error")) return;
        if (imeEvidence) captureIme("background-error");
        else assertFalse("Screenshot encountered a blocking error: "+ui,ui.optBoolean("notice_modal"));
        await("The error dialog close button has no layout",()-> {
            JSONArray target=MainActivity.diagnostics().optJSONArray("notice_close_target");
            return target!=null && target.length()==4 && target.optDouble(2)>0 && target.optDouble(3)>0;
        });
        tapTarget("notice_close_target");
        await("The background error dialog did not close",
            ()->!MainActivity.diagnostics().optBoolean("notice_active"));
    }
    private static void tapVisibleProfileField(String field) throws Exception {
        dismissErrorDialogIfPresent();
        await("Could not scroll the subscription "+field+" into its viewport",()-> {
            JSONObject ui=MainActivity.diagnostics();
            if (!ui.optBoolean("live")) return false;
            JSONArray target=ui.getJSONArray("profile_"+field+"_target");
            JSONArray viewport=ui.getJSONArray("profile_editor_viewport");
            double center=target.getDouble(1)+target.getDouble(3)/2;
            double top=viewport.getDouble(1), bottom=top+viewport.getDouble(3);
            if (center>=top+4 && center<=bottom-4) return true;
            assertTrue("Subscription scroll viewport is too short to swipe",bottom-top>16);
            float scale=(float)ui.getDouble("scale");
            float horizontal=(float)(viewport.getDouble(0)+8)*scale;
            float upper=(float)(top+4)*scale, lower=(float)(bottom-4)*scale;
            float start=center>bottom?lower:upper, end=center>bottom?upper:lower;
            long down=SystemClock.uptimeMillis();
            pointer(MotionEvent.ACTION_DOWN,horizontal,start,down,down);
            for (int index=1;index<=10;index++) {
                SystemClock.sleep(20);
                pointer(MotionEvent.ACTION_MOVE,horizontal,start+(end-start)*index/10,down,SystemClock.uptimeMillis());
            }
            pointer(MotionEvent.ACTION_UP,horizontal,end,down,SystemClock.uptimeMillis());
            SystemClock.sleep(750);
            return false;
        });
        tapTarget("profile_"+field+"_target");
    }
    private static android.view.View inputEditor(android.view.View view) {
        if ("SlintInputView".equals(view.getClass().getSimpleName())) return view;
        if (view instanceof android.view.ViewGroup) {
            android.view.ViewGroup group=(android.view.ViewGroup)view;
            for (int index=0;index<group.getChildCount();index++) {
                android.view.View found=inputEditor(group.getChildAt(index));
                if (found!=null) return found;
            }
        }
        return null;
    }
    @SuppressWarnings("deprecation")
    private static int imeBottom(android.view.View editor) {
        android.view.WindowInsets insets=editor.getRootWindowInsets();
        if (Build.VERSION.SDK_INT>=30) {
            return insets!=null && insets.isVisible(android.view.WindowInsets.Type.ime())
                ? insets.getInsets(android.view.WindowInsets.Type.ime()).bottom : 0;
        }
        android.view.View root=editor.getRootView();
        android.graphics.Rect frame=new android.graphics.Rect();
        root.getWindowVisibleDisplayFrame(frame);
        int[] origin=new int[2];
        root.getLocationOnScreen(origin);
        int obscured=Math.max(0,origin[1]+root.getHeight()-frame.bottom);
        int navigation=insets!=null ? insets.getStableInsetBottom() : 0;
        return obscured>navigation ? obscured : 0;
    }
    private static boolean imeVisible() {
        java.util.concurrent.atomic.AtomicBoolean visible=new java.util.concurrent.atomic.AtomicBoolean();
        MainActivity activity=resumedActivity();
        if (activity==null) return false;
        InstrumentationRegistry.getInstrumentation().runOnMainSync(()-> {
            android.view.View editor=inputEditor(activity.getWindow().getDecorView());
            if (editor==null || !editor.hasFocus()) return;
            android.view.inputmethod.InputMethodManager manager=activity.getSystemService(
                android.view.inputmethod.InputMethodManager.class);
            if (!manager.isActive(editor)) return;
            visible.set(imeBottom(editor)>0);
        });
        return visible.get();
    }
    private static void assertImeLayout(String field) throws Exception {
        MainActivity activity=resumedActivity();
        assertNotNull("No resumed input Activity",activity);
        java.util.concurrent.atomic.AtomicInteger bottom=new java.util.concurrent.atomic.AtomicInteger();
        InstrumentationRegistry.getInstrumentation().runOnMainSync(()-> {
            android.view.View editor=inputEditor(activity.getWindow().getDecorView());
            assertNotNull("Slint input editor is missing",editor);
            bottom.set(imeBottom(editor));
        });
        assertTrue("Android IME has no occluded area",bottom.get()>0);
        JSONObject ui=MainActivity.diagnostics();
        double occluded=bottom.get()/ui.getDouble("scale");
        double expected=Math.max(0,occluded-ui.getDouble("safe_bottom"));
        assertTrue("Android IME must extend above the navigation area",expected>0);
        assertEquals("Keyboard height must exclude the bottom safe area exactly once",
            expected,ui.getDouble("keyboard_inset"),2);
        JSONArray target=ui.getJSONArray("profile_"+field+"_target");
        JSONArray viewport=ui.getJSONArray("profile_editor_viewport");
        assertEquals("Missing subscription scroll viewport",4,viewport.length());
        assertTrue("Subscription "+field+" is above the scroll viewport",
            target.getDouble(1)>=viewport.getDouble(1)-2);
        assertTrue("Subscription "+field+" is clipped by the scroll viewport",
            target.getDouble(1)+target.getDouble(3)<=viewport.getDouble(1)+viewport.getDouble(3)+2);
        assertTrue("Subscription "+field+" is covered by the keyboard",
            target.getDouble(1)+target.getDouble(3)<=ui.getDouble("height")-occluded+2);
    }
    private static void commitImeText(String field,String text) throws Exception {
        await("Android input connection did not synchronize the focused "+field,()-> {
            int expected=MainActivity.diagnostics().getInt("profile_"+field+"_length");
            java.util.concurrent.atomic.AtomicBoolean synchronizedText=new java.util.concurrent.atomic.AtomicBoolean();
            MainActivity activity=resumedActivity();
            if (activity==null) return false;
            InstrumentationRegistry.getInstrumentation().runOnMainSync(()-> {
                android.view.View editor=inputEditor(activity.getWindow().getDecorView());
                if (editor==null || !editor.hasFocus()) return;
                android.view.inputmethod.InputConnection connection=editor.onCreateInputConnection(
                    new android.view.inputmethod.EditorInfo());
                if (!(connection instanceof android.view.inputmethod.BaseInputConnection)) return;
                android.text.Editable editable=((android.view.inputmethod.BaseInputConnection)connection).getEditable();
                synchronizedText.set(editable!=null
                    && Character.codePointCount(editable,0,editable.length())==expected);
            });
            return synchronizedText.get();
        });
        MainActivity activity=resumedActivity();
        assertNotNull("No resumed input Activity",activity);
        InstrumentationRegistry.getInstrumentation().runOnMainSync(()-> {
            android.view.View editor=inputEditor(activity.getWindow().getDecorView());
            assertNotNull("Slint input editor is missing",editor);
            assertTrue("Slint input editor has no Android focus",editor.hasFocus());
            assertTrue("Android has not connected the input editor",activity.getSystemService(
                android.view.inputmethod.InputMethodManager.class).isActive(editor));
            android.view.inputmethod.InputConnection connection=editor.onCreateInputConnection(
                new android.view.inputmethod.EditorInfo());
            assertTrue("Expected the production editable connection",
                connection instanceof android.view.inputmethod.BaseInputConnection);
            android.text.Editable editable=((android.view.inputmethod.BaseInputConnection)connection).getEditable();
            assertNotNull("Input connection has no editable text",editable);
            connection.beginBatchEdit();
            try {
                assertTrue("Input connection did not finish composition",connection.finishComposingText());
                assertTrue(connection.setSelection(0,editable.length()));
                assertTrue("Input connection rejected committed text",connection.commitText(text,1));
            } finally {connection.endBatchEdit();}
        });
    }
    private static void captureIme(String phase) throws Exception {
        Context context=InstrumentationRegistry.getInstrumentation().getTargetContext();
        File directory=new File(context.getExternalFilesDir(null),"ime-report");
        boolean probe=Boolean.parseBoolean(InstrumentationRegistry.getArguments().getString("ime_probe","false"));
        int round=Integer.parseInt(InstrumentationRegistry.getArguments().getString("ime_round","1"));
        assertTrue(round>=1 && round<=3);
        directory=new File(directory,(probe?"production":"regression")+"/round-"+round);
        assertTrue(directory.isDirectory() || directory.mkdirs());
        for (String service:new String[]{"input_method","window windows"}) {
            String name=service.startsWith("window")?"windows":"input-method";
            java.nio.file.Files.write(new File(directory,phase+"-"+name+".txt").toPath(),
                shell("dumpsys "+service).getBytes(StandardCharsets.UTF_8));
        }
        android.graphics.Bitmap screenshot=InstrumentationRegistry.getInstrumentation().getUiAutomation().takeScreenshot();
        assertNotNull("Could not capture the actual keyboard surface",screenshot);
        try (OutputStream output=new FileOutputStream(new File(directory,phase+".png"))) {
            assertTrue(screenshot.compress(android.graphics.Bitmap.CompressFormat.PNG,100,output));
        } finally {screenshot.recycle();}
        JSONObject androidState=new JSONObject();
        MainActivity activity=resumedActivity();
        if (activity!=null) InstrumentationRegistry.getInstrumentation().runOnMainSync(()-> {
            try {
                android.view.View decor=activity.getWindow().getDecorView();
                android.view.View editor=inputEditor(decor);
                android.graphics.Rect visibleFrame=new android.graphics.Rect();
                decor.getWindowVisibleDisplayFrame(visibleFrame);
                androidState.put("api",Build.VERSION.SDK_INT)
                    .put("window_focus",activity.hasWindowFocus())
                    .put("editor_focus",editor!=null && editor.hasFocus())
                    .put("editor_active",editor!=null && activity.getSystemService(
                        android.view.inputmethod.InputMethodManager.class).isActive(editor))
                    .put("ime_occlusion_bottom",editor!=null ? imeBottom(editor) : 0)
                    .put("decor_height",decor.getHeight())
                    .put("visible_frame_bottom",visibleFrame.bottom)
                    .put("soft_input_mode",activity.getWindow().getAttributes().softInputMode)
                    .put("animator_duration_scale",android.provider.Settings.Global.getFloat(
                        activity.getContentResolver(),android.provider.Settings.Global.ANIMATOR_DURATION_SCALE,1));
                if (Build.VERSION.SDK_INT>=30 && editor!=null && editor.getRootWindowInsets()!=null) {
                    android.view.WindowInsets insets=editor.getRootWindowInsets();
                    androidState.put("ime_visible",insets.isVisible(android.view.WindowInsets.Type.ime()))
                        .put("ime_bottom",insets.getInsets(android.view.WindowInsets.Type.ime()).bottom);
                }
            } catch (JSONException error) {throw new IllegalStateException(error);}
        });
        JSONObject state=new JSONObject().put("slint",MainActivity.diagnostics()).put("android",androidState);
        java.nio.file.Files.write(new File(directory,phase+"-state.json").toPath(),
            state.toString(2).getBytes(StandardCharsets.UTF_8));
        System.out.println("IME "+phase+": "+state);
    }
    @Test public void subscriptionKeyboardProbeCapturesBothFields() throws Exception {
        Context context=InstrumentationRegistry.getInstrumentation().getTargetContext();
        shell("settings put system font_scale 1.0");
        context.startActivity(new Intent(context,MainActivity.class).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK));
        try {
            await("Keyboard probe Activity did not receive focus",()->inStage(androidx.test.runner.lifecycle.Stage.RESUMED,true));
            await("Keyboard probe core did not start",CoreBridge::ready);
            dismissErrorDialogIfPresent();
            tab(2,2);
            await("Keyboard probe add control has no layout",()-> {
                JSONArray target=MainActivity.diagnostics().optJSONArray("profile_add_target");
                return target!=null && target.length()==4 && target.optDouble(2)>0 && target.optDouble(3)>0;
            });
            tapTarget("profile_add_target");
            await("Keyboard probe editor did not open",()->MainActivity.diagnostics().optInt("detail")==3);
            dismissErrorDialogIfPresent();
            captureIme("before-focus");
            for (String field:new String[]{"name","source"}) {
                tapTarget("profile_"+field+"_target");
                await("Keyboard probe "+field+" did not gain focus",()-> {
                    if (MainActivity.diagnostics().optBoolean("profile_"+field+"_focused")) return true;
                    tapTarget("profile_"+field+"_target");
                    SystemClock.sleep(250);
                    return MainActivity.diagnostics().optBoolean("profile_"+field+"_focused");
                });
                captureIme(field+"-after-tap");
                SystemClock.sleep(1500);
                captureIme(field+"-settled");
            }
            shell("input keyevent KEYCODE_BACK");
            SystemClock.sleep(500);
            captureIme("after-back");
            if (MainActivity.diagnostics().optInt("detail")==3) {
                tapTarget("profile_name_target");
                SystemClock.sleep(1500);
                captureIme("reopened");
            }
            System.out.println("Visual probe only: inspect screenshots; no keyboard-rendering success is asserted.");
        } catch (Throwable error) {
            try {captureIme("failure");} catch (Throwable snapshot) {error.addSuppressed(snapshot);}
            throw error;
        }
    }
    private static void verifyRestoredImeAndHide(String orientation) throws Exception {
        java.util.concurrent.atomic.AtomicReference<String> previous=new java.util.concurrent.atomic.AtomicReference<>();
        java.util.concurrent.atomic.AtomicLong stableSince=new java.util.concurrent.atomic.AtomicLong();
        await("Restored "+orientation+" keyboard state did not settle",()-> {
            JSONObject ui=MainActivity.diagnostics();
            if (!ui.optBoolean("live")) return false;
            String state=imeVisible()+"/"+ui.getDouble("keyboard_inset")+"/"
                +ui.getBoolean("profile_name_focused")+"/"+ui.getBoolean("profile_source_focused");
            long now=SystemClock.uptimeMillis();
            if (!state.equals(previous.getAndSet(state))) stableSince.set(now);
            return now-stableSince.get()>=500;
        });
        if (imeVisible()) {
            await("Restored "+orientation+" Slint keyboard inset did not synchronize",
                ()->MainActivity.diagnostics().optDouble("keyboard_inset")>0);
            JSONObject ui=MainActivity.diagnostics();
            String field=ui.optBoolean("profile_source_focused")?"source":"name";
            assertTrue("Restored keyboard has no focused subscription field",
                ui.optBoolean("profile_"+field+"_focused"));
            assertImeLayout(field);
            captureIme(orientation+"-restored-visible");
            shell("input keyevent KEYCODE_BACK");
        }
        await("Restored "+orientation+" IME did not hide after Back",()->!imeVisible()
            && MainActivity.diagnostics().optDouble("keyboard_inset")==0);
        assertEquals("Navigation bars must not become a keyboard inset after rotation",
            0,MainActivity.diagnostics().getDouble("keyboard_inset"),0);
        captureIme(orientation+"-hidden");
        assertFalse("The keyboard reopened while capturing "+orientation,imeVisible());
        assertEquals("Slint keyboard inset changed after hiding "+orientation,
            0,MainActivity.diagnostics().getDouble("keyboard_inset"),0);
    }
    @Test public void subscriptionFieldsShowImeAcceptTextAndReopenAfterBack() throws Exception {
        Context context=InstrumentationRegistry.getInstrumentation().getTargetContext();
        shell("settings put system font_scale 1.0");
        context.startActivity(new Intent(context,MainActivity.class).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK));
        try {
            await("Input test Activity did not receive focus",()->inStage(androidx.test.runner.lifecycle.Stage.RESUMED,true));
            waitForAppWindow();
            await("Input test core did not start",CoreBridge::ready);
            dismissErrorDialogIfPresent();
            tab(2,2);
            await("Add subscription control has no layout",()-> {
                JSONArray target=MainActivity.diagnostics().optJSONArray("profile_add_target");
                return target!=null && target.length()==4 && target.optDouble(2)>0 && target.optDouble(3)>0;
            });
            tapTarget("profile_add_target");
            await("Add subscription did not open",()->MainActivity.diagnostics().optInt("detail")==3);
            dismissErrorDialogIfPresent();
            captureIme("before-focus");
            int draftNameLength=MainActivity.diagnostics().getInt("profile_name_length");
            int draftSourceLength=MainActivity.diagnostics().getInt("profile_source_length");
            MainActivity.emit("error","检查更新失败：测试网络不可用");
            await("The background error notice did not appear",()-> {
                JSONObject ui=MainActivity.diagnostics();
                JSONArray target=ui.optJSONArray("notice_close_target");
                return ui.optBoolean("notice_active") && ui.optBoolean("notice_error")
                    && target!=null && target.length()==4 && target.optDouble(2)>0 && target.optDouble(3)>0;
            });
            captureIme("notice-before-close");
            assertFalse("A recoverable update error must not open a modal",
                MainActivity.diagnostics().optBoolean("notice_modal"));
            tapTarget("notice_close_target");
            await("The error notice did not close",()-> {
                JSONObject ui=MainActivity.diagnostics();
                return ui.optBoolean("live") && !ui.optBoolean("notice_active");
            });
            assertEquals("Closing the notice must keep the subscription editor open",
                3,MainActivity.diagnostics().getInt("detail"));
            assertEquals(draftNameLength,MainActivity.diagnostics().getInt("profile_name_length"));
            assertEquals(draftSourceLength,MainActivity.diagnostics().getInt("profile_source_length"));
            MainActivity.emit("critical","内核启动失败：测试权限不足。\n原订阅和输入内容均已保留。");
            await("The critical error dialog did not appear",()->MainActivity.diagnostics().optBoolean("notice_modal"));
            shell("input keyevent KEYCODE_BACK");
            await("Android Back did not dismiss the error dialog while keeping the Activity active",()-> {
                JSONObject ui=MainActivity.diagnostics();
                return ui.optBoolean("live") && !ui.optBoolean("notice_active")
                    && inStage(androidx.test.runner.lifecycle.Stage.RESUMED,true);
            });
            assertEquals("Back must preserve the editor under the error dialog",
                3,MainActivity.diagnostics().getInt("detail"));
            assertEquals(draftNameLength,MainActivity.diagnostics().getInt("profile_name_length"));
            assertEquals(draftSourceLength,MainActivity.diagnostics().getInt("profile_source_length"));
            int iteration=0;
            for (String field:new String[]{"name","source","name","source","name"}) {
                tapVisibleProfileField(field);
                await("Subscription "+field+" did not gain Slint focus",
                    ()->MainActivity.diagnostics().optBoolean("profile_"+field+"_focused"));
                await("Focused subscription "+field+" did not show the actual Android IME",ProxyTunTest::imeVisible);
                await("Slint did not receive the keyboard inset",()->MainActivity.diagnostics().optDouble("keyboard_inset")>0);
                String text=field.equals("name")?"ImeProbe":"https://example.invalid/ime";
                commitImeText(field,text);
                await("IME text did not reach the subscription "+field,()->MainActivity.diagnostics()
                    .optInt("profile_"+field+"_length",-1)==text.length());
                SystemClock.sleep(1500);
                assertEquals("Committed text changed after the input connection settled",
                    text.length(),MainActivity.diagnostics().getInt("profile_"+field+"_length"));
                assertTrue("Android IME disappeared after switching to "+field,imeVisible());
                assertTrue("Slint lost the keyboard inset after switching to "+field,
                    MainActivity.diagnostics().optDouble("keyboard_inset")>0);
                assertImeLayout(field);
                captureIme((iteration++)+"-"+field+"-visible");
                if (iteration<3) continue;
                shell("input keyevent KEYCODE_BACK");
                await("Android IME did not hide after Back",()->!imeVisible()
                    && MainActivity.diagnostics().optDouble("keyboard_inset")==0);
                assertEquals("Back should hide the IME and keep the editor open",3,MainActivity.diagnostics().getInt("detail"));
                captureIme((iteration-1)+"-"+field+"-hidden");
            }
            shell("am start -W -a android.intent.action.MAIN -c android.intent.category.HOME");
            await("Input test Activity did not enter background",()->inStage(androidx.test.runner.lifecycle.Stage.STOPPED,false));
            shell("am start -W -n org.clashofrust.android/.MainActivity");
            await("Input test Activity did not resume",()->inStage(androidx.test.runner.lifecycle.Stage.RESUMED,true));
            waitForAppWindow();
            captureIme("before-resumed-tap");
            verifyRestoredImeAndHide("foreground");
            tapVisibleProfileField("source");
            await("Resumed subscription source did not receive touch focus",
                ()->MainActivity.diagnostics().optBoolean("profile_source_focused"));
            await("Resumed subscription editor did not show the IME",ProxyTunTest::imeVisible);
            await("Resumed Slint editor did not receive the keyboard inset",
                ()->MainActivity.diagnostics().optDouble("keyboard_inset")>0);
            SystemClock.sleep(1500);
            assertImeLayout("source");
            captureIme("after-resume");
            shell("input keyevent KEYCODE_BACK");
            await("Resumed IME did not hide",()->!imeVisible()
                && MainActivity.diagnostics().optDouble("keyboard_inset")==0);
            android.app.UiAutomation automation=InstrumentationRegistry.getInstrumentation().getUiAutomation();
            try {
                assertTrue("Window manager rejected landscape rotation",
                    automation.setRotation(android.app.UiAutomation.ROTATION_FREEZE_90));
                await("Input surface did not enter landscape",()-> {
                    JSONObject ui=MainActivity.diagnostics();
                    return ui.optDouble("width")>ui.optDouble("height");
                });
                SystemClock.sleep(1500);
                verifyRestoredImeAndHide("landscape");
                for (String field:new String[]{"name","source"}) {
                    tapVisibleProfileField(field);
                    await("Landscape subscription "+field+" did not gain focus",
                        ()->MainActivity.diagnostics().optBoolean("profile_"+field+"_focused"));
                    await("Landscape editor did not show the IME",ProxyTunTest::imeVisible);
                    await("Landscape Slint editor did not receive the keyboard inset",
                        ()->MainActivity.diagnostics().optDouble("keyboard_inset")>0);
                    SystemClock.sleep(1500);
                    assertImeLayout(field);
                    captureIme("landscape-"+field+"-visible");
                }
                shell("input keyevent KEYCODE_BACK");
                await("Landscape keyboard inset did not clear",()->!imeVisible()
                    && MainActivity.diagnostics().optDouble("keyboard_inset")==0);
            } catch (Exception | AssertionError error) {
                try {captureIme("landscape-failure");}
                catch (Exception | AssertionError snapshot) {error.addSuppressed(snapshot);}
                throw error;
            } finally {
                automation.setRotation(android.app.UiAutomation.ROTATION_FREEZE_0);
                try {
                    await("Input surface did not return to portrait",()-> {
                        JSONObject ui=MainActivity.diagnostics();
                        return ui.optDouble("height")>ui.optDouble("width");
                    });
                } finally {automation.setRotation(android.app.UiAutomation.ROTATION_UNFREEZE);}
            }
            SystemClock.sleep(1500);
            verifyRestoredImeAndHide("portrait");
            captureIme("final");
        } catch (Exception | AssertionError error) {
            try {captureIme("failure");}
            catch (Exception | AssertionError snapshot) {error.addSuppressed(snapshot);}
            throw error;
        }
    }
    private interface Condition { boolean ready() throws Exception; }
    private static boolean vpnActive(ConnectivityManager connectivity) {
        NetworkCapabilities capabilities=connectivity.getNetworkCapabilities(connectivity.getActiveNetwork());
        return capabilities!=null && capabilities.hasTransport(NetworkCapabilities.TRANSPORT_VPN);
    }
    private static void await(String message, Condition condition) throws Exception {
        long deadline = System.nanoTime() + 30_000_000_000L;
        while (System.nanoTime() < deadline) {
            if (condition.ready()) return;
            Thread.sleep(100);
        }
        fail(message+"; actual UI: "+MainActivity.diagnostics());
    }
    private static String value(String yaml, String name) {
        for (String line : yaml.split("\n")) {
            if (line.startsWith(name + ":")) return line.substring(name.length()+1).trim().replace("\"", "").replace("'", "");
        }
        throw new IllegalStateException("Missing " + name);
    }
    private static String read(URL url, java.net.Proxy proxy, String secret) throws Exception {
        HttpURLConnection connection = (HttpURLConnection)url.openConnection(proxy);
        connection.setConnectTimeout(5000); connection.setReadTimeout(5000);
        connection.setUseCaches(false);
        connection.setRequestProperty("Cache-Control", "no-store");
        connection.setRequestProperty("Connection", "close");
        if (secret != null) connection.setRequestProperty("Authorization", "Bearer " + secret);
        try (InputStream in = connection.getInputStream()) {
            ByteArrayOutputStream out = new ByteArrayOutputStream();
            byte[] buffer = new byte[4096]; int n;
            while ((n=in.read(buffer))!=-1) out.write(buffer,0,n);
            return out.toString(StandardCharsets.UTF_8.name());
        } finally { connection.disconnect(); }
    }
    private static void noTunRoute() throws Exception {
        try (Socket connection=new Socket(java.net.Proxy.NO_PROXY)) {
            try {connection.connect(new InetSocketAddress("198.51.100.1",18081),1000);}
            catch (IOException blocked) {return;}
            fail("Reserved test origin was reachable without VPN; negative control is invalid");
        }
    }
    private static java.util.List<android.view.accessibility.AccessibilityNodeInfo> dialogViews(
        android.view.accessibility.AccessibilityNodeInfo root, String id) {
        try {
            java.util.List<android.view.accessibility.AccessibilityNodeInfo> views =
                root.findAccessibilityNodeInfosByViewId(id);
            return views == null ? java.util.Collections.emptyList() : views;
        } catch (NullPointerException staleWindow) {
            // The framework can lose its node list while the system dialog is
            // being replaced. The bounded consent loop reacquires its root.
            android.util.Log.w("ClashOfRustTest", "VPN dialog accessibility window changed", staleWindow);
            return java.util.Collections.emptyList();
        }
    }
    @SuppressWarnings("deprecation")
    private static boolean checked(android.view.accessibility.AccessibilityNodeInfo node) {
        if (Build.VERSION.SDK_INT >= 36)
            return node.getChecked() == android.view.accessibility.AccessibilityNodeInfo.CHECKED_STATE_TRUE;
        return node.isChecked();
    }
    private static void logConsentNode(android.view.accessibility.AccessibilityNodeInfo node,
            int depth, StringBuilder details) {
        if (node==null || depth>5 || details.length()>4000) return;
        details.append('\n').append(depth).append(' ')
            .append(node.getPackageName()).append(' ').append(node.getClassName()).append(' ')
            .append(node.getViewIdResourceName()).append(" enabled=").append(node.isEnabled())
            .append(" clickable=").append(node.isClickable());
        for (int index=0;index<node.getChildCount();index++)
            logConsentNode(node.getChild(index),depth+1,details);
    }
    private static android.view.accessibility.AccessibilityNodeInfo consentRoot(android.app.UiAutomation automation) {
        android.view.accessibility.AccessibilityNodeInfo active=automation.getRootInActiveWindow();
        if (active!=null && "com.android.vpndialogs".contentEquals(active.getPackageName())) return active;
        for (android.view.accessibility.AccessibilityWindowInfo window:automation.getWindows()) {
            if (!window.isActive() && !window.isFocused()) continue;
            android.view.accessibility.AccessibilityNodeInfo root=window.getRoot();
            if (root!=null && "com.android.vpndialogs".contentEquals(root.getPackageName())) return root;
        }
        return null;
    }
    private static void consent(boolean allow) throws Exception {
        android.app.UiAutomation automation=InstrumentationRegistry.getInstrumentation().getUiAutomation();
        android.accessibilityservice.AccessibilityServiceInfo service=automation.getServiceInfo();
        if (service==null) throw new IOException("Android accessibility automation is unavailable");
        int originalFlags=service.flags;
        service.flags |= android.accessibilityservice.AccessibilityServiceInfo.FLAG_REPORT_VIEW_IDS
            | android.accessibilityservice.AccessibilityServiceInfo.FLAG_RETRIEVE_INTERACTIVE_WINDOWS;
        automation.setServiceInfo(service);
        java.util.concurrent.atomic.AtomicBoolean attempted=new java.util.concurrent.atomic.AtomicBoolean();
        try {
            await("Android VPN consent did not return to the app",()-> {
                if (attempted.get() && inStage(androidx.test.runner.lifecycle.Stage.RESUMED,true))
                    return true;
                android.view.accessibility.AccessibilityNodeInfo root=consentRoot(automation);
                if (root==null) return false;
                for (android.view.accessibility.AccessibilityNodeInfo check:dialogViews(root,"android:id/checkbox"))
                    if (check.isCheckable() && !checked(check))
                        check.performAction(android.view.accessibility.AccessibilityNodeInfo.ACTION_CLICK);
                for (android.view.accessibility.AccessibilityNodeInfo button:dialogViews(root,
                    allow ? "android:id/button1" : "android:id/button2")) {
                    if (button.isEnabled()) {
                        attempted.set(true);
                        button.performAction(android.view.accessibility.AccessibilityNodeInfo.ACTION_CLICK);
                    }
                }
                return false;
            });
        } catch (AssertionError error) {
            try {
                StringBuilder details=new StringBuilder("VPN consent accessibility tree; attempted=")
                    .append(attempted.get());
                logConsentNode(automation.getRootInActiveWindow(),0,details);
                for (android.view.accessibility.AccessibilityWindowInfo window:automation.getWindows()) {
                    details.append("\nwindow active=").append(window.isActive())
                        .append(" focused=").append(window.isFocused());
                    logConsentNode(window.getRoot(),0,details);
                }
                android.util.Log.w("ClashOfRustTest",details.toString());
            } catch (RuntimeException failure) {
                error.addSuppressed(failure);
            }
            throw error;
        } finally {
            service.flags=originalFlags;
            automation.setServiceInfo(service);
        }
    }
    private static void authorization(long request,String status) throws Exception {
        await("VPN authorization did not reach "+status,()-> {
            JSONObject state=MainActivity.authorizationState();
            return state.optLong("request")==request && status.equals(state.optString("status"));
        });
    }
    private static void verifyFirstVpnConsent(Context context,String original) throws Exception {
        java.util.concurrent.atomic.AtomicReference<MainActivity> owner=new java.util.concurrent.atomic.AtomicReference<>();
        InstrumentationRegistry.getInstrumentation().runOnMainSync(()-> {
            for (android.app.Activity activity:androidx.test.runner.lifecycle.ActivityLifecycleMonitorRegistry.getInstance()
                .getActivitiesInStage(androidx.test.runner.lifecycle.Stage.RESUMED))
                if (activity instanceof MainActivity) owner.set((MainActivity)activity);
        });
        MainActivity activity=owner.get();assertNotNull(activity);
        assertNotNull("Test must start without pre-granted VPN consent",VpnService.prepare(context));
        activity.request("{\"op\":\"authorize-vpn\",\"request\":101}");
        authorization(101,"pending");
        assertEquals("DNS changed while waiting for consent",original,CoreBridge.config());
        assertFalse(TunnelService.connected());
        consent(false);authorization(101,"denied");
        assertNotNull(VpnService.prepare(context));
        assertEquals(original,CoreBridge.config());

        activity.request("{\"op\":\"authorize-vpn\",\"request\":102}");
        authorization(102,"pending");
        activity.request("{\"op\":\"cancel-vpn-authorization\",\"request\":102}");
        authorization(102,"cancelled");
        // A late Allow after timeout/cancellation must not start the VPN.
        consent(true);
        await("Late VPN permission was not delivered",()->VpnService.prepare(context)==null
            && inStage(androidx.test.runner.lifecycle.Stage.RESUMED,true));
        authorization(102,"cancelled");
        assertFalse("Cancelled consent started a VPN",TunnelService.connected());
        assertEquals(original,CoreBridge.config());

        shell("appops set org.clashofrust.android ACTIVATE_VPN default");
        assertNotNull(VpnService.prepare(context));
        activity.request("{\"op\":\"authorize-vpn\",\"request\":103}");
        authorization(103,"pending");consent(true);authorization(103,"granted");
        await("Activity did not resume after consent",()->inStage(androidx.test.runner.lifecycle.Stage.RESUMED,true));
        assertNull(VpnService.prepare(context));
        assertFalse("Consent must wait for configuration before connecting",TunnelService.connected());
        assertEquals(original,CoreBridge.config());
        activity.request("{\"op\":\"authorize-vpn\",\"request\":104}");
        authorization(104,"granted");
        assertFalse(TunnelService.connected());

        // Ordinary VPN consent has a separate path from protection consent.
        // Disconnect must cancel it even before a TunnelService exists.
        shell("appops set org.clashofrust.android ACTIVATE_VPN default");
        assertNotNull(VpnService.prepare(context));
        activity.request("{\"op\":\"connect\"}");
        await("Ordinary VPN consent did not open",()-> {
            android.view.accessibility.AccessibilityNodeInfo root=InstrumentationRegistry.getInstrumentation()
                .getUiAutomation().getRootInActiveWindow();
            return root!=null && "com.android.vpndialogs".contentEquals(root.getPackageName());
        });
        activity.request("{\"op\":\"disconnect\"}");
        consent(true);
        await("Cancelled ordinary consent did not return",()->VpnService.prepare(context)==null
            && inStage(androidx.test.runner.lifecycle.Stage.RESUMED,true));
        assertStaysDisconnected(context,"Late ordinary consent reopened the VPN");
        assertEquals(original,CoreBridge.config());

        // Model a service start that was queued before the user disconnected.
        String stale=TunnelService.prepareStart(context);
        TunnelService.stop(context);
        context.startForegroundService(new Intent(context,TunnelService.class).putExtra(TunnelService.REQUEST,stale));
        assertStaysDisconnected(context,"A stale service start reopened the VPN");

        // Hold the lifecycle queue so Disconnect definitely arrives before a
        // previously accepted connect begins. Releasing it must not create TUN.
        java.lang.reflect.Field field=TunnelService.class.getDeclaredField("serial");
        field.setAccessible(true);
        java.util.concurrent.ExecutorService serial=(java.util.concurrent.ExecutorService)field.get(null);
        java.util.concurrent.CountDownLatch entered=new java.util.concurrent.CountDownLatch(1);
        java.util.concurrent.CountDownLatch release=new java.util.concurrent.CountDownLatch(1);
        long generation=TunnelService.generation();
        serial.execute(()-> {
            entered.countDown();
            try {release.await(10,java.util.concurrent.TimeUnit.SECONDS);}
            catch (InterruptedException error) {Thread.currentThread().interrupt();}
        });
        try {
            assertTrue(entered.await(5,java.util.concurrent.TimeUnit.SECONDS));
            TunnelService.start(context,false);
            await("Queued VPN start was not accepted",TunnelService::pending);
            await("Slint did not expose cancellation for the pending VPN",()->MainActivity.diagnostics().optBoolean("vpn_pending"));
            TunnelService.stop(context);
        } finally {release.countDown();}
        assertStaysDisconnected(context,"A cancelled lifecycle job reopened the VPN");
        assertEquals("A cancelled lifecycle job established a TUN",generation,TunnelService.generation());
    }
    private static void assertStaysDisconnected(Context context,String message) throws Exception {
        long deadline=System.nanoTime()+java.util.concurrent.TimeUnit.SECONDS.toNanos(2);
        do {
            assertFalse(message,TunnelService.connected());
            assertFalse(message,context.getSharedPreferences("vpn",Context.MODE_PRIVATE).getBoolean("desired",false));
            assertFalse(message,vpnActive(context.getSystemService(ConnectivityManager.class)));
            SystemClock.sleep(50);
        } while (System.nanoTime()<deadline);
    }
    private static void tunCarriesSocket(String controller,String secret) throws Exception {
        tunCarriesSocket(controller,secret,"198.51.100.1",false,"CI-TUN-origin");
    }
    private static void tunCarriesSocket(String controller,String secret,String destination,boolean verifyOwner,String expectedNode) throws Exception {
        // NetworkCallbacks describe the control plane. A just-replaced VPN can
        // still be waiting for netd's UID routes; require fresh real traffic,
        // with a bounded deadline, rather than sleeping or rerunning the test.
        long deadline=SystemClock.elapsedRealtime()+15000;
        IOException last=null;
        ConnectivityManager manager=InstrumentationRegistry.getInstrumentation().getTargetContext()
            .getSystemService(ConnectivityManager.class);
        String before=interfaceCounters(manager);
        do {
            try { assertTunSocket(controller,secret,destination,verifyOwner,expectedNode); return; }
            catch (IOException transition) { last=transition; }
            SystemClock.sleep(100);
        } while (SystemClock.elapsedRealtime()<deadline);
        throw new IOException("VPN data path did not become ready; before="+before+"; "+networkState(),last);
    }
    private static void tunResolvesAndCarriesHttp() throws Exception {
        long deadline=SystemClock.elapsedRealtime()+15000;
        IOException last=null;
        do {
            try {
                assertEquals("cor-android-network-ok",read(
                    URI.create("http://android-tun.test:18081/probe").toURL(),java.net.Proxy.NO_PROXY,null));
                return;
            } catch (IOException transition) {last=transition;}
            SystemClock.sleep(100);
        } while (SystemClock.elapsedRealtime()<deadline);
        throw new IOException("VPN hostname traffic did not become ready; "+networkState(),last);
    }
    private static String interfaceCounters(ConnectivityManager manager) {
        LinkProperties properties=manager.getLinkProperties(manager.getActiveNetwork());
        String device=properties==null ? null : properties.getInterfaceName();
        StringBuilder details=new StringBuilder();
        if (device!=null && device.matches("[A-Za-z0-9_.-]+")) {
            for (String counter:new String[]{"rx_packets","tx_packets","rx_dropped","tx_dropped"}) {
                try {
                    java.nio.file.Path file=java.nio.file.Paths.get("/sys/class/net",device,"statistics",counter);
                    details.append('\n').append(device).append(' ').append(counter).append('=')
                        .append(new String(java.nio.file.Files.readAllBytes(file),StandardCharsets.US_ASCII).trim());
                } catch (IOException | SecurityException unavailable) {
                    details.append('\n').append(device).append(' ').append(counter).append(" unavailable");
                }
            }
        }
        return details.toString();
    }
    private static String networkState() {
        ConnectivityManager manager=InstrumentationRegistry.getInstrumentation().getTargetContext()
            .getSystemService(ConnectivityManager.class);
        Network current=manager.getActiveNetwork();
        StringBuilder details=new StringBuilder("active="+current+"; capabilities="+manager.getNetworkCapabilities(current)
            +"; links="+manager.getLinkProperties(current)+"; VPN="+TunnelService.connected()
            +"; generation="+TunnelService.generation()+"; error="+TunnelService.error())
            .append(interfaceCounters(manager));
        // Capture the data plane too: a VPN capability alone cannot establish
        // whether packets reached the device or its native reader still owns it.
        for (String command:new String[]{"ip -s link show","ip rule show","ip route show table all"}) {
            try {details.append('\n').append(command).append('\n').append(shell(command));}
            catch (Exception error) {details.append(" diagnostics unavailable: ").append(error);}
        }
        try (java.nio.file.DirectoryStream<java.nio.file.Path> files=java.nio.file.Files.newDirectoryStream(
                java.nio.file.Paths.get("/proc/self/fdinfo"))) {
            for (java.nio.file.Path file:files) {
                try {
                    String info=new String(java.nio.file.Files.readAllBytes(file),StandardCharsets.UTF_8);
                    if (info.contains("iff:")) details.append("\nTUN fd ").append(file.getFileName()).append('\n').append(info);
                } catch (IOException closed) { /* Descriptors can retire during enumeration. */ }
            }
        } catch (IOException error) {details.append(" fd diagnostics unavailable: ").append(error);}
        return details.toString();
    }
    private static void awaitDirectRecovery(ConnectivityManager manager) throws Exception {
        long deadline=SystemClock.elapsedRealtime()+15000;
        IOException last=null;
        do {
            Network network=manager.getActiveNetwork();
            NetworkCapabilities caps=manager.getNetworkCapabilities(network);
            if (!TunnelService.connected() && caps!=null && !caps.hasTransport(NetworkCapabilities.TRANSPORT_VPN)) {
                try {
                    // Never bind a Network explicitly: that could hide a broken
                    // default route. Open a new default-network connection.
                    assertEquals("cor-android-network-ok",read(URI.create("http://10.0.2.2:18081/probe").toURL(),java.net.Proxy.NO_PROXY,null));
                    if (network.equals(manager.getActiveNetwork())) { noTunRoute(); return; }
                } catch (IOException transition) { last=transition; }
            }
            SystemClock.sleep(100);
        } while (SystemClock.elapsedRealtime()<deadline);
        throw new IOException("Default network did not recover after VPN disconnect; "+networkState(),last);
    }
    private static void assertTunSocket(String controller,String secret,String destination,boolean verifyOwner,String expectedNode) throws Exception {
        // The physical network cannot reach this reserved address. Only the
        // test core's explicit HTTP outbound can relay it to the host origin.
        // Keep a fresh socket open and require its exact TUN source port too.
        try (Socket connection=new Socket(java.net.Proxy.NO_PROXY)) {
            connection.connect(new InetSocketAddress(destination,18081),5000);
            connection.setSoTimeout(5000);
            connection.getOutputStream().write(("GET /hold HTTP/1.1\r\nHost: 198.51.100.1\r\nConnection: close\r\nContent-Length: 1\r\n\r\n")
                .getBytes(StandardCharsets.US_ASCII));
            assertHeldResponse(connection.getInputStream());
            connection.setSoTimeout(1);
            long deadline=SystemClock.elapsedRealtime()+5000;
            JSONObject records;
            do {
                records=new JSONObject(read(new URL("http://"+controller+"/connections"),java.net.Proxy.NO_PROXY,secret));
                JSONArray connections=records.getJSONArray("connections");
                for (int index=0;index<connections.length();index++) {
                    JSONObject record=connections.getJSONObject(index);
                    JSONObject metadata=record.getJSONObject("metadata");
                    JSONArray chains=record.getJSONArray("chains");
                    for (int chain=0;chain<chains.length();chain++)
                        if (metadata.optString("type").equalsIgnoreCase("Tun")
                            && metadata.optInt("sourcePort")==connection.getLocalPort()
                            && metadata.optInt("destinationPort")==18081
                            && (!verifyOwner || metadata.optInt("uid")==android.os.Process.myUid())
                            && expectedNode.equals(chains.optString(chain))) {
                            connection.getOutputStream().write('!');
                            return;
                        }
                }
                assertHeldConnectionPending(connection.getInputStream());
                SystemClock.sleep(100);
            } while (SystemClock.elapsedRealtime()<deadline);
            fail("No matching TUN connection: "+connection.getLocalSocketAddress()+" -> "+connection.getRemoteSocketAddress()
                +"; a direct-only test must fail; records="+records);
        }
    }
    private static void assertHeldResponse(InputStream stream) throws IOException {
        ByteArrayOutputStream header=new ByteArrayOutputStream();
        int ending=0;
        while (ending<4 && header.size()<8192) {
            int value=stream.read();
            if (value<0) throw new EOFException("Held HTTP headers were truncated");
            header.write(value);
            int expected=ending%2==0 ? '\r' : '\n';
            ending=value==expected ? ending+1 : value=='\r' ? 1 : 0;
        }
        assertEquals("Held HTTP headers exceed the size limit",4,ending);
        String status=header.toString(StandardCharsets.US_ASCII.name()).split("\r\n",2)[0];
        assertTrue("Held TUN request failed: "+status,status.contains(" 200 "));
        byte[] expected="cor-android-network-ok".getBytes(StandardCharsets.US_ASCII);
        for (byte value:expected) {
            int actual=stream.read();
            if (actual<0) throw new EOFException("Held HTTP response body was truncated");
            assertEquals("Held HTTP response returned the wrong body",value&255,actual);
        }
    }
    private static void assertHeldConnectionPending(InputStream stream) throws IOException {
        try {
            if (stream.read()<0) throw new EOFException("Held TUN connection closed before acknowledgement");
            fail("Held TUN response completed before controller verification");
        } catch (java.net.SocketTimeoutException pending) {
            return;
        }
    }
    @Test public void heldResponseClassifiesTruncationAsTransportFailure() throws Exception {
        for (String raw:new String[]{"", "HTTP/1.1 200 OK\r\nX-Test: 1\r\n", "HTTP/1.1 200 OK\r\n\r\n"}) {
            try {
                assertHeldResponse(new ByteArrayInputStream(raw.getBytes(StandardCharsets.US_ASCII)));
                fail("Truncated HTTP response must fail");
            } catch (EOFException expected) { /* The existing readiness loop may open a fresh connection. */ }
        }
        AssertionError rejected=null;
        try {assertHeldResponse(new ByteArrayInputStream("HTTP/1.1 403 Forbidden\r\n\r\nx".getBytes(StandardCharsets.US_ASCII)));}
        catch (AssertionError error) {rejected=error;}
        assertNotNull("An HTTP error must fail without a readiness retry",rejected);
        assertHeldResponse(new ByteArrayInputStream("HTTP/1.1 200 OK\r\nX-Test: 1\r\n\r\ncor-android-network-ok".getBytes(StandardCharsets.US_ASCII)));
        try {
            assertHeldConnectionPending(new ByteArrayInputStream(new byte[0]));
            fail("Closed probes must open a fresh connection within the readiness deadline");
        } catch (EOFException expected) {
            assertNotNull(expected);
        }
        assertHeldConnectionPending(new InputStream() {
            @Override public int read() throws IOException { throw new java.net.SocketTimeoutException(); }
        });
        AssertionError premature=null;
        try { assertHeldConnectionPending(new ByteArrayInputStream(new byte[]{'!'})); }
        catch (AssertionError error) { premature=error; }
        assertNotNull("Premature completion must not be retried as a transport transition",premature);
    }
    private static JSONArray previewBounds(JSONObject ui, String name) {
        JSONObject targets = ui.optJSONObject("targets");
        return targets == null ? null : targets.optJSONArray(name);
    }
    private static boolean tryPreviewTarget(String name) throws Exception {
        if (!inStage(androidx.test.runner.lifecycle.Stage.RESUMED, true)) return false;
        dismissErrorDialogIfPresent(false);
        JSONObject ui = MainActivity.diagnostics();
        JSONArray bounds = previewBounds(ui, name);
        if (!ui.optBoolean("live") || bounds == null || bounds.length() != 4
                || bounds.optDouble(2) <= 0 || bounds.optDouble(3) <= 0) return false;
        double x = bounds.getDouble(0) + bounds.getDouble(2) / 2;
        double y = bounds.getDouble(1) + bounds.getDouble(3) / 2;
        if (x < 0 || y < 0 || x >= ui.getDouble("width") || y >= ui.getDouble("height")) return false;
        float scale = (float)ui.getDouble("scale");
        MainActivity activity = resumedActivity();
        if (activity == null) return false;
        int[] origin = new int[2];
        java.util.concurrent.atomic.AtomicBoolean ready = new java.util.concurrent.atomic.AtomicBoolean();
        InstrumentationRegistry.getInstrumentation().runOnMainSync(() -> {
            android.view.View decor = activity.getWindow().getDecorView();
            if (activity.hasWindowFocus() && decor.isAttachedToWindow() && decor.isShown()
                    && Math.abs(decor.getWidth() - ui.optDouble("width") * scale) <= 2
                    && Math.abs(decor.getHeight() - ui.optDouble("height") * scale) <= 2) {
                decor.getLocationOnScreen(origin);
                ready.set(true);
            }
        });
        return ready.get() && tryTap(origin[0] + (float)x * scale, origin[1] + (float)y * scale);
    }
    private static void previewTarget(String name) throws Exception {
        await("Screenshot control was unavailable: " + name, () -> tryPreviewTarget(name));
    }
    private static void previewScrollableTarget(String name, String viewport) throws Exception {
        for (int attempt = 0; attempt < 12; attempt++) {
            waitForAppWindow();
            JSONObject ui = MainActivity.diagnostics();
            JSONArray target = previewBounds(ui, name);
            JSONArray bounds = previewBounds(ui, viewport);
            if (target == null || bounds == null) {
                SystemClock.sleep(200);
                continue;
            }
            double top = Math.max(0, bounds.getDouble(1));
            double bottom = Math.min(ui.getDouble("height"), bounds.getDouble(1) + bounds.getDouble(3));
            double targetTop = target.getDouble(1), targetBottom = targetTop + target.getDouble(3);
            if (targetTop >= top && targetBottom <= bottom && tryPreviewTarget(name)) return;
            previewSwipe(viewport, targetBottom > bottom);
        }
        fail("Screenshot control could not be scrolled into view: " + name + "; actual UI: " + MainActivity.diagnostics());
    }
    private static void previewSwipe(boolean forward) throws Exception {
        previewSwipe("preview-content", forward);
    }
    private static void previewSwipe(String viewport, boolean forward) throws Exception {
        dismissErrorDialogIfPresent(false);
        JSONObject ui = MainActivity.diagnostics();
        JSONArray bounds = previewBounds(ui, viewport);
        assertNotNull("Screenshot content has no measured bounds", bounds);
        float scale = (float)ui.getDouble("scale");
        int[] origin = windowOrigin();
        float x = origin[0] + (float)(bounds.getDouble(0) + bounds.getDouble(2) * 0.55) * scale;
        float top = origin[1] + (float)(bounds.getDouble(1) + bounds.getDouble(3) * 0.22) * scale;
        float bottom = origin[1] + (float)(bounds.getDouble(1) + bounds.getDouble(3) * 0.78) * scale;
        float start = forward ? bottom : top, end = forward ? top : bottom;
        long down = SystemClock.uptimeMillis();
        pointer(MotionEvent.ACTION_DOWN, x, start, down, down);
        for (int index = 1; index <= 12; index++) {
            SystemClock.sleep(20);
            pointer(MotionEvent.ACTION_MOVE, x, start + (end - start) * index / 12, down, SystemClock.uptimeMillis());
        }
        pointer(MotionEvent.ACTION_UP, x, end, down, SystemClock.uptimeMillis());
        SystemClock.sleep(500);
    }
    private static void previewNavigate(int page) throws Exception {
        await("Could not navigate to screenshot page " + page, () -> {
            JSONObject ui = MainActivity.diagnostics();
            if (ui.optInt("page", -1) == page && ui.optInt("detail", -1) == 0) return true;
            tryPreviewTarget("preview-nav:" + page);
            SystemClock.sleep(350);
            return false;
        });
    }
    private static void previewTool(int page) throws Exception {
        previewNavigate(10);
        for (int attempt = 0; attempt < 5; attempt++) {
            JSONObject ui = MainActivity.diagnostics();
            JSONArray bounds = previewBounds(ui, "preview-tool:" + page);
            JSONArray content = previewBounds(ui, "preview-content");
            if (bounds != null && content != null && bounds.length() == 4 && content.length() == 4) {
                double center = bounds.getDouble(1) + bounds.getDouble(3) / 2;
                if (center >= content.getDouble(1) && center < content.getDouble(1) + content.getDouble(3)) {
                    previewTarget("preview-tool:" + page);
                    await("Tool page did not open: " + page,
                        () -> MainActivity.diagnostics().optInt("page", -1) == page);
                    return;
                }
                previewSwipe(center > content.getDouble(1));
            } else {
                SystemClock.sleep(350);
            }
        }
        fail("Tool is outside the screenshot viewport: " + page);
    }
    private static android.graphics.Bitmap previewWindowBitmap() throws Exception {
        java.util.concurrent.atomic.AtomicReference<MainActivity> focused = new java.util.concurrent.atomic.AtomicReference<>();
        await("Screenshot activity did not regain focus", () -> {
            focused.set(resumedActivity());
            return focused.get() != null;
        });
        MainActivity activity = focused.get();
        java.util.concurrent.atomic.AtomicReference<android.graphics.Bitmap> bitmap =
            new java.util.concurrent.atomic.AtomicReference<>();
        java.util.concurrent.atomic.AtomicReference<RuntimeException> failure =
            new java.util.concurrent.atomic.AtomicReference<>();
        java.util.concurrent.atomic.AtomicInteger result = new java.util.concurrent.atomic.AtomicInteger(-1);
        java.util.concurrent.CountDownLatch completed = new java.util.concurrent.CountDownLatch(1);
        InstrumentationRegistry.getInstrumentation().runOnMainSync(() -> {
            try {
                android.view.View decor = activity.getWindow().getDecorView();
                android.graphics.Bitmap target = android.graphics.Bitmap.createBitmap(
                    decor.getWidth(), decor.getHeight(), android.graphics.Bitmap.Config.ARGB_8888);
                bitmap.set(target);
                android.view.PixelCopy.request(activity.getWindow(), target, status -> {
                    result.set(status);
                    completed.countDown();
                }, new android.os.Handler(android.os.Looper.getMainLooper()));
            } catch (RuntimeException error) {
                failure.set(error);
                completed.countDown();
            }
        });
        assertTrue("Window screenshot timed out", completed.await(10, java.util.concurrent.TimeUnit.SECONDS));
        if (failure.get() != null) throw failure.get();
        assertEquals("Could not copy the Slint window surface", android.view.PixelCopy.SUCCESS, result.get());
        return bitmap.get();
    }
    private static double previewBlackRatio(android.graphics.Bitmap bitmap) {
        int black = 0, sampled = 0;
        int stepX = Math.max(1, bitmap.getWidth() / 64);
        int stepY = Math.max(1, bitmap.getHeight() / 64);
        for (int row = stepY / 2; row < bitmap.getHeight(); row += stepY) {
            for (int column = stepX / 2; column < bitmap.getWidth(); column += stepX) {
                int color = bitmap.getPixel(column, row);
                if (android.graphics.Color.alpha(color) < 128
                        || (android.graphics.Color.red(color) < 8 && android.graphics.Color.green(color) < 8
                            && android.graphics.Color.blue(color) < 8)) black++;
                sampled++;
            }
        }
        return (double)black / sampled;
    }
    private static void capturePreview(File directory, JSONArray cases, String name) throws Exception {
        waitForAppWindow();
        SystemClock.sleep(400);
        JSONObject ui = MainActivity.diagnostics();
        assertTrue("Screenshot is not from the live Slint window: " + name, ui.optBoolean("live"));
        assertFalse("Screenshot preview must not start VPN", TunnelService.connected() || TunnelService.pending());
        int[] origin = windowOrigin();
        ui.put("window_origin_px", new JSONArray().put(origin[0]).put(origin[1]));
        android.graphics.Bitmap screenshot = InstrumentationRegistry.getInstrumentation().getUiAutomation().takeScreenshot();
        assertNotNull("Could not capture " + name, screenshot);
        String screenImage = name + "-screen.png";
        try (OutputStream output = new FileOutputStream(new File(directory, screenImage))) {
            assertTrue("Could not encode " + name, screenshot.compress(android.graphics.Bitmap.CompressFormat.PNG, 100, output));
        } finally { screenshot.recycle(); }
        android.graphics.Bitmap window = previewWindowBitmap();
        int width = window.getWidth(), height = window.getHeight();
        double blackRatio = previewBlackRatio(window);
        try (OutputStream output = new FileOutputStream(new File(directory, name + ".png"))) {
            assertTrue("Could not encode the window " + name,
                window.compress(android.graphics.Bitmap.CompressFormat.PNG, 100, output));
        } finally { window.recycle(); }
        double scale = ui.getDouble("scale");
        assertEquals("Screenshot window width differs from Slint", Math.round(ui.getDouble("width") * scale), width, 2);
        assertEquals("Screenshot window height differs from Slint", Math.round(ui.getDouble("height") * scale), height, 2);
        assertTrue("Screenshot window contains excessive black or transparent pixels: " + blackRatio, blackRatio <= 0.2);
        cases.put(new JSONObject().put("name", name).put("image", name + ".png")
            .put("screen_image", screenImage).put("capture_method", "pixelcopy-window")
            .put("black_sample_ratio", blackRatio)
            .put("pixel_width", width).put("pixel_height", height).put("state", ui));
        JSONObject manifest = new JSONObject().put("schema", 3).put("scope", "android-ui-preview")
            .put("android_api", Build.VERSION.SDK_INT).put("android_release", Build.VERSION.RELEASE)
            .put("cases", cases);
        java.nio.file.Files.write(new File(directory, "manifest.json").toPath(),
            manifest.toString(2).getBytes(StandardCharsets.UTF_8));
    }
    private static void previewSettingsTop() throws Exception {
        for (int attempt = 0; attempt < 6 && MainActivity.diagnostics().optDouble("settings_scroll_y") < -2; attempt++)
            previewSwipe(false);
        assertTrue("Settings did not return to its top",
            MainActivity.diagnostics().optDouble("settings_scroll_y") >= -2);
    }
    private static void previewSettings(File directory, JSONArray cases, String prefix) throws Exception {
        previewNavigate(9);
        previewSettingsTop();
        capturePreview(directory, cases, prefix + "-settings-top");
        previewSwipe(true);
        capturePreview(directory, cases, prefix + "-settings-middle");
        previewSwipe(true);
        capturePreview(directory, cases, prefix + "-settings-bottom");
        previewSettingsTop();
    }
    private static void previewApplicationCatalog(Context context) throws Exception {
        JSONObject catalog = ApplicationCatalog.read(context);
        JSONArray entries = catalog.getJSONArray("applications");
        java.util.Set<String> selected = ApplicationRouting.load(context).packages;
        int expected = 0, other = 0, icons = 0;
        for (int index = 0; index < entries.length(); index++) {
            JSONObject item = entries.getJSONObject(index);
            boolean visible = item.getBoolean("launchable") || !item.getBoolean("system")
                || selected.contains(item.getString("package"));
            if (visible) {
                expected++;
                if (!context.getPackageName().equals(item.getString("package"))) other++;
                String icon = item.getString("icon");
                if (!icon.isEmpty() && new File(icon).isFile()) icons++;
            }
        }
        JSONObject ui = MainActivity.diagnostics();
        assertEquals("Application enumeration failed: " + ui, "", ui.getString("applications_error"));
        assertEquals(entries.length(), ui.getInt("applications_total"));
        boolean all = ui.getInt("applications_mode") == 0;
        assertEquals(all ? 0 : expected, ui.getInt("applications_visible"));
        assertTrue("The emulator has no other visible applications", other > 0);
        assertFalse("Application visibility was incorrectly reported as restricted", catalog.getBoolean("visibility_limited"));
        assertTrue("No installed application icon was loaded", icons > 0);
        if (all) assertEquals("All applications must hide the list and icons", 0, ui.getInt("applications_icons"));
        else assertTrue("Slint did not receive application icons", ui.getInt("applications_icons") > 0);
        assertEquals("Saved routing summary is missing", catalog.getString("summary"), ui.getString("applications_saved_summary"));
    }
    private static void openApplicationRouting() throws Exception {
        previewNavigate(9);
        previewSettingsTop();
        waitForAppWindow();
        java.util.concurrent.atomic.AtomicReference<String> previous = new java.util.concurrent.atomic.AtomicReference<>();
        java.util.concurrent.atomic.AtomicLong stableSince = new java.util.concurrent.atomic.AtomicLong();
        await("Application routing entry layout did not settle", () -> {
            JSONObject ui = MainActivity.diagnostics();
            JSONArray bounds = previewBounds(ui, "preview-application-routing");
            if (bounds == null || bounds.length() != 4 || bounds.optDouble(2) <= 0 || bounds.optDouble(3) <= 0) return false;
            String geometry = bounds.toString() + "/" + ui.optDouble("width") + "/" + ui.optDouble("height");
            long now = SystemClock.uptimeMillis();
            if (!geometry.equals(previous.getAndSet(geometry))) stableSince.set(now);
            return now - stableSince.get() >= 250;
        });
        previewTarget("preview-application-routing");
        await("Application enumeration did not finish", () -> {
            JSONObject ui = MainActivity.diagnostics();
            return ui.optInt("detail") == 18 && !ui.optBoolean("applications_loading")
                && (ui.optBoolean("applications_loaded") || !ui.optString("applications_error").isEmpty());
        });
    }
    private static void applicationMode(int mode) throws Exception {
        JSONObject before = MainActivity.diagnostics();
        int previous = before.getInt("applications_mode");
        int total = before.getInt("applications_total");
        int selected = before.getInt("applications_selected");
        int expected = previous == mode || mode == 0 ? selected
            : previous == 0 ? (mode == 1 ? total : 0) : total - selected;
        previewTarget("preview-applications-mode:" + mode);
        await("Application routing mode or automatic inversion did not apply: " + mode, () -> {
            JSONObject ui = MainActivity.diagnostics();
            if (!ui.optString("applications_error").isEmpty())
                throw new IOException(ui.getString("applications_error"));
            if (ui.optInt("applications_mode", -1) == mode
                    && ui.optInt("applications_selected", -1) == expected) return true;
            if (ui.optInt("applications_mode", -1) == previous && previous != mode) {
                tryPreviewTarget("preview-applications-mode:" + mode);
                SystemClock.sleep(350);
            }
            return false;
        });
        if (mode == 0)
            assertEquals("All applications must remove the selection list", 0,
                MainActivity.diagnostics().getInt("applications_visible"));
    }
    private static void previewApplications(Context context, File directory, JSONArray cases,
            String prefix, boolean modes) throws Exception {
        openApplicationRouting();
        previewApplicationPolicies(directory, cases, prefix);
        openApplicationRouting();
        previewApplicationCatalog(context);
        applicationMode(0);
        capturePreview(directory, cases, prefix + "-applications-all");
        applicationMode(1);
        capturePreview(directory, cases, prefix + "-applications-default");
        if (modes) {
            previewTarget("preview-applications-system");
            await("System components did not become visible",
                () -> MainActivity.diagnostics().optBoolean("applications_system"));
            capturePreview(directory, cases, prefix + "-applications-system");
            previewTarget("preview-applications-system");
            await("System component filter did not return",
                () -> !MainActivity.diagnostics().optBoolean("applications_system"));
            for (int round = 0; round < 3; round++) {
                applicationMode(2);
                applicationMode(2);
                if (round == 0) capturePreview(directory, cases, prefix + "-applications-exclude");
                applicationMode(1);
                applicationMode(1);
            }
            capturePreview(directory, cases, prefix + "-applications-only");
        }
        double beforeScroll = MainActivity.diagnostics().getDouble("applications_scroll_y");
        previewSwipe(true);
        await("Application list did not scroll",
            () -> MainActivity.diagnostics().getDouble("applications_scroll_y") < beforeScroll - 1);
        capturePreview(directory, cases, prefix + "-applications-scrolled");
        boolean dirty = MainActivity.diagnostics().getBoolean("applications_dirty");
        shell("input keyevent KEYCODE_BACK");
        if (dirty) {
            await("Unsaved application changes did not request confirmation",
                () -> MainActivity.diagnostics().optInt("detail") == 12);
            if (modes) capturePreview(directory, cases, prefix + "-applications-unsaved");
            previewTarget("preview-discard");
        }
        await("Application routing did not return to settings",
            () -> MainActivity.diagnostics().optInt("detail", -1) == 0);
    }
    private static void previewPrimaryPages(File directory, JSONArray cases, String prefix) throws Exception {
        waitForAppWindow();
        previewNavigate(0);
        capturePreview(directory, cases, prefix + "-home");
        previewNavigate(1);
        await("Proxy groups were not loaded", () -> MainActivity.diagnostics().optInt("groups") > 0);
        capturePreview(directory, cases, prefix + "-proxies-grid");
        previewTarget("nodes-view");
        await("Proxy list view did not open", () -> MainActivity.diagnostics().optBoolean("nodes_list"));
        capturePreview(directory, cases, prefix + "-proxies-list");
        previewTarget("nodes-view");
        await("Proxy grid view did not return", () -> !MainActivity.diagnostics().optBoolean("nodes_list"));
        previewNavigate(2);
        await("Subscriptions were not loaded", () -> MainActivity.diagnostics().optInt("profiles") > 0);
        capturePreview(directory, cases, prefix + "-subscriptions-grid");
        previewTarget("profiles-view");
        await("Subscription list view did not open", () -> MainActivity.diagnostics().optBoolean("profiles_list"));
        capturePreview(directory, cases, prefix + "-subscriptions-list");
        previewTarget("profiles-view");
        await("Subscription grid view did not return", () -> !MainActivity.diagnostics().optBoolean("profiles_list"));
    }
    @Test public void androidUiPreviewCapturesPagesAndApplications() throws Exception {
        Context context = InstrumentationRegistry.getInstrumentation().getTargetContext();
        String before = ApplicationRouting.load(context).summary();
        File directory = new File(context.getExternalFilesDir(null), "application-preview");
        assertTrue(directory.isDirectory() || directory.mkdirs());
        JSONArray cases = new JSONArray();
        android.app.UiAutomation automation = InstrumentationRegistry.getInstrumentation().getUiAutomation();
        context.startActivity(new Intent(context, MainActivity.class).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK));
        try {
            await("Slint did not start for Android screenshots", () -> MainActivity.diagnostics().optBoolean("live"));
            await("Android screenshot activity did not receive focus", () -> inStage(androidx.test.runner.lifecycle.Stage.RESUMED, true));
            waitForAppWindow();
            if (MainActivity.diagnostics().optBoolean("notice_error")) {
                capturePreview(directory, cases, "startup-error");
                dismissErrorDialogIfPresent(false);
            }
            previewApplications(context, directory, cases, "phone", true);
            previewPrimaryPages(directory, cases, "phone");
            tapTarget("profile_add_target");
            await("Subscription editor did not open", () -> MainActivity.diagnostics().optInt("detail") == 3);
            capturePreview(directory, cases, "phone-subscription-editor");
            shell("input keyevent KEYCODE_BACK");
            await("Subscription editor did not close", () -> MainActivity.diagnostics().optInt("detail") == 0);
            previewNavigate(10);
            capturePreview(directory, cases, "phone-tools");
            for (int page = 3; page <= 8; page++) {
                previewTool(page);
                String[] names = {"connections", "rules", "logs", "network", "ip", "ai"};
                capturePreview(directory, cases, "phone-" + names[page - 3]);
                if (page == 4) {
                    previewTarget("rule-shortcuts-open");
                    await("Rule shortcuts did not open", () -> MainActivity.diagnostics().optBoolean("shortcuts_open"));
                    capturePreview(directory, cases, "phone-rule-shortcuts");
                    shell("input keyevent KEYCODE_BACK");
                    await("Rule shortcuts did not close", () -> !MainActivity.diagnostics().optBoolean("shortcuts_open"));
                }
            }
            previewSettings(directory, cases, "phone");
            assertTrue("Window manager rejected landscape rotation", automation.setRotation(android.app.UiAutomation.ROTATION_FREEZE_90));
            await("Phone did not enter landscape", () -> {
                JSONObject ui = MainActivity.diagnostics();
                return ui.optDouble("width") > ui.optDouble("height");
            });
            previewPrimaryPages(directory, cases, "landscape");
            previewSettings(directory, cases, "landscape");
            previewApplications(context, directory, cases, "landscape", false);
            assertTrue("Window manager rejected portrait rotation", automation.setRotation(android.app.UiAutomation.ROTATION_FREEZE_0));
            await("Phone did not return to portrait before tablet resize", () -> {
                JSONObject ui = MainActivity.diagnostics();
                return ui.optDouble("width") < ui.optDouble("height");
            });
            shell("wm size 800x1280");
            shell("wm density 160");
            assertTrue("Window manager rejected tablet portrait rotation",
                automation.setRotation(android.app.UiAutomation.ROTATION_FREEZE_0));
            await("Tablet portrait viewport was not applied", () -> {
                JSONObject ui = MainActivity.diagnostics();
                return Math.abs(ui.optDouble("scale") - 1.0) < 0.001
                    && ui.optDouble("width") >= 760 && ui.optDouble("height") > ui.optDouble("width");
            });
            previewPrimaryPages(directory, cases, "tablet-portrait");
            previewSettings(directory, cases, "tablet-portrait");
            previewApplications(context, directory, cases, "tablet-portrait", false);
            assertTrue("Window manager rejected tablet rotation", automation.setRotation(android.app.UiAutomation.ROTATION_FREEZE_90));
            await("Tablet landscape viewport was not applied", () -> {
                JSONObject ui = MainActivity.diagnostics();
                return Math.abs(ui.optDouble("scale") - 1.0) < 0.001
                    && ui.optDouble("width") >= 1024 && ui.optDouble("width") > ui.optDouble("height");
            });
            previewPrimaryPages(directory, cases, "tablet-landscape");
            previewSettings(directory, cases, "tablet-landscape");
            previewApplications(context, directory, cases, "tablet-landscape", false);
            assertEquals("Preview must not save application routing", before, ApplicationRouting.load(context).summary());
            assertFalse("Preview must not start VPN", TunnelService.connected() || TunnelService.pending());
        } catch (Exception | AssertionError error) {
            try { capturePreview(directory, cases, "failure"); }
            catch (Exception | AssertionError snapshot) { error.addSuppressed(snapshot); }
            throw error;
        } finally {
            shell("wm size reset");
            shell("wm density reset");
            automation.setRotation(android.app.UiAutomation.ROTATION_UNFREEZE);
        }
    }
    @Test public void applicationRoutingPersistsAndKeepsOtherAppsOnline() throws Exception {
        incidentNotificationsKeepSeverityPermissionAndRateLimitsSeparate();
        Context context = InstrumentationRegistry.getInstrumentation().getTargetContext();
        persistNetworkProfile(context);
        android.content.pm.ApplicationInfo bundled = new android.content.pm.ApplicationInfo();
        bundled.flags = android.content.pm.ApplicationInfo.FLAG_SYSTEM;
        assertTrue("Preinstalled launchable apps must remain selectable",
            ApplicationCatalog.visibleWithoutComponents(bundled, true, false));
        assertFalse("Non-launchable system components are hidden by default",
            ApplicationCatalog.visibleWithoutComponents(bundled, false, false));
        assertTrue("Filtering must not hide selected system components",
            ApplicationCatalog.visibleWithoutComponents(bundled, false, true));
        bundled.flags = 0;
        assertTrue("Background user applications must remain selectable",
            ApplicationCatalog.visibleWithoutComponents(bundled, false, false));
        context.startActivity(new Intent(context, MainActivity.class).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK));
        await("Native core did not restore for application routing", CoreBridge::ready);
        assertNull("Application routing requires the consent granted by the preceding VPN test", VpnService.prepare(context));
        android.content.SharedPreferences preferences = context.getSharedPreferences("vpn", Context.MODE_PRIVATE);
        ApplicationRouting original = ApplicationRouting.load(preferences);
        android.content.SharedPreferences migration = context.getSharedPreferences("routing-regression", Context.MODE_PRIVATE);
        String self = context.getPackageName();
        String other = "com.android.shell";
        context.getPackageManager().getApplicationInfo(other, 0);
        String removed = "org.clashofrust.uninstalled.regression";
        try {
            context.getPackageManager().getApplicationInfo(removed, 0);
            fail("The removed-application fixture must not be installed");
        } catch (android.content.pm.PackageManager.NameNotFoundException expected) {
            assertNotNull(expected);
        }
        try {
            assertTrue(migration.edit().clear().commit());
            assertEquals(ApplicationRouting.ALL, ApplicationRouting.load(migration).mode);
            assertTrue(migration.edit().putStringSet("excluded", java.util.Collections.singleton(self)).commit());
            ApplicationRouting legacy = ApplicationRouting.load(migration);
            assertEquals(ApplicationRouting.EXCLUDE, legacy.mode);
            assertEquals(java.util.Collections.singleton(self), legacy.packages);
            legacy.persist(migration);
            assertEquals(ApplicationRouting.EXCLUDE, ApplicationRouting.load(migration).mode);
            assertEquals(java.util.Collections.singleton(self), ApplicationRouting.load(migration).packages);
            ApplicationRouting protectedRouting = ApplicationRouting.effective(legacy, true);
            assertEquals(ApplicationRouting.ALL, protectedRouting.mode);
            assertTrue(protectedRouting.packages.isEmpty());
            assertEquals(java.util.Collections.singleton(self), legacy.packages);
            try {
                new ApplicationRouting(ApplicationRouting.ONLY, java.util.Collections.emptySet()).save(context);
                fail("An empty allowlist must never capture every application");
            } catch (IllegalArgumentException expected) {
                assertEquals("请至少选择一个应用", expected.getMessage());
            }
            String raw = CoreBridge.config();
            assertNetworkProfile(raw);
            String controller = value(raw, "external-controller");
            String secret = value(raw, "secret");
            ConnectivityManager connectivity = context.getSystemService(ConnectivityManager.class);
            new ApplicationRouting(ApplicationRouting.ONLY, java.util.Collections.singleton(self)).save(context);
            openApplicationRouting();
            previewApplicationCatalog(context);
            java.util.Set<String> available = new java.util.HashSet<>();
            JSONArray catalog = ApplicationCatalog.read(context).getJSONArray("applications");
            for (int index = 0; index < catalog.length(); index++)
                available.add(catalog.getJSONObject(index).getString("package"));
            java.util.Set<String> complement = new java.util.HashSet<>(available);
            complement.remove(self);
            for (int round = 0; round < 3; round++) {
                applicationMode(2);
                applicationMode(2);
                assertTrue("Mode changes must remain drafts", MainActivity.diagnostics().getBoolean("applications_dirty"));
                assertEquals(java.util.Collections.singleton(self), ApplicationRouting.load(context).packages);
                applicationMode(1);
                applicationMode(1);
                assertFalse("A full round trip must restore the saved state",
                    MainActivity.diagnostics().getBoolean("applications_dirty"));
            }
            applicationMode(2);
            previewTarget("preview-applications-save");
            await("UI save did not persist the inverted selection", () -> {
                ApplicationRouting saved = ApplicationRouting.load(context);
                return ApplicationRouting.EXCLUDE.equals(saved.mode) && complement.equals(saved.packages)
                    && !MainActivity.diagnostics().optBoolean("applications_dirty", true);
            });
            TunnelService.start(context, false);
            await("The inverted exclusion must keep this application inside VPN", () -> {
                if (!TunnelService.error().isEmpty()) throw new IOException(TunnelService.error());
                return TunnelService.connected() && vpnActive(connectivity);
            });
            tunCarriesSocket(controller, secret);
            for (int handover=0;handover<3;handover++) {
                long previousGeneration=TunnelService.generation();
                TunnelService.start(context,true);
                await("Repeated routing replacement did not reconnect",()->
                    TunnelService.connected() && TunnelService.generation()>previousGeneration);
                tunCarriesSocket(controller,secret);
            }
            shell("input keyevent KEYCODE_BACK");
            await("Saved editor did not close", () -> MainActivity.diagnostics().optInt("detail", -1) == 0);
            openApplicationRouting();
            assertEquals(2, MainActivity.diagnostics().getInt("applications_mode"));
            assertEquals(complement.size(), MainActivity.diagnostics().getInt("applications_selected"));
            applicationMode(1);
            long invertedGeneration = TunnelService.generation();
            previewTarget("preview-applications-save");
            await("Returning to only-selected must save and reconnect automatically", () -> {
                if (!TunnelService.error().isEmpty()) throw new IOException(TunnelService.error());
                ApplicationRouting saved = ApplicationRouting.load(context);
                return ApplicationRouting.ONLY.equals(saved.mode)
                    && java.util.Collections.singleton(self).equals(saved.packages)
                    && TunnelService.generation() > invertedGeneration
                    && TunnelService.connected() && vpnActive(connectivity)
                    && !MainActivity.diagnostics().optBoolean("applications_dirty", true);
            });
            tunCarriesSocket(controller, secret);
            applicationMode(0);
            applicationMode(2);
            assertEquals(0, MainActivity.diagnostics().getInt("applications_selected"));
            applicationMode(0);
            applicationMode(1);
            assertEquals(available.size(), MainActivity.diagnostics().getInt("applications_selected"));
            previewTarget("preview-applications-revert");
            await("Revert must restore the saved mode and selection", () -> {
                JSONObject ui = MainActivity.diagnostics();
                return ui.optInt("applications_mode", -1) == 1 && ui.optInt("applications_selected", -1) == 1
                    && !ui.optBoolean("applications_dirty", true);
            });
            applicationMode(0);
            long allGeneration = TunnelService.generation();
            previewTarget("preview-applications-save");
            await("All-applications mode must save and reconnect automatically", () -> {
                if (!TunnelService.error().isEmpty()) throw new IOException(TunnelService.error());
                return ApplicationRouting.ALL.equals(ApplicationRouting.load(context).mode)
                    && TunnelService.generation() > allGeneration
                    && TunnelService.connected() && vpnActive(connectivity)
                    && !MainActivity.diagnostics().optBoolean("applications_dirty", true);
            });
            tunCarriesSocket(controller, secret);
            shell("input keyevent KEYCODE_BACK");
            await("Saved all-applications editor did not close", () -> MainActivity.diagnostics().optInt("detail", -1) == 0);
            verifyApplicationOutbounds(context, controller, secret);
            openApplicationRouting();
            assertEquals(0, MainActivity.diagnostics().getInt("applications_mode"));
            assertEquals(0, MainActivity.diagnostics().getInt("applications_visible"));
            shell("input keyevent KEYCODE_BACK");
            await("All-applications editor did not close", () -> MainActivity.diagnostics().optInt("detail", -1) == 0);
            long preservedGeneration=TunnelService.generation();
            new ApplicationRouting(ApplicationRouting.ONLY,java.util.Collections.singleton(removed)).persist(preferences);
            TunnelService.start(context,true);
            await("Rejected routing replacement did not preserve the working VPN",()->
                !TunnelService.error().isEmpty() && TunnelService.connected() && !TunnelService.pending());
            assertEquals("Rejected routing replacement changed the VPN generation",preservedGeneration,TunnelService.generation());
            tunCarriesSocket(controller,secret);
            new ApplicationRouting(ApplicationRouting.ALL,java.util.Collections.emptySet()).persist(preferences);
            await("Rejected routing replacement did not show its error",()->
                MainActivity.diagnostics().optBoolean("notice_active") && MainActivity.diagnostics().optBoolean("notice_error"));
            tapTarget("notice_close_target");
            await("Rejected routing replacement notice did not close",()->!MainActivity.diagnostics().optBoolean("notice_active"));
            TunnelService.stop(context);
            await("UI routing regression did not disconnect VPN", () -> !TunnelService.connected() && !TunnelService.pending());
            awaitDirectRecovery(connectivity);
            String[] modes = {ApplicationRouting.ALL, ApplicationRouting.ONLY, ApplicationRouting.ONLY,
                ApplicationRouting.EXCLUDE, ApplicationRouting.EXCLUDE};
            String[] packages = {self, self, other, self, other};
            boolean[] captured = {true, true, false, false, true};
            for (int index = 0; index < modes.length; index++) {
                String mode = modes[index];
                java.util.Set<String> selected = new java.util.HashSet<>(java.util.Collections.singleton(packages[index]));
                selected.add(removed);
                new ApplicationRouting(mode, selected).save(context);
                assertEquals(mode, ApplicationRouting.load(context).mode);
                boolean throughVpn = captured[index];
                TunnelService.start(context, false);
                await("Application routing did not establish VPN for " + mode, () -> {
                    if (!TunnelService.error().isEmpty()) throw new IOException(TunnelService.error());
                    return TunnelService.connected();
                });
                await("The application's selected route did not match " + mode,
                    () -> vpnActive(connectivity) == throughVpn);
                if (throughVpn) tunCarriesSocket(controller, secret);
                else {
                    assertEquals("An app outside the VPN must retain direct access", "cor-android-network-ok",
                        read(new URL("http://10.0.2.2:18081/probe"), java.net.Proxy.NO_PROXY, null));
                    noTunRoute();
                }
                ApplicationRouting cleaned = ApplicationRouting.load(context);
                if (!ApplicationRouting.ALL.equals(mode)) {
                    assertFalse("Uninstalled applications must be pruned for " + mode,
                        cleaned.packages.contains(removed));
                    assertEquals("Installed applications must remain selected for " + mode,
                        java.util.Collections.singleton(packages[index]), cleaned.packages);
                }
                new ApplicationRouting(ApplicationRouting.ALL, java.util.Collections.emptySet()).save(context);
                assertEquals("Saving preferences alone must not change the live VPN", throughVpn, vpnActive(connectivity));
                long previousGeneration = TunnelService.generation();
                JSONObject saved = ApplicationCatalog.save(context, new JSONObject()
                    .put("mode", ApplicationRouting.ALL).put("selected", new JSONArray()));
                assertTrue("Saving while connected must request a VPN reconfiguration", saved.getBoolean("reconnecting"));
                assertEquals("", saved.getString("reconnect_error"));
                await("Saved application routing was not applied without a manual reconnect", () -> {
                    if (!TunnelService.error().isEmpty()) throw new IOException(TunnelService.error());
                    return TunnelService.generation() > previousGeneration
                        && TunnelService.connected() && vpnActive(connectivity);
                });
                assertEquals(ApplicationRouting.ALL, ApplicationRouting.load(context).mode);
                tunCarriesSocket(controller, secret);
                TunnelService.stop(context);
                await("Application routing left VPN connected", () -> !TunnelService.connected());
                awaitDirectRecovery(connectivity);
            }
            new ApplicationRouting(ApplicationRouting.ONLY,
                java.util.Collections.singleton(removed)).save(context);
            TunnelService.start(context, false);
            await("An entirely uninstalled allowlist was not rejected", () -> !TunnelService.error().isEmpty());
            assertFalse(TunnelService.connected());
            assertTrue(ApplicationRouting.load(context).packages.isEmpty());
            assertFalse("A rejected allowlist must not capture all apps", vpnActive(connectivity));
        } finally {
            TunnelService.stop(context);
            await("Application routing cleanup did not stop VPN", () -> !TunnelService.connected() && !TunnelService.pending());
            original.persist(preferences);
            assertTrue(migration.edit().clear().commit());
        }
    }

    private static String applicationOutboundConfig(String raw,String packageName,String target,boolean rejectMissing) {
        String runtime=raw.replace("find-process-mode: off","find-process-mode: strict")
            .replace("find-process-mode: 'off'","find-process-mode: strict")
            .replace("find-process-mode: \"off\"","find-process-mode: strict");
        String effective="missing-node".equals(target) ? (rejectMissing ? "REJECT" : "") : target;
        if (!effective.isEmpty()) runtime=runtime.replace("rules:\n","rules:\n- PROCESS-NAME,"+packageName+","+effective+"\n- PROCESS-NAME,"+packageName+",REJECT\n");
        runtime=runtime.replace("rules:\n","rules:\n- PROCESS-NAME,cor-reject:"+packageName+",REJECT\n");
        return runtime+"\ncor-application-rules:\n  schema: 1\n  routes:\n"
            +"    - {package: "+packageName+", target: '"+target+"', reject_missing: "+rejectMissing+"}\n";
    }

    private static void previewApplicationPolicies(File directory, JSONArray cases, String prefix) throws Exception {
        previewScrollableTarget("application-policies-open", "preview-applications-scroll");
        await("Application policies did not load the current profile", () ->
            MainActivity.diagnostics().optInt("detail")==19
                && MainActivity.diagnostics().optBoolean("application_policies_loaded"));
        assertEquals("Application policy UI must follow the actual Android capability",
            Build.VERSION.SDK_INT>=29,MainActivity.diagnostics().getBoolean("application_policies_supported"));
        int before = MainActivity.diagnostics().getInt("application_policies_count");
        if (directory != null) capturePreview(directory, cases, prefix + "-application-policy-groups");
        if (Build.VERSION.SDK_INT<29) {
            assertNull("Unsupported Android must not offer a writable application rule form",
                previewBounds(MainActivity.diagnostics(),"application-policy-save"));
        } else {
            previewTarget("application-policy-new");
            await("New application rule group did not open", () ->
                MainActivity.diagnostics().optBoolean("application_policies_editing"));
            assertTrue(MainActivity.diagnostics().getBoolean("application_policies_reject_missing"));
            assertFalse("New groups must not silently choose an outbound",
                MainActivity.diagnostics().getBoolean("application_policies_target_selected"));
            assertEquals(0, MainActivity.diagnostics().getInt("application_policies_selected"));
            if (directory != null) capturePreview(directory, cases, prefix + "-application-policy-editor");
            await("Application picker control was outside its scroll viewport", () -> {
                if (tryPreviewTarget("application-policy-application")) return true;
                previewSwipe(true);
                return false;
            });
            await("Application rule picker did not open", () ->
                MainActivity.diagnostics().optBoolean("application_policies_choosing"));
            if (directory != null) capturePreview(directory, cases, prefix + "-application-policy-picker");
            shell("input keyevent KEYCODE_BACK");
            await("Back must return from application picker to the same draft", () ->
                !MainActivity.diagnostics().optBoolean("application_policies_choosing")
                    && MainActivity.diagnostics().optBoolean("application_policies_editing"));
            assertTrue(MainActivity.diagnostics().getBoolean("application_policies_reject_missing"));
            shell("input keyevent KEYCODE_BACK");
            await("Back did not cancel the unchanged group draft", () ->
                !MainActivity.diagnostics().optBoolean("application_policies_editing"));
            assertEquals("Cancelled groups must not change saved rules", before,
                MainActivity.diagnostics().getInt("application_policies_count"));
        }
        shell("input keyevent KEYCODE_BACK");
        await("Application policies did not close", () ->
            MainActivity.diagnostics().optInt("detail",-1)==0
                && !MainActivity.diagnostics().optBoolean("application_policies_loaded"));
    }

    private static void verifyApplicationOutbounds(Context context,String controller,String secret) throws Exception {
        assertEquals(Build.VERSION.SDK_INT>=29,ApplicationOwners.capability().getBoolean("supported"));
        openApplicationRouting();
        previewApplicationPolicies(null, null, "");
        if (Build.VERSION.SDK_INT<29) return;
        String original=CoreBridge.config();
        String restrictive=original.replace("198.51.100.1/32","203.0.113.1/32")
            .replace("MATCH,DIRECT","MATCH,REJECT");
        String self=context.getPackageName();
        try {
            for (String nodeName : new String[]{"🇺🇸 美国节点", "🚀 日本 [01].+", "👩🏽‍💻 台湾✈️"}) {
                String named=restrictive.replace("CI-TUN-origin",nodeName)
                    + "\nproxy-groups:\n  - name: cor-application-unicode\n    type: select\n"
                    + "    hidden: true\n    proxies: ['"+nodeName+"']\n    empty-fallback: REJECT\n";
                long missingBefore=new JSONObject(org.clashofrust.nativecore.mobile.Mobile.applicationRoutingDiagnostics())
                    .getLong("missing_rejected");
                CoreBridge.reload(applicationOutboundConfig(named,self,"cor-application-unicode",true));
                tunCarriesSocket(controller,secret,"198.51.100.1",true,nodeName);
                tunCarriesSocket(controller,secret,"application-policy.test",true,nodeName);
                assertEquals("A valid emoji target was reported as missing",missingBefore,
                    new JSONObject(org.clashofrust.nativecore.mobile.Mobile.applicationRoutingDiagnostics())
                        .getLong("missing_rejected"));
            }
            CoreBridge.reload(applicationOutboundConfig(restrictive,self,"CI-TUN-origin",false));
            tunCarriesSocket(controller,secret,"198.51.100.1",true,"CI-TUN-origin");
            tunCarriesSocket(controller,secret,"application-policy.test",true,"CI-TUN-origin");
            long rejectedBefore=new JSONObject(org.clashofrust.nativecore.mobile.Mobile.applicationRoutingDiagnostics())
                .getLong("udp_rejected");
            long udpMissingBefore=new JSONObject(org.clashofrust.nativecore.mobile.Mobile.applicationRoutingDiagnostics())
                .getLong("missing_rejected");
            try (DatagramSocket socket=new DatagramSocket()) {
                socket.connect(InetAddress.getByName("198.51.100.1"),18081);
                socket.send(new DatagramPacket(new byte[]{1},1));
                await("Application UDP must not fall through to another outbound",
                    ()->new JSONObject(org.clashofrust.nativecore.mobile.Mobile.applicationRoutingDiagnostics())
                        .getLong("udp_rejected")>rejectedBefore);
            }
            assertEquals("Unsupported UDP must not be reported as a missing target",udpMissingBefore,
                new JSONObject(org.clashofrust.nativecore.mobile.Mobile.applicationRoutingDiagnostics())
                    .getLong("missing_rejected"));
            CoreBridge.reload(applicationOutboundConfig(restrictive,self,"DIRECT",false));
            assertEquals("cor-android-network-ok",read(new URL("http://10.0.2.2:18081/probe"),java.net.Proxy.NO_PROXY,null));
            long missingRejectedBefore=new JSONObject(org.clashofrust.nativecore.mobile.Mobile.applicationRoutingDiagnostics())
                .getLong("missing_rejected");
            CoreBridge.reload(applicationOutboundConfig(original,self,"REJECT",false));
            boolean blocked=false;
            try {read(new URL("http://10.0.2.2:18081/probe"),java.net.Proxy.NO_PROXY,null);}
            catch (IOException expected) {blocked=true;}
            assertTrue("An application reject rule must override the subscription",blocked);
            assertEquals("Explicit rejection must not be reported as an unavailable target",
                missingRejectedBefore,new JSONObject(org.clashofrust.nativecore.mobile.Mobile.applicationRoutingDiagnostics())
                    .getLong("missing_rejected"));
            CoreBridge.reload(applicationOutboundConfig(original,self,"missing-node",false));
            assertEquals("cor-android-network-ok",read(new URL("http://10.0.2.2:18081/probe"),java.net.Proxy.NO_PROXY,null));
            CoreBridge.reload(applicationOutboundConfig(original,self,"missing-node",true));
            blocked=false;
            try {read(new URL("http://10.0.2.2:18081/probe"),java.net.Proxy.NO_PROXY,null);}
            catch (IOException expected) {blocked=true;}
            assertTrue("A missing target with blocking enabled must not use DIRECT",blocked);
            assertTrue("Missing-target rejection did not reach the incident notification counter",
                new JSONObject(org.clashofrust.nativecore.mobile.Mobile.applicationRoutingDiagnostics())
                    .getLong("missing_rejected")>missingRejectedBefore);
            await("Missing-target rejection did not leave a recovery reminder", () ->
                "application-target-missing".equals(IncidentNotifications.state(context).optString("code")));
            CoreBridge.reload(applicationOutboundConfig(restrictive,"com.android.shell","CI-TUN-origin",false));
            long matchesBefore=new JSONObject(org.clashofrust.nativecore.mobile.Mobile.applicationRoutingDiagnostics())
                .getLong("matches");
            String probeApk=InstrumentationRegistry.getInstrumentation().getContext().getApplicationInfo().sourceDir;
            JSONObject response=new JSONObject(shell("app_process -Djava.class.path="+probeApk
                +" /system/bin org.clashofrust.android.ApplicationUidProbe").trim());
            assertEquals("The independent probe must run as the assigned UID",
                context.getPackageManager().getApplicationInfo("com.android.shell",0).uid,response.getInt("uid"));
            assertEquals("A different UID did not use its assigned outbound: "+response,
                "cor-android-network-ok",response.optString("body"));
            assertEquals(200,response.getInt("status"));
            assertTrue("The independent UID must produce a fresh application rule match",
                new JSONObject(org.clashofrust.nativecore.mobile.Mobile.applicationRoutingDiagnostics())
                    .getLong("matches")>matchesBefore);
            blocked=false;
            try {read(new URL("http://10.0.2.2:18081/probe"),java.net.Proxy.NO_PROXY,null);}
            catch (IOException expected) {blocked=true;}
            assertTrue("Another application's assignment must not affect this UID",blocked);
            assertTrue(new JSONObject(org.clashofrust.nativecore.mobile.Mobile.applicationRoutingDiagnostics()).getLong("matches")>0);
        } finally {
            CoreBridge.reload(original);
        }
    }

    private static void measureDiagnosticGroup() throws Exception {
        measureDiagnostic(false);
    }

    private static void measureDiagnostic(boolean all) throws Exception {
        dismissErrorDialogIfPresent(false);
        previewNavigate(1);
        if (!all) {
            await("The measurement strategy group is not available",
                () -> MainActivity.diagnostics().optInt("groups") > 0);
        }
        dismissErrorDialogIfPresent(false);
        if (MainActivity.diagnostics().optBoolean("notice_active")) {
            tapTarget("notice_close_target");
            await("The previous notification did not close",
                () -> !MainActivity.diagnostics().optBoolean("notice_active"));
        }
        String key = all ? "preview-measure-all" : "preview-measure-group:CI-diagnostics";
        for (int attempt = 0; attempt < 10; attempt++) {
            JSONObject ui = MainActivity.diagnostics();
            JSONArray target = previewBounds(ui, key);
            JSONArray viewport = previewBounds(ui, "preview-content");
            if (target != null && viewport != null) {
                double center = target.getDouble(1) + target.getDouble(3) / 2;
                if (center > viewport.getDouble(1)
                        && center < viewport.getDouble(1) + viewport.getDouble(3)) break;
                previewSwipe(center >= viewport.getDouble(1) + viewport.getDouble(3));
            } else SystemClock.sleep(200);
        }
        previewTarget(key);
        await("Group measurement did not finish", () -> {
            JSONObject ui = MainActivity.diagnostics();
            return ui.optBoolean("notice_active") && !ui.optBoolean("delay_busy");
        });
    }

    private static boolean pickerFocused(android.app.UiAutomation automation, String packageName) {
        for (android.view.accessibility.AccessibilityWindowInfo window : automation.getWindows()) {
            if (window.getType() != android.view.accessibility.AccessibilityWindowInfo.TYPE_APPLICATION
                    || !window.isFocused()) continue;
            android.view.accessibility.AccessibilityNodeInfo root = window.getRoot();
            if (root != null && android.text.TextUtils.equals(root.getPackageName(), packageName)) return true;
        }
        return false;
    }

    private static void closeDiagnosticPicker() throws Exception {
        Context context = InstrumentationRegistry.getInstrumentation().getTargetContext();
        ComponentName picker = new Intent(Intent.ACTION_CREATE_DOCUMENT).setType("application/json")
            .addCategory(Intent.CATEGORY_OPENABLE).resolveActivity(context.getPackageManager());
        assertNotNull("No system document picker resolves the diagnostic export", picker);
        android.app.UiAutomation automation = InstrumentationRegistry.getInstrumentation().getUiAutomation();
        android.accessibilityservice.AccessibilityServiceInfo service = automation.getServiceInfo();
        assertNotNull("Android accessibility automation is unavailable", service);
        int originalFlags = service.flags;
        service.flags |= android.accessibilityservice.AccessibilityServiceInfo.FLAG_RETRIEVE_INTERACTIVE_WINDOWS;
        automation.setServiceInfo(service);
        try {
            for (int attempt = 0; attempt < 3; attempt++) {
                await("Neither the document picker nor the application has window focus", () ->
                    inStage(androidx.test.runner.lifecycle.Stage.RESUMED, true)
                        || pickerFocused(automation, picker.getPackageName()));
                if (inStage(androidx.test.runner.lifecycle.Stage.RESUMED, true)) return;
                shell("input keyevent KEYCODE_BACK");
                long deadline = SystemClock.uptimeMillis() + 2000;
                while (SystemClock.uptimeMillis() < deadline) {
                    if (inStage(androidx.test.runner.lifecycle.Stage.RESUMED, true)) return;
                    SystemClock.sleep(100);
                }
            }
            await("The document picker did not return to the application",
                () -> inStage(androidx.test.runner.lifecycle.Stage.RESUMED, true));
        } finally {
            service.flags = originalFlags;
            automation.setServiceInfo(service);
        }
    }

    private static JSONObject exportMeasurementReport(File directory, String name) throws Exception {
        dismissErrorDialogIfPresent(false);
        previewTool(5);
        MainActivity activity = resumedActivity();
        assertNotNull(activity);
        previewTarget("preview-export-diagnostics");
        java.lang.reflect.Field pending = MainActivity.class.getDeclaredField("exportContent");
        pending.setAccessible(true);
        java.util.concurrent.atomic.AtomicReference<String> content = new java.util.concurrent.atomic.AtomicReference<>();
        await("The real diagnostic export did not reach the document picker", () -> {
            InstrumentationRegistry.getInstrumentation().runOnMainSync(() -> {
                try { content.set((String)pending.get(activity)); }
                catch (IllegalAccessException error) { throw new AssertionError(error); }
            });
            return content.get() != null;
        });
        String expected = content.get();
        JSONObject prepared = new JSONObject(expected);
        assertEquals("The selected export is not a diagnostic report", "diagnose", prepared.getString("command"));
        java.lang.reflect.Field failure = MainActivity.class.getDeclaredField("documentFailure");
        failure.setAccessible(true);
        await("The diagnostic document picker did not open",
            () -> !inStage(androidx.test.runner.lifecycle.Stage.RESUMED, false));
        assertTrue(directory.isDirectory() || directory.mkdirs());
        File destination = new File(directory, name + ".json");
        InstrumentationRegistry.getInstrumentation().runOnMainSync(() ->
            activity.onActivityResult(12, android.app.Activity.RESULT_OK,
                new Intent().setData(Uri.fromFile(destination))));
        await("Diagnostic export did not finish writing valid JSON", () -> {
            String failed = (String)failure.get(activity);
            assertNull("Diagnostic export failed: " + failed, failed);
            if (!destination.isFile()) return false;
            try {
                new JSONObject(new String(java.nio.file.Files.readAllBytes(destination.toPath()), StandardCharsets.UTF_8));
                return true;
            } catch (JSONException error) { return false; }
        });
        closeDiagnosticPicker();
        waitForAppWindow();
        String saved = new String(java.nio.file.Files.readAllBytes(destination.toPath()), StandardCharsets.UTF_8);
        assertEquals("Exported bytes differ from the actual report payload", expected, saved);
        assertFalse("The custom measurement URL leaked into diagnostics", saved.contains("ci-only-url-secret"));
        JSONObject report = new JSONObject(saved);
        assertEquals(1, report.getInt("schema"));
        assertEquals("diagnose", report.getString("command"));
        JSONObject data = report.getJSONObject("data");
        assertEquals("collected", data.getJSONObject("core_logs").getString("state"));
        JSONObject context = data.getJSONObject("context");
        JSONArray measurements = context.getJSONArray("measurements");
        assertTrue(measurements.length() > 0 && measurements.length() <= 4);
        for (int index = 0; index < measurements.length(); index++) {
            JSONObject measurement = measurements.getJSONObject(index);
            assertEquals(1, measurement.getInt("schema"));
            assertFalse(measurement.getString("started_at").isEmpty());
            assertTrue(measurement.getLong("duration_ms") >= 0);
            assertEquals(5000, measurement.getInt("probe_timeout_ms"));
            assertEquals(6, measurement.getInt("concurrency"));
        }
        return report;
    }

    private static JSONObject latestMeasurement(JSONObject report) throws Exception {
        JSONArray measurements = report.getJSONObject("data").getJSONObject("context").getJSONArray("measurements");
        return measurements.getJSONObject(measurements.length() - 1);
    }

    private static void assertMixedMeasurement(JSONObject report, boolean vpn) throws Exception {
        JSONObject measurement = latestMeasurement(report);
        assertEquals("group", measurement.getString("scope"));
        assertTrue(measurement.getBoolean("completed"));
        assertEquals("complete", measurement.getString("phase"));
        assertEquals(2, measurement.getInt("attempted"));
        assertEquals(1, measurement.getInt("succeeded"));
        assertEquals(1, measurement.getInt("node_failures"));
        assertEquals(0, measurement.getInt("request_failures"));
        assertEquals(1, measurement.getJSONObject("codes").getInt("node_failed"));
        assertEquals("subscription_custom", measurement.getJSONArray("targets").getString(0));
        JSONObject context = report.getJSONObject("data").getJSONObject("context");
        JSONObject platform = context.getJSONObject("platform").getJSONObject("Ok");
        assertEquals(Build.VERSION.SDK_INT, platform.getInt("sdk"));
        assertEquals(vpn, platform.getJSONObject("vpn").getBoolean("connected"));
        JSONObject protection = platform.getJSONObject("socket_protection");
        assertEquals(vpn, protection.getBoolean("required"));
        assertEquals(0, protection.getLong("failures"));
        if (vpn) assertTrue(protection.getLong("attempts") > 0);
        JSONArray networks = platform.getJSONObject("network").getJSONArray("physical_networks");
        assertTrue(networks.length() > 0 && networks.length() <= 8);
        for (int index = 0; index < networks.length(); index++) {
            JSONObject network = networks.getJSONObject(index);
            assertEquals(Build.VERSION.SDK_INT >= 29, network.has("mtu"));
            assertEquals(Build.VERSION.SDK_INT >= 28, network.has("private_dns_active"));
            assertFalse(network.has("dns"));
            assertFalse(network.has("private_dns_hostname"));
        }
    }

    @Test public void measurementDiagnosticsDistinguishFailuresAndExport() throws Exception {
        Context context = InstrumentationRegistry.getInstrumentation().getTargetContext();
        persistNetworkProfile(context);
        File profile = new File(context.getFilesDir(),
            "client/profiles/b4f7a20e-402d-4739-90dd-4b517591e653.yaml");
        String source = new String(java.nio.file.Files.readAllBytes(profile.toPath()), StandardCharsets.UTF_8);
        source = source.replace("rules:\n",
            "  - {name: CI-diagnostics-offline, type: http, server: 127.0.0.1, port: 1}\n"
            + "proxy-groups:\n  - name: CI-diagnostics\n    type: url-test\n"
            + "    proxies: [CI-TUN-origin, CI-diagnostics-offline]\n"
            + "    url: http://198.51.100.1:18081/diagnostic-probe?token=ci-only-url-secret\n"
            + "    expected-status: 200\n    interval: 3600\n    lazy: true\nrules:\n");
        java.nio.file.Files.write(profile.toPath(), source.getBytes(StandardCharsets.UTF_8));
        context.startActivity(new Intent(context, MainActivity.class).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK));
        await("The measurement test core did not start", CoreBridge::ready);
        await("The measurement Activity did not receive focus",
            () -> inStage(androidx.test.runner.lifecycle.Stage.RESUMED, true));
        waitForAppWindow();
        String raw = CoreBridge.config();
        String secret = value(raw, "secret");
        File directory = new File(context.getExternalFilesDir(null), "measurement-diagnostics");
        ConnectivityManager connectivity = context.getSystemService(ConnectivityManager.class);
        try {
            await("The measurement diagnostic has no underlying network", () ->
                new JSONObject(resumedActivity().request("{\"op\":\"diagnostics\"}"))
                    .getJSONObject("network").getJSONArray("physical_networks").length() > 0);
            measureDiagnosticGroup();
            measureDiagnosticGroup();
            JSONObject direct = exportMeasurementReport(directory, "without-vpn");
            assertMixedMeasurement(direct, false);
            assertNull("The preceding network test must grant real VPN consent", VpnService.prepare(context));
            TunnelService.start(context, false);
            await("VPN did not connect for the measurement test", () -> {
                if (!TunnelService.error().isEmpty()) throw new IOException(TunnelService.error());
                return TunnelService.connected() && vpnActive(connectivity);
            });
            measureDiagnosticGroup();
            JSONObject vpn = exportMeasurementReport(directory, "with-vpn");
            assertMixedMeasurement(vpn, true);
            assertTrue("Received core logs have no observation timestamp",
                vpn.getJSONObject("data").getJSONObject("core_logs").getString("contents").contains("[observed_at="));
            TunnelService.stop(context);
            awaitDirectRecovery(connectivity);
            String rejected = raw.replaceFirst("(?m)^secret:.*$", "secret: ci-controller-rejected");
            assertNotEquals("No controller secret was changed for the isolated failure", raw, rejected);
            CoreBridge.reload(rejected);
            measureDiagnostic(true);
            JSONObject authentication = exportMeasurementReport(directory, "controller-authentication");
            JSONObject failed = latestMeasurement(authentication);
            assertFalse(failed.getBoolean("completed"));
            assertEquals("all", failed.getString("scope"));
            assertEquals("query_nodes", failed.getString("phase"));
            assertEquals(0, failed.getInt("attempted"));
            assertEquals("controller_authentication", failed.getJSONObject("failure").getString("code"));
            assertEquals(401, failed.getJSONObject("failure").getInt("http_status"));
            assertEquals(0, failed.getInt("node_failures"));
            CoreBridge.reload(raw);
            measureDiagnosticGroup();
            JSONObject recovered = exportMeasurementReport(directory, "recovered");
            assertMixedMeasurement(recovered, false);
            assertEquals(4, recovered.getJSONObject("data").getJSONObject("context")
                .getJSONArray("measurements").length());
            for (JSONObject report : new JSONObject[]{direct, vpn, authentication, recovered}) {
                assertFalse("The live controller credential leaked into the exported report", report.toString().contains(secret));
                assertFalse(report.toString().contains("ci-controller-rejected"));
            }
            JSONObject receipt = new JSONObject().put("schema", 1).put("sdk", Build.VERSION.SDK_INT)
                .put("cases", new JSONArray(new String[]{"without-vpn", "with-vpn",
                    "controller-authentication", "recovered"})).put("passed", true);
            java.nio.file.Files.write(new File(directory, "result.json").toPath(),
                receipt.toString().getBytes(StandardCharsets.UTF_8));
        } finally {
            CoreBridge.reload(raw);
            TunnelService.stop(context);
        }
    }

    @Test public void basicSystemProxyAndTun() throws Exception {
        heldResponseClassifiesTruncationAsTransportFailure();
        Context context = InstrumentationRegistry.getInstrumentation().getTargetContext();
        persistNetworkProfile(context);
        context.startActivity(new Intent(context, MainActivity.class).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK));
        await("Native UI did not start its core", CoreBridge::ready);
        await("Activity did not receive focus", () -> inStage(androidx.test.runner.lifecycle.Stage.RESUMED, true));
        waitForAppWindow();
        String raw = CoreBridge.config();
        assertNetworkProfile(raw);
        MainActivity activity = resumedActivity();
        assertNotNull(activity);
        assertNotNull("Test must start without pre-granted VPN consent", VpnService.prepare(context));
        activity.request("{\"op\":\"authorize-vpn\",\"request\":103}");
        authorization(103, "pending");
        consent(true);
        authorization(103, "granted");
        assertNull(VpnService.prepare(context));
        String secret = value(raw, "secret");
        String controller = value(raw, "external-controller");
        int mixed = Integer.parseInt(value(raw, "mixed-port"));
        java.net.Proxy proxy = new java.net.Proxy(java.net.Proxy.Type.HTTP, new InetSocketAddress("127.0.0.1", mixed));
        assertEquals("cor-android-network-ok", read(URI.create("http://10.0.2.2:18081/probe").toURL(), proxy, null));
        noTunRoute();
        ConnectivityManager connectivity = context.getSystemService(ConnectivityManager.class);
        try {
            TunnelService.start(context, false);
            await("VPN did not connect", () -> {
                if (!TunnelService.error().isEmpty()) throw new IOException(TunnelService.error());
                return TunnelService.connected();
            });
            await("Android did not make VPN its active network", () -> vpnActive(connectivity));
            tunCarriesSocket(controller, secret);
            tunResolvesAndCarriesHttp();
            JSONObject diagnostics = new JSONObject(activity.request("{\"op\":\"diagnostics\"}"));
            JSONObject protection = diagnostics.getJSONObject("socket_protection");
            assertTrue(protection.getBoolean("required"));
            assertTrue("VPN probes did not observe protected outbound sockets", protection.getLong("attempts") > 0);
            assertEquals("Android rejected outbound socket protection", 0, protection.getLong("failures"));
            assertTrue(diagnostics.getJSONObject("vpn").getBoolean("connected"));
            JSONArray[] observed = new JSONArray[1];
            await("Network diagnostics did not observe an underlying network", () -> {
                observed[0] = new JSONObject(activity.request("{\"op\":\"diagnostics\"}"))
                    .getJSONObject("network").getJSONArray("physical_networks");
                return observed[0].length() > 0;
            });
            JSONArray networks = observed[0];
            for (int index = 0; index < networks.length(); index++) {
                assertEquals("MTU diagnostics must respect the Android API version",
                    Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q, networks.getJSONObject(index).has("mtu"));
            }
        } finally {
            TunnelService.stop(context);
        }
        awaitDirectRecovery(connectivity);
    }

    @Test public void localProxyAndRealTunCarryTrafficAndDisconnectCleanly() throws Exception {
        Context context = InstrumentationRegistry.getInstrumentation().getTargetContext();
        persistNetworkProfile(context);
        context.startActivity(new Intent(context, MainActivity.class).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK));
        await("Native UI did not start its core; inspect logcat", CoreBridge::ready);
        await("Activity did not receive focus",()->inStage(androidx.test.runner.lifecycle.Stage.RESUMED,true));
        waitForAppWindow();
        heldResponseClassifiesTruncationAsTransportFailure();
        String raw = CoreBridge.config();
        assertNetworkProfile(raw);
        verifyFirstVpnConsent(context,raw);
        String secret = value(raw,"secret");
        int mixed = Integer.parseInt(value(raw,"mixed-port"));
        String controller = value(raw,"external-controller");
        assertTrue(controller.startsWith("127.0.0.1:"));
        java.net.Proxy proxy = new java.net.Proxy(java.net.Proxy.Type.HTTP,new InetSocketAddress("127.0.0.1",mixed));
        assertEquals("cor-android-network-ok", read(URI.create("http://10.0.2.2:18081/probe").toURL(),proxy,null));
        assertNotNull(new JSONObject(read(new URL("http://"+controller+"/version"),java.net.Proxy.NO_PROXY,secret)).getString("version"));
        assertNull("The real consent dialog must grant VPN permission", VpnService.prepare(context));
        noTunRoute();
        shell("cmd statusbar add-tile org.clashofrust.android/.VpnTileService");
        for (int cycle=0; cycle<2; cycle++) {
            if (cycle==0) TunnelService.start(context,false);
            else shell("cmd statusbar click-tile org.clashofrust.android/.VpnTileService");
            await("VPN did not connect; inspect logcat", () -> {
                if (!TunnelService.error().isEmpty()) throw new IOException(TunnelService.error());
                return TunnelService.connected();
            });
            ConnectivityManager connectivity=context.getSystemService(ConnectivityManager.class);
            await("Android did not make the VPN its active network", () -> vpnActive(connectivity));
            tunCarriesSocket(controller,secret);
            // A settings reload must preserve the independently owned Android TUN.
            CoreBridge.reload(CoreBridge.config());
            tunCarriesSocket(controller,secret);
            if (cycle == 0) {
                shell("am start -W -a android.intent.action.MAIN -c android.intent.category.HOME");
                await("VPN Activity did not enter background",()->inStage(androidx.test.runner.lifecycle.Stage.STOPPED,false));
                assertTrue("Closing the UI disconnected the foreground VPN", TunnelService.connected());
                assertTrue("Explicit disconnect must be remembered for sticky recovery",
                    context.getSharedPreferences("vpn",Context.MODE_PRIVATE).getBoolean("desired",false));
                tunResolvesAndCarriesHttp();
                shell("am start -W -n org.clashofrust.android/.MainActivity");
                await("VPN Activity did not resume",()->inStage(androidx.test.runner.lifecycle.Stage.RESUMED,true));
                java.util.concurrent.atomic.AtomicBoolean monitoring=new java.util.concurrent.atomic.AtomicBoolean(true);
                java.util.concurrent.atomic.AtomicBoolean bypassed=new java.util.concurrent.atomic.AtomicBoolean();
                java.util.concurrent.atomic.AtomicReference<Throwable> samplingError=new java.util.concurrent.atomic.AtomicReference<>();
                Thread routes=new Thread(()-> {
                    try {
                        while (monitoring.get()) {
                            Network active=connectivity.getActiveNetwork();
                            NetworkCapabilities capabilities=connectivity.getNetworkCapabilities(active);
                            if (active!=null && capabilities!=null && !capabilities.hasTransport(NetworkCapabilities.TRANSPORT_VPN)
                                && active.equals(connectivity.getActiveNetwork())) bypassed.set(true);
                            SystemClock.sleep(10);
                        }
                    } catch (Throwable error) {samplingError.set(error);}
                },"vpn-route-check");
                routes.setDaemon(true); routes.start();
                try {
                    MainActivity original=resumedActivity();
                    assertNotNull("VPN Activity was not resumed before recreation",original);
                    long generation=TunnelService.generation();
                    long uiGeneration=MainActivity.diagnostics().optLong("ui_generation");
                    String fixture=CoreBridge.config();
                    InstrumentationRegistry.getInstrumentation().runOnMainSync(original::recreate);
                    await("VPN Activity was not recreated",()-> {
                        MainActivity replacement=resumedActivity();
                        return replacement!=null && replacement!=original;
                    });
                    waitForAppWindow();
                    await("Recreated native UI did not render",()-> {
                        JSONObject ui=MainActivity.diagnostics();
                        return ui.optLong("ui_generation")>uiGeneration && ui.optDouble("width")>0;
                    });
                    assertTrue("Recreating the UI disconnected the VPN",TunnelService.connected());
                    assertEquals("Recreating the UI restarted the VPN",generation,TunnelService.generation());
                    assertTrue("Recreating the UI changed the active VPN route",vpnActive(connectivity));
                    assertTrue("Recreating the UI stopped the process-owned core",CoreBridge.ready());
                    assertEquals("Recreating the UI reloaded the running core",fixture,CoreBridge.config());
                    tunCarriesSocket(controller,secret);
                    for (int handover=0;handover<5;handover++) {
                        long previousGeneration = TunnelService.generation();
                        TunnelService.start(context,true);
                        await("VPN did not reconfigure",()->TunnelService.connected() && TunnelService.generation()>previousGeneration);
                        await("Android did not restore VPN routing after reconfigure",()->vpnActive(connectivity));
                        tunCarriesSocket(controller,secret);
                    }
                } finally { monitoring.set(false); routes.join(1000); }
                assertFalse("VPN route monitor did not finish",routes.isAlive());
                assertNull("Could not monitor VPN handover: "+samplingError.get(),samplingError.get());
                assertFalse("VPN reconfigure exposed the underlying network",bypassed.get());
            }
            tunCarriesSocket(controller,secret);
            if (cycle==0) TunnelService.stop(context);
            else shell("cmd statusbar click-tile org.clashofrust.android/.VpnTileService");
            await("VPN routes remained after disconnect", () -> {
                NetworkCapabilities capabilities=connectivity.getNetworkCapabilities(connectivity.getActiveNetwork());
                return !TunnelService.connected() && capabilities != null && !capabilities.hasTransport(NetworkCapabilities.TRANSPORT_VPN);
            });
            awaitDirectRecovery(connectivity);
            assertFalse("Explicit disconnect must not restart on sticky recovery",
                context.getSharedPreferences("vpn",Context.MODE_PRIVATE).getBoolean("desired",true));
        }
        assertNetworkProfile(CoreBridge.config());
        assertTrue("The generated runtime must survive a cold process",
            CoreBridge.config().equals(new String(java.nio.file.Files.readAllBytes(
                new File(context.getFilesDir(), "client/runtime/config.yaml").toPath()), StandardCharsets.UTF_8)));
    }
    @Test public void quickTileRestoresColdCoreWithoutOpeningTheActivity() throws Exception {
        Context context=InstrumentationRegistry.getInstrumentation().getTargetContext();
        assertFalse("Cold tile fixture must use a fresh process",CoreBridge.ready());
        assertTrue("Previous test must leave a persisted config",CoreBridge.restorable(context));
        assertNull("First-use test must already grant real VPN consent",VpnService.prepare(context));
        assertFalse(inStage(androidx.test.runner.lifecycle.Stage.RESUMED,false));
        // Android 14+ blocks application-side reads of this system-owned key,
        // even under instrumentation. Only the shell prepares the test fixture.
        String previousTiles=shell("settings get secure sysui_qs_tiles").trim();
        if (previousTiles.equals("null")) previousTiles=null;
        try {
            // A tile appended after the emulator's defaults may be off-screen.
            // Make it visible so this verifies the real, listening System UI tile.
            quickTiles(context,"custom("+new ComponentName(context,VpnTileService.class).flattenToString()+"),internet,bt");
            shell("cmd statusbar add-tile org.clashofrust.android/.VpnTileService");
            shell("cmd statusbar expand-settings");
            await("Quick tile was not rendered by System UI",()->tileState()!=-1);
            VpnTileService.refresh(context);
            await("Quick tile did not become visible",()->tileState()==android.service.quicksettings.Tile.STATE_INACTIVE);
            assertFalse("Listening to a tile must not start the core",CoreBridge.ready());
            shell("cmd statusbar click-tile org.clashofrust.android/.VpnTileService");
            await("Cold quick tile did not connect",()-> {
                if (!TunnelService.error().isEmpty()) throw new IOException(TunnelService.error());
                return TunnelService.connected();
            });
            assertTrue(CoreBridge.ready());
            if (Build.VERSION.SDK_INT >= 33)
                shell("pm grant org.clashofrust.android android.permission.POST_NOTIFICATIONS");
            await("Cold VPN notification did not show its actual mode and node", () ->
                vpnNotification(context).contains("规则 · DIRECT"));
            context.getSharedPreferences("appearance", Context.MODE_PRIVATE).edit().putBoolean("notification-speeds", false).commit();
            await("Hiding speeds also hid the VPN node or mode", () -> {
                String text = vpnNotification(context);
                return text.contains("规则 · DIRECT") && !text.contains("↓");
            });
            context.getSharedPreferences("appearance", Context.MODE_PRIVATE).edit().putBoolean("notification-speeds", true).commit();
            await("Cold VPN notification did not restore speed display", () -> vpnNotification(context).contains("↓"));
            shell("cmd statusbar expand-settings");
            await("Quick tile did not display its connected state",()->tileState()==android.service.quicksettings.Tile.STATE_ACTIVE);
            assertFalse("Quick tile opened the Activity",inStage(androidx.test.runner.lifecycle.Stage.RESUMED,false));
            assertFalse(inStage(androidx.test.runner.lifecycle.Stage.STARTED,false));
            assertFalse(inStage(androidx.test.runner.lifecycle.Stage.CREATED,false));
            String raw=CoreBridge.config();
            assertNetworkProfile(raw);
            tunCarriesSocket(value(raw,"external-controller"),value(raw,"secret"));
            shell("cmd statusbar click-tile org.clashofrust.android/.VpnTileService");
            await("Cold tile did not disconnect",()->!TunnelService.connected());
            await("VPN notification remained after disconnect", () -> vpnNotification(context).isEmpty());
            await("Quick tile remained visually enabled after disconnect",()->tileState()==android.service.quicksettings.Tile.STATE_INACTIVE);
            noTunRoute();
            assertFalse(inStage(androidx.test.runner.lifecycle.Stage.RESUMED,false));
        } finally {
            try { shell("cmd statusbar collapse"); }
            finally {
                try { quickTiles(context,previousTiles); }
                finally { TunnelService.stop(context); }
            }
        }
    }
    private static String vpnNotification(Context context) {
        android.app.NotificationManager manager = context.getSystemService(android.app.NotificationManager.class);
        for (android.service.notification.StatusBarNotification entry : manager.getActiveNotifications()) {
            if (entry.getId() == 1) {
                CharSequence text = entry.getNotification().extras.getCharSequence(android.app.Notification.EXTRA_TEXT);
                CharSequence title = entry.getNotification().extras.getCharSequence(android.app.Notification.EXTRA_TITLE);
                return (title == null ? "" : title.toString()) + "\n" + (text == null ? "" : text.toString());
            }
        }
        return "";
    }
    private static void quickTiles(Context context,String value) {
        if (Build.VERSION.SDK_INT<29) throw new AssertionError("Tile UI regression requires Android 10+");
        android.app.UiAutomation automation=InstrumentationRegistry.getInstrumentation().getUiAutomation();
        automation.adoptShellPermissionIdentity("android.permission.WRITE_SECURE_SETTINGS");
        try {
            assertTrue("Cannot prepare visible quick tile",android.provider.Settings.Secure.putString(
                context.getContentResolver(),"sysui_qs_tiles",value));
        } finally { automation.dropShellPermissionIdentity(); }
    }
    private static int tileState() {
        // Active TileService bindings may end immediately after updateTile().
        // Inspect the actual toggle rendered by System UI, not service lifetime.
        android.view.accessibility.AccessibilityNodeInfo root=InstrumentationRegistry.getInstrumentation()
            .getUiAutomation().getRootInActiveWindow();
        if (root==null || !"com.android.systemui".contentEquals(root.getPackageName())) return -1;
        for (android.view.accessibility.AccessibilityNodeInfo label:root.findAccessibilityNodeInfosByText("Clash of Rust")) {
            for (android.view.accessibility.AccessibilityNodeInfo view=label;view!=null;view=view.getParent()) {
                if (view.isCheckable()) return !view.isEnabled() || !view.isClickable()?android.service.quicksettings.Tile.STATE_UNAVAILABLE
                    :checked(view)?android.service.quicksettings.Tile.STATE_ACTIVE:android.service.quicksettings.Tile.STATE_INACTIVE;
            }
        }
        return -1;
    }
}
