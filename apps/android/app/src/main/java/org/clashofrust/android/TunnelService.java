package org.clashofrust.android;

import android.app.*;
import android.content.*;
import android.net.VpnService;
import android.os.*;
import java.io.File;
import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.LinkOption;
import java.nio.file.StandardCopyOption;
import java.util.HashMap;
import java.util.Map;
import java.util.concurrent.ScheduledExecutorService;
import java.util.concurrent.ScheduledFuture;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.Executors;
import java.util.function.LongSupplier;
import org.clashofrust.nativecore.mobile.Mobile;
import org.json.JSONObject;

/** The foreground service owns the TUN; closing the window leaves it connected. */
public final class TunnelService extends VpnService {
    static final String RECONFIGURE = "org.clashofrust.android.RECONFIGURE";
    static final String REQUEST = "vpn_request";
    private static final String NOTIFICATION_CHANNEL = "vpn";
    private static final Object requests = new Object();
    private static final Map<String, Integer> pendingStarts = new HashMap<>();
    private static volatile TunnelService instance;
    private static volatile boolean connected;
    private static volatile String lastError = "";
    private static volatile long generation;
    private static String queuedRequest;
    private static String recoveryReservation;
    private static String rollbackRequest;
    private static String rollbackStatus;
    private static String rollbackError;
    private static Boolean rollbackLive;
    private static long rollbackDeadline;
    private static boolean rollbackConsentReady;
    private static boolean rollbackStartSent;
    private static boolean rollbackWorkClaimed;
    private static int pendingVpnWork;
    private static long rollbackPreviousGeneration;
    private static long rollbackGeneration;
    private static String completedRecovery;
    private static int pendingCleanup;
    private static long lastApplicationMissingRejected;
    private static String notificationChannelName;
    // One process-wide queue also orders cleanup against a newly created service.
    private static final ScheduledExecutorService serial = Executors.newSingleThreadScheduledExecutor();
    private ScheduledFuture<?> notificationTask;
    private ParcelFileDescriptor descriptor;
    private String interfaceName;
    private boolean interfaceIpv6;
    private volatile boolean forwardingHeld;
    private volatile boolean stopping;
    private boolean idleStopQueued;
    private volatile String lastStatus = "";
    private JSONObject networkStatus = new JSONObject();
    private volatile int latestStartId;
    private volatile String acceptedRequest;
    static String prepareStart(Context context) {
        synchronized (requests) {
            return recoveryBlocked(context) ? null : newRequest(context);
        }
    }
    private static File recoveryRecord(Context context) {
        return new File(context.getFilesDir(), "client/protection-recovery.json");
    }
    static boolean recoveryRequired(Context context) {
        return !Files.notExists(recoveryRecord(context).toPath(), LinkOption.NOFOLLOW_LINKS);
    }
    private static boolean recoveryBlocked(Context context) {
        return recoveryReservation != null || recoveryRequired(context);
    }
    private static boolean recoveryStart(String request) {
        return request != null && request.equals(rollbackRequest)
            && request.equals(queuedRequest) && "pending".equals(rollbackStatus);
    }
    private static boolean liveRecovery(String request) {
        return request != null && request.equals(rollbackRequest) && Boolean.TRUE.equals(rollbackLive);
    }
    private static boolean serviceWorkPending() {
        TunnelService active = instance;
        return !pendingStarts.isEmpty() || pendingVpnWork != 0 || pendingCleanup != 0 || (active != null
            && (active.latestStartId == 0 || active.idleStopQueued || active.stopping));
    }
    private static boolean transitionPending() {
        return queuedRequest != null || rollbackConsentReady || serviceWorkPending();
    }
    private static void expireRecoveryStart(Context context, String request) {
        if (recoveryStart(request) && !rollbackWorkClaimed && SystemClock.elapsedRealtime() >= rollbackDeadline) {
            cancelRecoveryForTimeout(context, request);
        }
    }
    private static void requireRecoveryBeforeDeadline(Context context, long deadline) throws IOException {
        requireRecoveryBeforeDeadline(context, deadline, SystemClock.elapsedRealtime());
    }
    private static void requireRecoveryBeforeDeadline(Context context, long deadline, long now) throws IOException {
        if (now >= deadline) {
            throw new IOException(AppLanguage.current(context).getString(R.string.vpn_recovery_timeout));
        }
    }
    private static void cancelRecoveryForTimeout(Context context, String request) {
        reportRollback(request, false, AppLanguage.current(context).getString(R.string.vpn_recovery_timeout));
        finishRequest(request);
    }
    static JSONObject expireVpnStart(Context context, String request) throws org.json.JSONException {
        synchronized (requests) {
            String status;
            if (request.equals(rollbackRequest)) {
                boolean cancelled = recoveryStart(request) && !rollbackWorkClaimed;
                if (cancelled) cancelRecoveryForTimeout(context, request);
                if (serviceWorkPending() || request.equals(queuedRequest)) {
                    status = "claimed";
                } else {
                    status = cancelled ? "cancelled" : "settled";
                }
            } else {
                status = recoveryReservation != null || transitionPending() ? "stale" : "idle";
            }
            return new JSONObject().put("request", request).put("status", status);
        }
    }
    static boolean startAllowed(Context context, String request) {
        synchronized (requests) {
            if (!current(context, request)) return false;
            expireRecoveryStart(context, request);
            return request.equals(rollbackRequest) ? recoveryStart(request) : !recoveryBlocked(context);
        }
    }
    private static String newRequest(Context context) {
        return newRequest(context, java.util.UUID.randomUUID().toString());
    }
    private static String newRequest(Context context, String request) {
        if (rollbackRequest != null && !"failed".equals(rollbackStatus)) {
            rollbackStatus = "failed";
            rollbackError = AppLanguage.current(context).getString(R.string.vpn_cancelled);
        }
        rollbackConsentReady = false;
        context.getSharedPreferences("vpn", MODE_PRIVATE).edit().putString(REQUEST, request).apply();
        queuedRequest = request;
        return request;
    }
    static void reserveRecovery(Context context, String token, boolean live, Boolean desired, long observedGeneration) throws Exception {
        synchronized (requests) {
            if (recoveryBlocked(context) || transitionPending()
                || connected != live || generation != observedGeneration
                || !java.util.Objects.equals(desiredPlan(context), desired)) {
                throw new java.io.IOException(AppLanguage.current(context).getString(R.string.vpn_connecting));
            }
            recoveryReservation = token;
        }
    }
    static void releaseRecovery(String token) {
        synchronized (requests) {
            if (token.equals(recoveryReservation)) recoveryReservation = null;
        }
    }
    static void holdRecovery(Context context) throws Exception {
        synchronized (requests) {
            if (!recoveryRequired(context) || transitionPending()) {
                throw new IOException(AppLanguage.current(context).getString(R.string.protection_unconfirmed));
            }
            TunnelService active = instance;
            if (connected && (active == null || active.descriptor == null || active.stopping)) {
                throw new IOException(AppLanguage.current(context).getString(R.string.protection_unconfirmed));
            }
            if (active != null && active.descriptor != null && !active.forwardingHeld) {
                Mobile.stopTun();
                active.forwardingHeld = true;
            }
        }
    }
    private static Boolean desiredPlan(Context context) {
        SharedPreferences preferences = context.getSharedPreferences("vpn", MODE_PRIVATE);
        return preferences.contains("desired") ? preferences.getBoolean("desired", false) : null;
    }
    static boolean current(Context context, String request) {
        return request != null && request.equals(context.getSharedPreferences("vpn", MODE_PRIVATE).getString(REQUEST, ""));
    }
    private static boolean savePlan(Context context, Boolean desired) {
        SharedPreferences preferences = context.getSharedPreferences("vpn", MODE_PRIVATE);
        Boolean previous = desiredPlan(context);
        if (editPlan(preferences, desired).commit()) return true;
        editPlan(preferences, previous).apply();
        return false;
    }
    private static SharedPreferences.Editor editPlan(SharedPreferences preferences, Boolean desired) {
        SharedPreferences.Editor editor = preferences.edit();
        return desired == null ? editor.remove("desired") : editor.putBoolean("desired", desired);
    }
    static void start(Context context, boolean reconfigure) { start(context, reconfigure, prepareStart(context)); }
    static boolean start(Context context, boolean reconfigure, String request) {
        synchronized (requests) {
            boolean recovering = liveRecovery(request);
            if (recovering && rollbackStartSent) return false;
            if (!startAllowed(context, request)) {
                finishRequest(request);
                return false;
            }
            Intent intent = new Intent(context, TunnelService.class).putExtra(REQUEST, request);
            if (reconfigure) intent.setAction(RECONFIGURE);
            if (recovering) rollbackStartSent = true;
            pendingStarts.merge(request, 1, Integer::sum);
            try {
                context.startForegroundService(intent);
                clearError();
            } catch (RuntimeException error) {
                finishStartDelivery(request);
                reportRollback(request, false, error.toString());
                finishRequest(request);
                throw error;
            }
            return true;
        }
    }
    private static void finishStartDelivery(String request) {
        pendingStarts.computeIfPresent(request, (id, count) -> count == 1 ? null : count - 1);
    }
    static void resumeRequest(Context context, String request) {
        synchronized (requests) {
            if (!recoveryBlocked(context) && current(context, request)) queuedRequest = request;
        }
    }
    static boolean cancelStart(Context context, String request) {
        synchronized (requests) {
            if (recoveryStart(request) && rollbackWorkClaimed) return false;
            if (!startAllowed(context, request)) return false;
            reportRollback(request, false, AppLanguage.current(context).getString(R.string.vpn_cancelled));
            finishRequest(request);
            return true;
        }
    }
    private static void finishRequest(String request) {
        synchronized (requests) {
            if (request != null && request.equals(queuedRequest)) {
                queuedRequest = null;
                if (request.equals(rollbackRequest)) rollbackConsentReady = false;
                scheduleIdleStop();
            }
        }
    }
    private static void scheduleIdleStop() {
        TunnelService active = instance;
        if (!transitionPending() && active != null) active.stopWhenIdle();
    }
    private static void enqueueVpnWork(Runnable work) {
        pendingVpnWork++;
        serial.execute(() -> {
            try {
                work.run();
            } finally {
                synchronized (requests) {
                    pendingVpnWork--;
                    scheduleIdleStop();
                }
            }
        });
    }
    private static void reportRollback(String request, boolean success, String error) {
        synchronized (requests) {
            if (request != null && request.equals(rollbackRequest) && "pending".equals(rollbackStatus)) {
                rollbackStatus = success ? "complete" : "failed";
                rollbackError = error;
                if (success) rollbackGeneration = generation;
            }
        }
    }
    static JSONObject restoreNetwork(Context context, String request, Boolean live, Boolean desired,
            long deadline) throws Exception {
        synchronized (requests) {
            expireRecoveryStart(context, rollbackRequest);
            if (request.equals(rollbackRequest)) {
                return new JSONObject().put("request", request).put("generation", rollbackPreviousGeneration);
            }
            if (recoveryReservation != null || transitionPending()) {
                throw new IOException(AppLanguage.current(context).getString(R.string.vpn_connecting));
            }
            if (!recoveryRequired(context)) {
                throw new IOException(AppLanguage.current(context).getString(R.string.protection_unconfirmed));
            }
            requireRecoveryBeforeDeadline(context, deadline);
            newRequest(context, request);
            rollbackRequest = request;
            rollbackStatus = "pending";
            rollbackError = "";
            rollbackLive = live;
            rollbackDeadline = deadline;
            rollbackStartSent = false;
            rollbackWorkClaimed = false;
            rollbackPreviousGeneration = generation;
            Context application = context.getApplicationContext();
            enqueueVpnWork(() -> applyRestoreNetwork(application, request, live, desired));
            return new JSONObject().put("request", request).put("generation", rollbackPreviousGeneration);
        }
    }
    static String takeRecoveryConsent(Context context) {
        synchronized (requests) {
            if (!rollbackConsentReady || !startAllowed(context, rollbackRequest)) return null;
            rollbackConsentReady = false;
            return rollbackRequest;
        }
    }
    private static void applyRestoreNetwork(Context context, String request, Boolean live, Boolean desired) {
        try {
            synchronized (requests) {
                if (!startAllowed(context, request)) return;
                if (!Boolean.TRUE.equals(live)) rollbackWorkClaimed = true;
                if (!savePlan(context, desired)) {
                    throw new IllegalStateException(AppLanguage.current(context).getString(R.string.vpn_plan_save_failed));
                }
                if (Boolean.TRUE.equals(live)) rollbackConsentReady = true;
            }
            if (live == null) {
                reportRollback(request, true, "");
                finishRequest(request);
                return;
            }
            if (live) {
                MainActivity.dispatchRecoveryConsent();
            } else {
                TunnelService active = instance;
                if (connected && (active == null || !active.forwardingHeld)) {
                    throw new IllegalStateException(AppLanguage.current(context).getString(R.string.vpn_cancelled));
                }
                ProtectionRuntime.pause(context);
                reportRollback(request, true, "");
                finishRequest(request);
            }
        } catch (Exception error) {
            reportRollback(request, false, error.toString());
            finishRequest(request);
        }
    }
    static void writeState(Context context, JSONObject state) throws org.json.JSONException {
        synchronized (requests) {
            expireRecoveryStart(context, rollbackRequest);
            Boolean desired = desiredPlan(context);
            state.put("vpn", connected).put("vpn_pending", pending())
                .put("vpn_held", forwardingHeld())
                .put("vpn_transition_pending", transitionPending())
                .put("vpn_generation", generation).put("vpn_error", lastError)
                .put("vpn_desired", desired == null ? JSONObject.NULL : desired)
                .put("vpn_rollback", rollbackRequest == null ? JSONObject.NULL
                    : new JSONObject().put("request", rollbackRequest).put("status", rollbackStatus)
                        .put("error", rollbackError).put("generation", rollbackGeneration));
        }
    }
    static boolean recoveryCommitted(Context context, String request) {
        synchronized (requests) {
            return request.equals(completedRecovery) && !recoveryRequired(context);
        }
    }
    @FunctionalInterface
    interface RecoveryTunStarter {
        void start(int fd, boolean ipv6) throws Exception;
    }
    static JSONObject finishRecovery(Context context, String request, boolean live, Boolean desired,
        long observedGeneration, String config, boolean ipv6, boolean protectedVpn, long deadline) throws Exception {
        return finishRecovery(context, request, live, desired, observedGeneration, config, ipv6, protectedVpn,
            deadline, SystemClock::elapsedRealtime, Mobile::startTun);
    }
    static JSONObject finishRecovery(Context context, String request, boolean live, Boolean desired,
        long observedGeneration, String config, boolean ipv6, boolean protectedVpn, long deadline,
        LongSupplier elapsedRealtime, RecoveryTunStarter startTun) throws Exception {
        synchronized (requests) {
            TunnelService active = instance;
            boolean hasDescriptor = active != null && active.descriptor != null;
            boolean held = hasDescriptor && active.forwardingHeld && !active.stopping;
            boolean vpnMatches = request.equals(rollbackRequest) && "complete".equals(rollbackStatus)
                && current(context, request)
                && deadline == rollbackDeadline
                && java.util.Objects.equals(rollbackLive, live)
                && recoveryReservation == null && !transitionPending()
                && (live ? connected && held && VpnService.prepare(context) == null
                    : (hasDescriptor ? held : !connected))
                && generation == observedGeneration && generation == rollbackGeneration
                && java.util.Objects.equals(desiredPlan(context), desired);
            boolean matches = CoreBridge.matchesRuntime(config, ipv6);
            boolean guardMatches = ProtectionRuntime.guardMatches(protectedVpn);
            if (vpnMatches && matches && guardMatches) {
                java.nio.file.Path record = recoveryRecord(context).toPath();
                if (!Files.isRegularFile(record, LinkOption.NOFOLLOW_LINKS)) {
                    throw new IOException(AppLanguage.current(context).getString(
                        R.string.protection_recovery_record_invalid));
                }
                java.nio.file.Path committed = record.resolveSibling("protection-recovery-committed.json");
                requireRecoveryBeforeDeadline(context, deadline, elapsedRealtime.getAsLong());
                if (live) {
                    startTun.start(active.descriptor.getFd(), active.interfaceIpv6);
                } else if (active != null && !active.stopping) {
                    try {
                        if (!active.disconnect(request, false, true)) {
                            throw new IOException(AppLanguage.current(context).getString(
                                R.string.protection_recovery_cleanup_incomplete));
                        }
                    } catch (Exception error) {
                        lastError = error.toString();
                        rollbackError = lastError;
                        rollbackStatus = "failed";
                        android.util.Log.e("ClashOfRust", "Disconnect VPN during protection recovery", error);
                        throw error;
                    }
                }
                try {
                    Files.move(record, committed, StandardCopyOption.ATOMIC_MOVE, StandardCopyOption.REPLACE_EXISTING);
                } catch (IOException error) {
                    if (live) {
                        active.forwardingHeld = false;
                        try { Mobile.stopTun(); active.forwardingHeld = true; }
                        catch (Exception stop) { error.addSuppressed(stop); }
                    }
                    throw new IOException(AppLanguage.current(context).getString(
                        R.string.protection_recovery_commit_failed), error);
                }
                completedRecovery = request;
                if (live) active.forwardingHeld = false;
                try { Files.deleteIfExists(committed); }
                catch (IOException error) {
                    android.util.Log.w("ClashOfRust", "Keep committed VPN recovery record", error);
                }
            }
            return new JSONObject().put("matches", matches).put("vpn_matches", vpnMatches)
                .put("guard_matches", guardMatches).put("record_cleared", request.equals(completedRecovery));
        }
    }
    static boolean connected() { return connected; }
    static boolean forwardingHeld() {
        TunnelService active = instance;
        return active != null && active.forwardingHeld;
    }
    static boolean pending() {
        synchronized (requests) {
            TunnelService active = instance;
            return active != null && !connected && !active.stopping
                && queuedRequest != null && queuedRequest.equals(active.acceptedRequest);
        }
    }
    static String error() { return lastError; }
    static void clearError() { lastError = ""; }
    static long generation() { return generation; }
    static void stop(Context context) {
        stop(context, false);
    }
    private static void stop(Context context, boolean revoked) {
        synchronized (requests) {
            if (!revoked && recoveryBlocked(context)) return;
            String request = newRequest(context);
            TunnelService active = instance;
            enqueueVpnWork(() -> {
                if (active != null) {
                    active.disconnect(request, !revoked, false);
                } else {
                    synchronized (requests) {
                        if (!current(context, request)) return;
                        if (!revoked && !recoveryBlocked(context)) {
                            context.getSharedPreferences("vpn", MODE_PRIVATE).edit().putBoolean("desired", false).apply();
                        }
                        finishRequest(request);
                    }
                }
            });
        }
    }
    @Override public void onCreate() {
        super.onCreate();
        synchronized (requests) {
            instance = this;
        }
        updateNotificationChannel(AppLanguage.current(this), true);
        notificationTask = serial.scheduleWithFixedDelay(this::updateStatus, 1, 1, TimeUnit.SECONDS);
    }
    static void refreshNotificationChannel(Context localized) {
        try { updateNotificationChannel(localized, false); }
        catch (RuntimeException error) { android.util.Log.w("ClashOfRust", "Update VPN notification channel name", error); }
    }
    private static synchronized void updateNotificationChannel(Context localized, boolean create) {
        String name = localized.getString(R.string.vpn_channel);
        if (!create && name.equals(notificationChannelName)) return;
        NotificationManager manager = localized.getSystemService(NotificationManager.class);
        NotificationChannel channel = manager.getNotificationChannel(NOTIFICATION_CHANNEL);
        if (channel == null && create) {
            manager.createNotificationChannel(new NotificationChannel(NOTIFICATION_CHANNEL, name, NotificationManager.IMPORTANCE_LOW));
        } else {
            SystemNotifications.renameChannel(manager, channel, name);
        }
        notificationChannelName = name;
    }
    @Override public int onStartCommand(Intent intent, int flags, int startId) {
        synchronized (requests) {
            latestStartId = startId;
            String request = intent == null ? null : intent.getStringExtra(REQUEST);
            finishStartDelivery(request);
            if (request != null && !current(this, request)) {
                // A previously queued foreground start must not undo a later stop.
                // Android's foreground deadline belongs to the service, even
                // when this request is stale and a newer start prevents stopSelf.
                startForeground(1, notification(AppLanguage.current(this).getString(
                    forwardingHeld() ? R.string.protection_unconfirmed : connected ? R.string.vpn_connected : R.string.vpn_cancelled)));
                if (connected || forwardingHeld) return START_STICKY;
                stopWhenIdle();
                return START_NOT_STICKY;
            }
            boolean restore = intent == null;
            boolean alwaysOn = Build.VERSION.SDK_INT >= 29 && isAlwaysOn();
            if (request == null ? recoveryBlocked(this) : !startAllowed(this, request)) {
                finishRequest(request);
                if (connected || forwardingHeld) return START_STICKY;
                startForeground(1, notification(AppLanguage.current(this).getString(R.string.protection_unconfirmed)));
                stopWhenIdle();
                return START_NOT_STICKY;
            }
            if (restore && !alwaysOn && !getSharedPreferences("vpn", MODE_PRIVATE).getBoolean("desired", false)) {
                startForeground(1, notification(AppLanguage.current(this).getString(R.string.vpn_cancelled)));
                stopWhenIdle();
                return START_NOT_STICKY;
            }
            if (request == null) request = prepareStart(this);
            acceptedRequest = request;
            stopping = false;
            clearError();
            try {
                startForeground(1, notification(AppLanguage.current(this).getString(R.string.vpn_connecting)));
            } catch (RuntimeException error) {
                reportRollback(request, false, error.toString());
                finishRequest(request);
                stopWhenIdle();
                return START_NOT_STICKY;
            }
            boolean reconfigure = intent != null && RECONFIGURE.equals(intent.getAction());
            String accepted = request;
            enqueueVpnWork(() -> connect(reconfigure, accepted));
            VpnTileService.refresh(this);
            return START_STICKY;
        }
    }
    private boolean claimRecoveryEstablish(String request) {
        synchronized (requests) {
            if (stopping || !startAllowed(this, request) || !liveRecovery(request)
                    || rollbackWorkClaimed) return false;
            rollbackWorkClaimed = true;
            return true;
        }
    }
    private void stopWhenIdle() {
        if (idleStopQueued || stopping || connected || forwardingHeld) return;
        idleStopQueued = true;
        serial.execute(() -> {
            synchronized (requests) {
                idleStopQueued = false;
                if (instance == this && latestStartId != 0 && !stopping
                        && !transitionPending() && descriptor == null && !connected && !forwardingHeld
                        && stopSelfResult(latestStartId)) {
                    stopping = true;
                    stopForeground(STOP_FOREGROUND_REMOVE);
                }
            }
        });
    }
    private void connect(boolean reconfigure, String request) {
        boolean recovery;
        boolean timedRecovery;
        synchronized (requests) {
            if (stopping || !startAllowed(this, request)) {
                finishRequest(request);
                return;
            }
            recovery = recoveryStart(request);
            timedRecovery = liveRecovery(request);
            if (descriptor != null && !reconfigure) {
                finishRequest(request);
                return;
            }
        }
        ParcelFileDescriptor previous = descriptor;
        ParcelFileDescriptor replacement = null;
        boolean previousReaderStopped = false;
        boolean recoveryProtectorInstalled = false;
        boolean recoveryEstablishClaimed = false;
        boolean established = false;
        boolean protectedVpn = false;
        try {
            CoreBridge.restore(this);
            if (stopping || !startAllowed(this, request)) return;
            if (VpnService.prepare(this) != null) throw new IllegalStateException(AppLanguage.current(this).getString(R.string.vpn_permission_expired));
            boolean ipv6 = CoreBridge.ipv6();
            protectedVpn = CoreBridge.vpnProtectedMode(this);
            Builder builder = new Builder().setSession("Clash of Rust").setMtu(1500)
                .addAddress("198.18.0.1", 30).addRoute("0.0.0.0", 0).addDnsServer("198.18.0.2");
            if (ipv6) builder.addAddress("fdfe:dcba:9876::1", 126).addRoute("::", 0).addDnsServer("fdfe:dcba:9876::2");
            // Do not allow IPv6 to escape the VPN when disabled: omitting its family blocks it.
            ApplicationRouting.apply(this, builder, protectedVpn);
            if (Build.VERSION.SDK_INT >= 29) builder.setMetered(false);
            Intent open = new Intent(this, MainActivity.class);
            builder.setConfigureIntent(PendingIntent.getActivity(this, 0, open, PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT));
            // Establish before closing the old interface. Android deactivates
            // the old VPN atomically when the replacement succeeds; closing it
            // first exposes the underlying network during every reconfigure.
            try (VpnRouting routes=new VpnRouting(this,previous==null ? null : interfaceName,ipv6)) {
                if (timedRecovery) {
                    if (previous != null && !forwardingHeld) {
                        throw new IOException(AppLanguage.current(this).getString(R.string.protection_unconfirmed));
                    }
                    if (previous == null) {
                        if (stopping || !startAllowed(this, request)) return;
                        Mobile.setProtector(fd -> protect(fd));
                        recoveryProtectorInstalled = true;
                    }
                    if (!claimRecoveryEstablish(request)) return;
                    recoveryEstablishClaimed = true;
                } else {
                    if (stopping || !current(this, request)) return;
                    if (previous == null) Mobile.setProtector(fd -> protect(fd));
                    if (previous != null && !forwardingHeld) {
                        Mobile.stopTun();
                        previousReaderStopped = true;
                    }
                    ProtectionRuntime.pause(this);
                }
                replacement = builder.establish();
                if (replacement == null) throw new IllegalStateException(AppLanguage.current(this).getString(R.string.vpn_establish_denied));
                established = true;
                connected = false;
                forwardingHeld = true;
                descriptor = replacement;
                replacement = null;
                interfaceName = null;
                interfaceIpv6 = ipv6;
                closeDescriptor(previous);
                previous = null;
                if (timedRecovery) ProtectionRuntime.pause(this);
                // establish() returns before ConnectivityService/netd finishes
                // moving the existing VPN Network to the new interface. Do
                // not start its packet reader against that transitional device.
                // gVisor stops its reader after a device read failure; publishing
                // routes afterwards does not restart that reader.
                String readyInterface=routes.await(() -> stopping || !current(this, request));
                interfaceName=readyInterface;
                android.util.Log.i("ClashOfRust", "VPN routing ready on "+interfaceName);
            }
            if (stopping || !current(this, request)) { cleanup(); return; }
            ProtectionRuntime.sync(this, true);
            if (!recovery && protectedVpn) awaitGuard(request);
            synchronized (requests) {
                if (stopping || !current(this, request)) { cleanup(); return; }
                if (!recovery) {
                    if (VpnService.prepare(this) != null) {
                        throw new IOException(AppLanguage.current(this).getString(R.string.vpn_permission_expired));
                    }
                    Mobile.startTun(descriptor.getFd(), ipv6);
                    forwardingHeld = false;
                    if (!savePlan(this, true)) {
                        Mobile.stopTun();
                        forwardingHeld = true;
                        throw new IOException(AppLanguage.current(this).getString(R.string.vpn_plan_save_failed));
                    }
                }
                connected = true; lastError = "";
                generation++;
                reportRollback(request, true, "");
            }
            VpnTileService.refresh(this);
            updateStatus();
            MainActivity.emit("vpn", true);
        } catch (Exception error) {
            if (timedRecovery && !recoveryEstablishClaimed) {
                if (startAllowed(this, request)) reportRollback(request, false, error.toString());
                return;
            }
            reportRollback(request, false, error.toString());
            closeDescriptor(replacement);
            if (stopping || !current(this, request)) {
                if (previous != descriptor) closeDescriptor(previous);
                cleanup(); return;
            }
            lastError = error.toString();
            android.util.Log.e("ClashOfRust", "VPN startup failed", error);
            if (established && forwardingHeld && (recovery || protectedVpn)) {
                try {
                    getSystemService(NotificationManager.class).notify(1,
                        notification(AppLanguage.current(this).getString(R.string.protection_unconfirmed)));
                } catch (RuntimeException status) { android.util.Log.w("ClashOfRust", "Update VPN recovery status", status); }
                VpnTileService.refresh(this);
                MainActivity.emit("critical", AppLanguage.current(this).getString(R.string.vpn_start_failed, lastError));
                IncidentNotifications.publish(this, "critical", true);
                return;
            }
            boolean keepPrevious = false;
            if (previous != null && !established) {
                try {
                    keepPrevious = VpnService.prepare(this) == null;
                } catch (Exception restore) {
                    keepPrevious = false;
                    lastError += "; " + AppLanguage.current(this).getString(R.string.vpn_restore_failed, restore);
                    android.util.Log.e("ClashOfRust", "Restore previous VPN reader", restore);
                }
            }
            if (keepPrevious) {
                // A rejected replacement leaves the old descriptor valid.
                // Resume its guard and keep the working connection available.
                connected = true;
                boolean protectionRestored = true;
                try {
                    ProtectionRuntime.sync(this);
                    if (previousReaderStopped && !forwardingHeld) {
                        if (protectedVpn) awaitGuard(request);
                        Mobile.startTun(previous.getFd(), interfaceIpv6);
                    }
                }
                catch (Exception guard) {
                    protectionRestored = false;
                    if (previousReaderStopped) forwardingHeld = true;
                    lastError += "; " + AppLanguage.current(this).getString(R.string.vpn_guard_restore_failed, guard);
                    android.util.Log.e("ClashOfRust", "Restore protection after rejected VPN replacement", guard);
                }
                try {
                    VpnTileService.refresh(this);
                    getSystemService(NotificationManager.class).notify(1, notification(AppLanguage.current(this).getString(R.string.vpn_rebuild_preserved)));
                } catch (RuntimeException status) { android.util.Log.w("ClashOfRust", "Update preserved VPN status", status); }
                MainActivity.emit(protectionRestored ? "error" : "critical", AppLanguage.current(this).getString(R.string.vpn_rebuild_preserved_detail, lastError));
                IncidentNotifications.publish(this, protectionRestored ? "recovery" : "critical", true);
                return;
            }
            if (previous != null) {
                closeDescriptor(previous);
                if (descriptor == previous) descriptor = null;
            }
            MainActivity.emit("critical", AppLanguage.current(this).getString(R.string.vpn_start_failed, lastError));
            IncidentNotifications.publish(this, "critical", true);
            disconnect(request, false, false);
        } finally {
            try {
                if (recoveryProtectorInstalled && !recoveryEstablishClaimed) Mobile.setProtector(null);
            } finally {
                finishRequest(request);
            }
        }
    }
    private void awaitGuard(String request) throws Exception {
        long deadline = android.os.SystemClock.elapsedRealtime() + 5000;
        while (!stopping && current(this, request)) {
            if (ProtectionRuntime.guardMatches(true)) return;
            if (android.os.SystemClock.elapsedRealtime() >= deadline) break;
            Thread.sleep(100);
        }
        throw new IOException(AppLanguage.current(this).getString(R.string.protection_unconfirmed));
    }
    private static boolean closeDescriptor(ParcelFileDescriptor value) {
        if (value == null) return true;
        try {
            value.close();
            return true;
        } catch (Exception error) {
            android.util.Log.e("ClashOfRust", "Close TUN descriptor", error);
            return false;
        }
    }
    private boolean cleanup() {
        return cleanup(false);
    }
    private boolean cleanup(boolean recovering) {
        boolean cleaned = true;
        try { Mobile.stopTun(); } catch (Exception error) {
            cleaned = false;
            android.util.Log.e("ClashOfRust", "Stop TUN", error);
            if (recovering) return false;
        }
        boolean closed = closeDescriptor(descriptor);
        if (recovering && !closed) return false;
        cleaned &= closed;
        descriptor = null;
        forwardingHeld = false;
        interfaceName = null;
        connected = false;
        networkStatus = new JSONObject();
        VpnTileService.refresh(this);
        Mobile.setProtector(null);
        try { ProtectionRuntime.sync(this); }
        catch (Exception error) {
            cleaned = false;
            android.util.Log.e("ClashOfRust", "Stop protection", error);
        }
        MainActivity.emit("vpn", false);
        return cleaned;
    }
    private boolean disconnect(String request, boolean updatePlan, boolean recovering) {
        synchronized (requests) {
            if (!current(this, request)) return false;
            stopping = true;
            if (updatePlan && !recoveryBlocked(this)) {
                getSharedPreferences("vpn", MODE_PRIVATE).edit().putBoolean("desired", false).apply();
            }
        }
        boolean stopped = false;
        try {
            boolean cleaned = cleanup(recovering);
            if (recovering && !cleaned) return false;
            synchronized (requests) {
                if (!current(this, request)) return false;
                stopForeground(STOP_FOREGROUND_REMOVE);
                stopped = stopSelfResult(latestStartId);
                reportRollback(request, cleaned,
                    cleaned ? "" : AppLanguage.current(this).getString(R.string.vpn_cancelled));
            }
            return cleaned;
        } catch (RuntimeException error) {
            reportRollback(request, false, error.toString());
            throw error;
        } finally {
            synchronized (requests) {
                if (!stopped && instance == this) stopping = false;
                finishRequest(request);
            }
        }
    }
    @Override public void onRevoke() {
        stop(this, true);
        lastError = AppLanguage.current(this).getString(R.string.vpn_permission_revoked);
        MainActivity.emit("error", lastError);
        IncidentNotifications.publish(this, "recovery", true);
    }
    @Override public void onDestroy() {
        if (notificationTask != null) notificationTask.cancel(false);
        synchronized (requests) {
            stopping = true;
            if (instance == this) instance = null;
            pendingCleanup++;
        }
        serial.execute(() -> {
            try {
                cleanup();
            } finally {
                synchronized (requests) {
                    pendingCleanup--;
                    reportRollback(acceptedRequest, false, AppLanguage.current(this).getString(R.string.vpn_cancelled));
                    finishRequest(acceptedRequest);
                    scheduleIdleStop();
                }
            }
        });
        super.onDestroy();
    }
    private Notification notification(String text) {
        lastStatus = text;
        String[] lines = text.split("\n", 2);
        String title = lines.length > 1 ? lines[0] : "Clash of Rust";
        String content = lines.length > 1 ? lines[1] : text;
        PendingIntent open = PendingIntent.getActivity(this, 0, new Intent(this, MainActivity.class),
            PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT);
        return new Notification.Builder(this, NOTIFICATION_CHANNEL).setContentTitle(title)
            .setContentText(content).setStyle(new Notification.BigTextStyle().bigText(content))
            .setOnlyAlertOnce(true).setShowWhen(false)
            .setSmallIcon(R.drawable.ic_app).setOngoing(true).setContentIntent(open).build();
    }
    static String statusText(Context context, String down, String up, String node, String mode, boolean speeds, String protection) {
        String label;
        switch (mode) {
            case "global": label = context.getString(R.string.mode_global); break;
            case "rule": label = context.getString(R.string.mode_rule); break;
            case "direct": label = context.getString(R.string.mode_direct); break;
            default: label = context.getString(R.string.mode_unknown); break;
        }
        String text = label + " · " + (node == null || node.isEmpty() ? context.getString(R.string.node_unknown) : node);
        if (speeds) text += "\n↓ " + down + "  ↑ " + up;
        return protection.isEmpty() ? text : protection + "\n" + text;
    }
    private static String speed(long bytes) {
        if (bytes < 1024) return bytes + " B/s";
        double value = bytes;
        String[] units = {"KiB/s", "MiB/s", "GiB/s", "TiB/s"};
        int unit = 0;
        value /= 1024;
        while (value >= 1024 && unit < units.length - 1) { value /= 1024; unit++; }
        return String.format(java.util.Locale.ROOT, "%.1f %s", value, units[unit]);
    }
    private void updateStatus() {
        if (instance != this || !connected || stopping) return;
        try {
            String raw = Mobile.notificationStatus();
            if (!raw.isEmpty()) networkStatus = new JSONObject(raw);
            long rejected = networkStatus.optLong("application_missing_rejected");
            if (rejected > lastApplicationMissingRejected) {
                lastApplicationMissingRejected = rejected;
                IncidentNotifications.publish(this, "application-target-missing", true);
            }
        } catch (Exception error) {
            android.util.Log.w("ClashOfRust", "VPN notification counters unavailable", error);
        }
        try {
            Context localized = AppLanguage.current(this);
            refreshNotificationChannel(localized);
            if (!SystemNotifications.permitted(this)) return;
            String text = statusText(localized, speed(networkStatus.optLong("down")), speed(networkStatus.optLong("up")),
                networkStatus.optString("node"), networkStatus.optString("mode"),
                getSharedPreferences("appearance", MODE_PRIVATE).getBoolean("notification-speeds", true),
                ProtectionRuntime.notificationStatus(localized));
            synchronized (requests) {
                if (instance == this && connected && !stopping && !text.equals(lastStatus))
                    getSystemService(NotificationManager.class).notify(1, notification(text));
            }
        } catch (Exception error) {
            android.util.Log.w("ClashOfRust", "VPN notification status unavailable", error);
        }
    }
}
