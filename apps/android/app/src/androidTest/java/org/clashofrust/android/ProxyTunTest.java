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
    private static void pointer(int action,float x,float y,long down,long now) {
        MotionEvent event=MotionEvent.obtain(down,now,action,x,y,0);
        event.setSource(android.view.InputDevice.SOURCE_TOUCHSCREEN);
        try { assertTrue("Android rejected touch input", InstrumentationRegistry.getInstrumentation().getUiAutomation().injectInputEvent(event,true)); }
        finally {event.recycle();}
    }
    private static void tap(float x,float y) {
        long start=SystemClock.uptimeMillis();
        pointer(MotionEvent.ACTION_DOWN,x,y,start,start);
        SystemClock.sleep(50);
        pointer(MotionEvent.ACTION_UP,x,y,start,SystemClock.uptimeMillis());
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
            float scale=(float)ui.getDouble("scale");
            tap((float)ui.getDouble("width")*(index+0.5f)/5*scale,
                ((float)ui.getDouble("height")-(float)ui.getDouble("safe_bottom")-38)*scale);
            SystemClock.sleep(500);
            return MainActivity.diagnostics().optInt("page",-1)==page;
        });
    }
    private static void verifyPresentation() throws Exception {
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
        await("Clash window is not the active input window",()-> {
            android.view.accessibility.AccessibilityNodeInfo root=automation.getRootInActiveWindow();
            return root!=null && "org.clashofrust.android".contentEquals(root.getPackageName());
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
    private static void consent(boolean allow) throws Exception {
        await("Android VPN consent dialog was not actionable",()-> {
            android.view.accessibility.AccessibilityNodeInfo root=InstrumentationRegistry.getInstrumentation()
                .getUiAutomation().getRootInActiveWindow();
            if (root==null || !"com.android.vpndialogs".contentEquals(root.getPackageName())) return false;
            for (android.view.accessibility.AccessibilityNodeInfo check:root.findAccessibilityNodeInfosByViewId("android:id/checkbox"))
                if (check.isCheckable() && !check.isChecked())
                    check.performAction(android.view.accessibility.AccessibilityNodeInfo.ACTION_CLICK);
            for (android.view.accessibility.AccessibilityNodeInfo button:root.findAccessibilityNodeInfosByViewId(
                allow ? "android:id/button1" : "android:id/button2"))
                if (button.isEnabled()) return button.performAction(android.view.accessibility.AccessibilityNodeInfo.ACTION_CLICK);
            return false;
        });
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
    }
    private static void tunCarriesSocket(String controller,String secret) throws Exception {
        // NetworkCallbacks describe the control plane. A just-replaced VPN can
        // still be waiting for netd's UID routes; require fresh real traffic,
        // with a bounded deadline, rather than sleeping or rerunning the test.
        long deadline=SystemClock.elapsedRealtime()+15000;
        IOException last=null;
        do {
            try { assertTunSocket(controller,secret); return; }
            catch (IOException transition) { last=transition; }
            SystemClock.sleep(100);
        } while (SystemClock.elapsedRealtime()<deadline);
        throw new IOException("VPN data path did not become ready; "+networkState(),last);
    }
    private static String networkState() {
        ConnectivityManager manager=InstrumentationRegistry.getInstrumentation().getTargetContext()
            .getSystemService(ConnectivityManager.class);
        Network current=manager.getActiveNetwork();
        return "active="+current+"; capabilities="+manager.getNetworkCapabilities(current)
            +"; links="+manager.getLinkProperties(current)+"; VPN="+TunnelService.connected()
            +"; generation="+TunnelService.generation()+"; error="+TunnelService.error();
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
                    assertEquals("cor-android-network-ok",read(new URL("http://10.0.2.2:18081/probe"),java.net.Proxy.NO_PROXY,null));
                    if (network.equals(manager.getActiveNetwork())) { noTunRoute(); return; }
                } catch (IOException transition) { last=transition; }
            }
            SystemClock.sleep(100);
        } while (SystemClock.elapsedRealtime()<deadline);
        throw new IOException("Default network did not recover after VPN disconnect; "+networkState(),last);
    }
    private static void assertTunSocket(String controller,String secret) throws Exception {
        // The physical network cannot reach this reserved address. Only the
        // test core's explicit HTTP outbound can relay it to the host origin.
        // Keep a fresh socket open and require its exact TUN source port too.
        try (Socket connection=new Socket(java.net.Proxy.NO_PROXY)) {
            connection.connect(new InetSocketAddress("198.51.100.1",18081),5000);
            connection.setSoTimeout(5000);
            connection.getOutputStream().write(("GET /hold HTTP/1.1\r\nHost: 198.51.100.1\r\nConnection: close\r\n\r\n")
                .getBytes(StandardCharsets.US_ASCII));
            BufferedReader stream=new BufferedReader(new InputStreamReader(connection.getInputStream(),StandardCharsets.US_ASCII));
            String status=stream.readLine();
            assertNotNull("Held TUN request returned no HTTP response",status);
            assertTrue("Held TUN request failed: "+status,status.contains(" 200 "));
            boolean headersComplete=false;
            for (int header=0;header<100;header++) {
                String line=stream.readLine();
                if (line==null) break;
                if (line.isEmpty()) {headersComplete=true;break;}
            }
            assertTrue("Incomplete held HTTP response",headersComplete);
            assertTrue(stream.read() >= 0);
            JSONObject records=new JSONObject(read(new URL("http://"+controller+"/connections"),java.net.Proxy.NO_PROXY,secret));
            JSONArray connections=records.getJSONArray("connections");
            boolean observed=false;
            for (int i=0;i<connections.length();i++) {
                JSONObject record=connections.getJSONObject(i);
                JSONObject metadata=record.getJSONObject("metadata");
                if (metadata.optString("type").equalsIgnoreCase("Tun")
                    && metadata.optInt("sourcePort")==connection.getLocalPort()
                    && metadata.optInt("destinationPort")==18081
                    && record.getJSONArray("chains").toString().contains("CI-TUN-origin")) observed=true;
            }
            assertTrue("No matching TUN connection: "+connection.getLocalSocketAddress()+" -> "+connection.getRemoteSocketAddress()
                +"; a direct-only test must fail; records="+records,observed);
        }
    }
    @Test public void localProxyAndRealTunCarryTrafficAndDisconnectCleanly() throws Exception {
        Context context = InstrumentationRegistry.getInstrumentation().getTargetContext();
        // A light emulator must still launch an explicitly dark app correctly
        // on its first frame, before the core/settings worker is ready.
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
        String raw = CoreBridge.config();
        verifyFirstVpnConsent(context,raw);
        String secret = value(raw,"secret");
        int mixed = Integer.parseInt(value(raw,"mixed-port"));
        String controller = value(raw,"external-controller");
        assertTrue(controller.startsWith("127.0.0.1:"));
        java.net.Proxy proxy = new java.net.Proxy(java.net.Proxy.Type.HTTP,new InetSocketAddress("127.0.0.1",mixed));
        assertEquals("cor-android-network-ok", read(new URL("http://10.0.2.2:18081/probe"),proxy,null));
        assertNotNull(new JSONObject(read(new URL("http://"+controller+"/version"),java.net.Proxy.NO_PROXY,secret)).getString("version"));
        assertNull("The real consent dialog must grant VPN permission", VpnService.prepare(context));
        noTunRoute();
        CoreBridge.reload("mixed-port: "+mixed+"\nexternal-controller: "+controller+"\nsecret: "+JSONObject.quote(secret)
            +"\nmode: rule\nipv6: true\nfind-process-mode: off\n"
            +"dns:\n  enable: true\n  enhanced-mode: fake-ip\n  nameserver: [https://1.1.1.1/dns-query]\n"
            +"hosts:\n  android-tun.test: 10.0.2.2\n"
            +"proxies:\n  - {name: CI-TUN-origin, type: http, server: 10.0.2.2, port: 18081}\n"
            +"rules:\n  - IP-CIDR,198.51.100.1/32,CI-TUN-origin,no-resolve\n  - MATCH,DIRECT\n");
        shell("cmd statusbar add-tile org.clashofrust.android/.VpnTileService");
        for (int cycle=0; cycle<2; cycle++) {
            if (cycle==0) context.startForegroundService(new Intent(context,TunnelService.class));
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
            if (cycle == 0) {
                shell("am start -W -a android.intent.action.MAIN -c android.intent.category.HOME");
                await("VPN Activity did not enter background",()->inStage(androidx.test.runner.lifecycle.Stage.STOPPED,false));
                assertTrue("Closing the UI disconnected the foreground VPN", TunnelService.connected());
                assertTrue("Explicit disconnect must be remembered for sticky recovery",
                    context.getSharedPreferences("vpn",Context.MODE_PRIVATE).getBoolean("desired",false));
                assertEquals("cor-android-network-ok", read(new URL("http://android-tun.test:18081/probe"),java.net.Proxy.NO_PROXY,null));
                shell("am start -W -n org.clashofrust.android/.MainActivity");
                await("VPN Activity did not resume",()->inStage(androidx.test.runner.lifecycle.Stage.RESUMED,true));
                long previousGeneration = TunnelService.generation();
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
                    context.startForegroundService(new Intent(context,TunnelService.class).setAction(TunnelService.RECONFIGURE));
                    await("VPN did not reconfigure",()->TunnelService.connected() && TunnelService.generation()>previousGeneration);
                    await("Android did not restore VPN routing after reconfigure",()->vpnActive(connectivity));
                    tunCarriesSocket(controller,secret);
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
        // The next instrumentation starts in a fresh process and must restore
        // this known test route without opening the Activity.
        java.nio.file.Files.write(new File(context.getFilesDir(),"client/runtime/config.yaml").toPath(),
            CoreBridge.config().getBytes(StandardCharsets.UTF_8));
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
            shell("cmd statusbar expand-settings");
            await("Quick tile did not display its connected state",()->tileState()==android.service.quicksettings.Tile.STATE_ACTIVE);
            assertFalse("Quick tile opened the Activity",inStage(androidx.test.runner.lifecycle.Stage.RESUMED,false));
            assertFalse(inStage(androidx.test.runner.lifecycle.Stage.STARTED,false));
            assertFalse(inStage(androidx.test.runner.lifecycle.Stage.CREATED,false));
            String raw=CoreBridge.config();
            tunCarriesSocket(value(raw,"external-controller"),value(raw,"secret"));
            shell("cmd statusbar click-tile org.clashofrust.android/.VpnTileService");
            await("Cold tile did not disconnect",()->!TunnelService.connected());
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
                    :view.isChecked()?android.service.quicksettings.Tile.STATE_ACTIVE:android.service.quicksettings.Tile.STATE_INACTIVE;
            }
        }
        return -1;
    }
}
