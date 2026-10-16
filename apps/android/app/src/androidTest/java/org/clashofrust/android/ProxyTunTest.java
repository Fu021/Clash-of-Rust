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
    private static void shell(String command) throws Exception {
        try (InputStream in=new android.os.ParcelFileDescriptor.AutoCloseInputStream(
            InstrumentationRegistry.getInstrumentation().getUiAutomation().executeShellCommand(command))) {
            byte[] bytes=new byte[1024];while (in.read(bytes)!=-1) { }
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
        if (secret != null) connection.setRequestProperty("Authorization", "Bearer " + secret);
        try (InputStream in = connection.getInputStream()) {
            ByteArrayOutputStream out = new ByteArrayOutputStream();
            byte[] buffer = new byte[4096]; int n;
            while ((n=in.read(buffer))!=-1) out.write(buffer,0,n);
            return out.toString(StandardCharsets.UTF_8.name());
        } finally { connection.disconnect(); }
    }
    @Test public void localProxyAndRealTunCarryTrafficAndDisconnectCleanly() throws Exception {
        Context context = InstrumentationRegistry.getInstrumentation().getTargetContext();
        context.startActivity(new Intent(context, MainActivity.class).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK));
        await("Native UI did not start its core; inspect logcat", CoreBridge::ready);
        await("Activity did not receive focus",()->inStage(androidx.test.runner.lifecycle.Stage.RESUMED,true));
        waitForAppWindow();
        verifyPresentation();
        String raw = CoreBridge.config();
        String secret = value(raw,"secret");
        int mixed = Integer.parseInt(value(raw,"mixed-port"));
        String controller = value(raw,"external-controller");
        assertTrue(controller.startsWith("127.0.0.1:"));
        java.net.Proxy proxy = new java.net.Proxy(java.net.Proxy.Type.HTTP,new InetSocketAddress("127.0.0.1",mixed));
        assertEquals("cor-android-network-ok", read(new URL("http://10.0.2.2:18081/probe"),proxy,null));
        assertNotNull(new JSONObject(read(new URL("http://"+controller+"/version"),java.net.Proxy.NO_PROXY,secret)).getString("version"));
        assertNull("CI must grant VPN consent through Android appops", VpnService.prepare(context));
        CoreBridge.reload(raw + "\nhosts:\n  android-tun.test: 10.0.2.2\n");
        shell("cmd statusbar add-tile org.clashofrust.android/.VpnTileService");
        for (int cycle=0; cycle<2; cycle++) {
            if (cycle==0) context.startForegroundService(new Intent(context,TunnelService.class));
            else shell("cmd statusbar click-tile org.clashofrust.android/.VpnTileService");
            await("VPN did not connect; inspect logcat", () -> {
                if (!TunnelService.error().isEmpty()) throw new IOException(TunnelService.error());
                return TunnelService.connected();
            });
            ConnectivityManager connectivity=context.getSystemService(ConnectivityManager.class);
            await("Android did not make the VPN its active network", () -> {
                NetworkCapabilities capabilities=connectivity.getNetworkCapabilities(connectivity.getActiveNetwork());
                return capabilities!=null && capabilities.hasTransport(NetworkCapabilities.TRANSPORT_VPN);
            });
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
                context.startForegroundService(new Intent(context,TunnelService.class).setAction(TunnelService.RECONFIGURE));
                await("VPN did not reconfigure",()->TunnelService.connected() && TunnelService.generation()>previousGeneration);
            }
            // Open a fresh OS socket after each VPN rebuild. HttpURLConnection
            // can reuse a pooled connection created on the previous network.
            try (Socket connection = new Socket(java.net.Proxy.NO_PROXY)) {
                connection.connect(new InetSocketAddress("android-tun.test",18081),5000);
                connection.setSoTimeout(5000);
                connection.getOutputStream().write(("GET /hold HTTP/1.1\r\nHost: android-tun.test\r\nConnection: close\r\n\r\n")
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
                JSONObject records = new JSONObject(read(new URL("http://"+controller+"/connections"),java.net.Proxy.NO_PROXY,secret));
                JSONArray connections = records.getJSONArray("connections");
                boolean observed=false;
                for (int i=0; i<connections.length(); i++) {
                    JSONObject metadata=connections.getJSONObject(i).getJSONObject("metadata");
                    // Android may reuse a DNS result across VPN generations,
                    // so host can be empty. Match this exact live socket instead.
                    if (metadata.optString("type").equalsIgnoreCase("Tun")
                        && metadata.optInt("sourcePort")==connection.getLocalPort()
                        && metadata.optInt("destinationPort")==18081) observed=true;
                }
                assertTrue("No real TUN connection was observed for port "+connection.getLocalPort()
                    +"; a direct-only test must fail; records="+records,observed);
            }
            if (cycle==0) TunnelService.stop(context);
            else shell("cmd statusbar click-tile org.clashofrust.android/.VpnTileService");
            await("VPN routes remained after disconnect", () -> {
                NetworkCapabilities capabilities=connectivity.getNetworkCapabilities(connectivity.getActiveNetwork());
                return !TunnelService.connected() && capabilities != null && !capabilities.hasTransport(NetworkCapabilities.TRANSPORT_VPN);
            });
            assertEquals("cor-android-network-ok",read(new URL("http://10.0.2.2:18081/probe"),java.net.Proxy.NO_PROXY,null));
            assertFalse("Explicit disconnect must not restart on sticky recovery",
                context.getSharedPreferences("vpn",Context.MODE_PRIVATE).getBoolean("desired",true));
        }
    }
}
