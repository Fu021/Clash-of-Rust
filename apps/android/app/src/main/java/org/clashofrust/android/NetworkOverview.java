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

    JSONObject diagnostics() throws Exception {
        Network active = manager.getActiveNetwork();
        NetworkCapabilities current = active == null ? null : manager.getNetworkCapabilities(active);
        JSONArray networks = new JSONArray();
        synchronized (links) {
            for (Map.Entry<Network,LinkProperties> entry : links.entrySet()) {
                if (networks.length() >= 8) break;
                NetworkCapabilities capabilities = manager.getNetworkCapabilities(entry.getKey());
                if (capabilities == null || capabilities.hasTransport(NetworkCapabilities.TRANSPORT_VPN)) continue;
                LinkProperties properties = entry.getValue();
                int dns4 = 0, dns6 = 0;
                for (InetAddress address : properties.getDnsServers()) {
                    if (address instanceof java.net.Inet6Address) dns6++; else dns4++;
                }
                JSONObject record = new JSONObject()
                    .put("wifi", capabilities.hasTransport(NetworkCapabilities.TRANSPORT_WIFI))
                    .put("cellular", capabilities.hasTransport(NetworkCapabilities.TRANSPORT_CELLULAR))
                    .put("ethernet", capabilities.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET))
                    .put("validated", capabilities.hasCapability(NetworkCapabilities.NET_CAPABILITY_VALIDATED))
                    .put("dns_ipv4", dns4).put("dns_ipv6", dns6).put("mtu", properties.getMtu());
                if (android.os.Build.VERSION.SDK_INT >= 28) {
                    record.put("private_dns_active", properties.isPrivateDnsActive())
                        .put("private_dns_strict", properties.getPrivateDnsServerName() != null);
                }
                networks.put(record);
            }
        }
        return new JSONObject().put("schema", 1)
            .put("active_available", current != null)
            .put("active_vpn", current != null && current.hasTransport(NetworkCapabilities.TRANSPORT_VPN))
            .put("active_validated", current != null && current.hasCapability(NetworkCapabilities.NET_CAPABILITY_VALIDATED))
            .put("physical_networks", networks);
    }

    @Override public void close() { manager.unregisterNetworkCallback(callback); }
}
