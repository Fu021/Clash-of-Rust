package org.clashofrust.android;

import android.content.Context;
import android.net.*;
import java.net.InetAddress;
import java.util.*;
import java.util.concurrent.atomic.AtomicLong;
import org.json.JSONArray;
import org.json.JSONObject;

final class NetworkOverview implements AutoCloseable {
    private final ConnectivityManager manager;
    private final Map<Network,LinkProperties> links = new LinkedHashMap<>();
    private final AtomicLong revision = new AtomicLong();
    private final ConnectivityManager.NetworkCallback callback = new ConnectivityManager.NetworkCallback() {
        @Override public void onLinkPropertiesChanged(Network network, LinkProperties value) {
            synchronized (links) { links.put(network, value); }
            revision.incrementAndGet();
        }
        @Override public void onLost(Network network) {
            synchronized (links) { links.remove(network); }
            revision.incrementAndGet();
        }
    };

    NetworkOverview(Context context) {
        manager = context.getSystemService(ConnectivityManager.class);
        manager.registerNetworkCallback(new NetworkRequest.Builder()
            .addCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
            .addCapability(NetworkCapabilities.NET_CAPABILITY_NOT_VPN).build(), callback);
    }

    long revision() { return revision.get(); }

    private static void add(Set<String> values, InetAddress address) {
        if (address.isAnyLocalAddress() || address.isLoopbackAddress()
            || address.isLinkLocalAddress() || address.isMulticastAddress()) return;
        String text = address.getHostAddress();
        if (text == null) return;
        int scope = text.indexOf('%');
        values.add(scope < 0 ? text : text.substring(0, scope));
    }

    JSONObject read() throws Exception {
        Set<String> addresses = new LinkedHashSet<>(), dns = new LinkedHashSet<>();
        Network active = manager.getActiveNetwork();
        synchronized (links) {
            List<Map.Entry<Network,LinkProperties>> ordered = new ArrayList<>(links.entrySet());
            ordered.sort(Comparator.comparing(entry -> !entry.getKey().equals(active)));
            for (Map.Entry<Network,LinkProperties> entry : ordered) {
                NetworkCapabilities capabilities = manager.getNetworkCapabilities(entry.getKey());
                if (capabilities == null || capabilities.hasTransport(NetworkCapabilities.TRANSPORT_VPN)) continue;
                for (LinkAddress address : entry.getValue().getLinkAddresses()) add(addresses, address.getAddress());
                for (InetAddress address : entry.getValue().getDnsServers()) add(dns, address);
            }
        }
        return new JSONObject().put("addresses", new JSONArray(new ArrayList<>(addresses)))
            .put("dns", new JSONArray(new ArrayList<>(dns)));
    }

    @Override public void close() { manager.unregisterNetworkCallback(callback); }
}
