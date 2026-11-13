package org.clashofrust.android;

import android.os.Process;
import android.os.SystemClock;
import java.io.ByteArrayOutputStream;
import java.io.InputStream;
import java.net.HttpURLConnection;
import java.net.Proxy;
import java.net.URL;
import java.nio.charset.StandardCharsets;
import org.json.JSONArray;
import org.json.JSONObject;

public final class ApplicationUidProbe {
    public static void main(String[] arguments) throws Exception {
        if (arguments.length != 0) throw new IllegalArgumentException("No arguments expected");
        System.out.println(request(new URL("http://198.51.100.1:18081/probe"), 15000));
    }

    static JSONObject request(URL target, long timeout) throws Exception {
        JSONObject result = new JSONObject().put("uid", Process.myUid());
        JSONArray attempts = new JSONArray();
        result.put("attempts", attempts);
        long deadline = SystemClock.elapsedRealtime() + timeout;
        do {
            long started = SystemClock.elapsedRealtime();
            JSONObject attempt = new JSONObject();
            attempts.put(attempt);
            HttpURLConnection connection = null;
            try {
                connection = (HttpURLConnection) target.openConnection(Proxy.NO_PROXY);
                int budget = (int)Math.max(1, Math.min(5000, (deadline - started) / 2));
                connection.setConnectTimeout(budget);
                connection.setReadTimeout(budget);
                connection.setUseCaches(false);
                connection.setInstanceFollowRedirects(false);
                int status = connection.getResponseCode();
                attempt.put("status", status);
                if (status != 200) return result.put("status", status);
                try (InputStream input = connection.getInputStream();
                        ByteArrayOutputStream output = new ByteArrayOutputStream()) {
                    byte[] buffer = new byte[1024];
                    int count;
                    while ((count = input.read(buffer)) != -1) {
                        if (output.size() + count > 16384) throw new IllegalStateException("Response exceeds limit");
                        output.write(buffer, 0, count);
                        connection.setReadTimeout((int)Math.max(1, Math.min(5000,
                            deadline - SystemClock.elapsedRealtime())));
                        if (SystemClock.elapsedRealtime() >= deadline)
                            throw new java.net.SocketTimeoutException("Probe deadline exceeded");
                    }
                    if (connection.getContentLengthLong() >= 0
                            && connection.getContentLengthLong() != output.size())
                        throw new java.io.EOFException("Incomplete probe response");
                    return result.put("status", status).put("body", output.toString(StandardCharsets.UTF_8.name()));
                }
            } catch (java.net.SocketException | java.net.SocketTimeoutException | java.io.EOFException transientError) {
                attempt.put("error", transientError.getClass().getSimpleName());
            } catch (java.net.ProtocolException invalid) {
                if (!"unexpected end of stream".equals(invalid.getMessage()))
                    return result.put("error", invalid.getClass().getSimpleName());
                attempt.put("error", invalid.getClass().getSimpleName());
            } catch (Exception error) {
                return result.put("error", error.getClass().getSimpleName());
            } finally {
                attempt.put("elapsed_ms", SystemClock.elapsedRealtime() - started);
                if (connection != null) connection.disconnect();
            }
            SystemClock.sleep(Math.min(100, Math.max(0, deadline - SystemClock.elapsedRealtime())));
        } while (SystemClock.elapsedRealtime() < deadline && attempts.length() < 16);
        return result.put("error", "DataPathDeadlineExceeded");
    }
}
