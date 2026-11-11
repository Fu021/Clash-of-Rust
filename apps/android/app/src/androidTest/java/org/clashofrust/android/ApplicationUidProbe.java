package org.clashofrust.android;

import android.os.Process;
import java.io.ByteArrayOutputStream;
import java.io.InputStream;
import java.net.HttpURLConnection;
import java.net.Proxy;
import java.net.URL;
import java.nio.charset.StandardCharsets;
import org.json.JSONObject;

public final class ApplicationUidProbe {
    public static void main(String[] arguments) throws Exception {
        JSONObject result = new JSONObject().put("uid", Process.myUid());
        HttpURLConnection connection = null;
        try {
            if (arguments.length != 0) throw new IllegalArgumentException("No arguments expected");
            connection = (HttpURLConnection) new URL("http://198.51.100.1:18081/probe")
                .openConnection(Proxy.NO_PROXY);
            connection.setConnectTimeout(5000);
            connection.setReadTimeout(5000);
            connection.setUseCaches(false);
            connection.setInstanceFollowRedirects(false);
            result.put("status", connection.getResponseCode());
            try (InputStream input = connection.getInputStream();
                    ByteArrayOutputStream output = new ByteArrayOutputStream()) {
                byte[] buffer = new byte[1024];
                int count;
                while ((count = input.read(buffer)) != -1) {
                    if (output.size() + count > 16384) throw new IllegalStateException("Response exceeds limit");
                    output.write(buffer, 0, count);
                }
                result.put("body", output.toString(StandardCharsets.UTF_8.name()));
            }
        } catch (Exception error) {
            result.put("error", error.getClass().getSimpleName());
        } finally {
            if (connection != null) connection.disconnect();
        }
        System.out.println(result);
    }
}
