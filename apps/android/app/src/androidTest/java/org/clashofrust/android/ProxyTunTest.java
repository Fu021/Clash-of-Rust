package org.clashofrust.android;

import android.content.*;
import android.net.*;
import android.os.Build;
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
    private interface Condition { boolean ready() throws Exception; }
    private static void await(String message, Condition condition) throws Exception {
        long deadline = System.nanoTime() + 30_000_000_000L;
        while (System.nanoTime() < deadline) {
            if (condition.ready()) return;
            Thread.sleep(100);
        }
        fail(message);
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
        for (int cycle=0; cycle<2; cycle++) {
            context.startForegroundService(new Intent(context,TunnelService.class));
            await("VPN did not connect: "+TunnelService.error(), TunnelService::connected);
            // This request has NO explicit proxy. It must traverse the OS TUN.
            HttpURLConnection connection = (HttpURLConnection)new URL("http://android-tun.test:18081/hold").openConnection(java.net.Proxy.NO_PROXY);
            connection.setConnectTimeout(5000); connection.setReadTimeout(5000);
            try (InputStream stream = connection.getInputStream()) {
                assertTrue(stream.read() >= 0);
                JSONObject records = new JSONObject(read(new URL("http://"+controller+"/connections"),java.net.Proxy.NO_PROXY,secret));
                JSONArray connections = records.getJSONArray("connections");
                boolean observed=false;
                for (int i=0; i<connections.length(); i++) {
                    JSONObject metadata=connections.getJSONObject(i).getJSONObject("metadata");
                    if (metadata.optString("type").equalsIgnoreCase("Tun")
                        && metadata.optString("host").equals("android-tun.test")) observed=true;
                }
                assertTrue("No real TUN connection was observed; a direct-only test must fail",observed);
            } finally { connection.disconnect(); }
            TunnelService.stop(context);
            ConnectivityManager connectivity=context.getSystemService(ConnectivityManager.class);
            await("VPN routes remained after disconnect", () -> {
                NetworkCapabilities capabilities=connectivity.getNetworkCapabilities(connectivity.getActiveNetwork());
                return !TunnelService.connected() && capabilities != null && !capabilities.hasTransport(NetworkCapabilities.TRANSPORT_VPN);
            });
            assertEquals("cor-android-network-ok",read(new URL("http://10.0.2.2:18081/probe"),java.net.Proxy.NO_PROXY,null));
        }
    }
}
