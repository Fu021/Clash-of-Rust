package org.clashofrust.android;

import android.net.*;
import android.content.Context;
import java.io.IOException;
import java.net.Inet4Address;
import java.net.Inet6Address;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;

/** Wait for ordered system callbacks, rather than an old VPN capability snapshot. */
final class VpnRouting extends ConnectivityManager.NetworkCallback implements AutoCloseable {
    private final ConnectivityManager manager;
    private final String previousInterface;
    private final boolean ipv6;
    private final CountDownLatch ready=new CountDownLatch(1);
    private Network network;
    private NetworkCapabilities capabilities;
    private LinkProperties links;
    private volatile String interfaceName;

    VpnRouting(Context context,String previousInterface,boolean ipv6) {
        manager=context.getSystemService(ConnectivityManager.class);
        this.previousInterface=previousInterface;
        this.ipv6=ipv6;
        manager.registerDefaultNetworkCallback(this);
    }
    @Override public synchronized void onAvailable(Network value) {
        network=value;capabilities=null;links=null;
    }
    @Override public synchronized void onCapabilitiesChanged(Network value,NetworkCapabilities snapshot) {
        if (!value.equals(network)) return;
        capabilities=snapshot;check();
    }
    @Override public synchronized void onLinkPropertiesChanged(Network value,LinkProperties snapshot) {
        if (!value.equals(network)) return;
        links=snapshot;check();
    }
    @Override public synchronized void onLost(Network value) {
        if (value.equals(network)) {network=null;capabilities=null;links=null;}
    }
    private void check() {
        if (capabilities==null || links==null || !capabilities.hasTransport(NetworkCapabilities.TRANSPORT_VPN)) return;
        String name=links.getInterfaceName();
        if (name==null || name.isEmpty() || name.equals(previousInterface)) return;
        boolean address4=false,address6=false,route4=false,route6=false;
        for (LinkAddress address:links.getLinkAddresses()) {
            if (address.getAddress() instanceof Inet4Address
                && "198.18.0.1".equals(address.getAddress().getHostAddress())) address4=true;
            if (address.getAddress() instanceof Inet6Address) address6=true;
        }
        for (RouteInfo route:links.getRoutes()) {
            if (!route.isDefaultRoute() || !name.equals(route.getInterface())) continue;
            if (route.getDestination().getAddress() instanceof Inet4Address) route4=true;
            if (route.getDestination().getAddress() instanceof Inet6Address) route6=true;
        }
        if (address4 && route4 && (!ipv6 || (address6 && route6))) {
            interfaceName=name;
            ready.countDown();
        }
    }
    String await(java.util.function.BooleanSupplier cancelled) throws IOException,InterruptedException {
        try {
            long deadline=System.nanoTime()+TimeUnit.SECONDS.toNanos(10);
            while (!ready.await(100,TimeUnit.MILLISECONDS)) {
                if (cancelled.getAsBoolean()) throw new java.util.concurrent.CancellationException("VPN 连接已取消");
                if (System.nanoTime()>=deadline) throw new IOException("系统尚未完成 VPN 路由切换");
            }
            if (cancelled.getAsBoolean()) throw new java.util.concurrent.CancellationException("VPN 连接已取消");
        } catch (InterruptedException error) {Thread.currentThread().interrupt();throw error;}
        return interfaceName;
    }
    @Override public void close() {
        try {manager.unregisterNetworkCallback(this);}
        catch (IllegalArgumentException removed) {
            android.util.Log.w("ClashOfRust","VPN readiness callback was already removed",removed);
        }
    }
}
