package org.clashofrust.android;

import android.app.PendingIntent;
import android.content.ComponentName;
import android.content.Context;
import android.content.Intent;
import android.net.VpnService;
import android.os.Build;
import android.service.quicksettings.Tile;
import android.service.quicksettings.TileService;

/** System Quick Settings entry; shares the same service and consent as the app. */
public final class VpnTileService extends TileService {
    private static java.lang.ref.WeakReference<VpnTileService> listening = new java.lang.ref.WeakReference<>(null);
    static final String CONNECT="org.clashofrust.android.CONNECT";
    static void refresh(Context context) {
        new android.os.Handler(android.os.Looper.getMainLooper()).post(()-> {
            VpnTileService active=listening.get();
            if (active!=null) active.update();
        });
        TileService.requestListeningState(context,new ComponentName(context,VpnTileService.class));
    }
    @Override public void onStartListening() { super.onStartListening(); listening=new java.lang.ref.WeakReference<>(this); update(); }
    @Override public void onStopListening() {
        if (listening.get()==this) listening.clear();
        super.onStopListening();
    }
    // Android 13 and below have no PendingIntent overload. Never call this on 14+.
    @android.annotation.SuppressLint("StartActivityAndCollapseDeprecated")
    @SuppressWarnings("deprecation")
    private void openLegacy(Intent intent) { startActivityAndCollapse(intent); }

    private void update() {
        Tile tile=getQsTile();
        if (tile!=null) {
            tile.setState(TunnelService.connected() || TunnelService.pending()?Tile.STATE_ACTIVE:Tile.STATE_INACTIVE);
            tile.setLabel("Clash of Rust");
            if (Build.VERSION.SDK_INT>=29) tile.setSubtitle(TunnelService.connected()?"VPN 已连接":TunnelService.pending()?"正在连接…":"VPN 未连接");
            tile.updateTile();
        }
    }
    @Override public void onClick() {
        super.onClick();
        if (isLocked()) { unlockAndRun(this::toggle); } else toggle();
    }
    @SuppressWarnings("deprecation")
    private void toggle() {
        if (TunnelService.connected() || TunnelService.pending()) {
            try {
                if (CoreBridge.protectedMode(this)) {
                    SystemNotifications.show(this,"Claude 保护","请先在 A÷检测中关闭 Claude 保护，再断开 VPN。");
                    return;
                }
            } catch (Exception error) {
                android.util.Log.e("ClashOfRust","Cannot safely disconnect protected VPN",error);
                return;
            }
            TunnelService.stop(this); return;
        }
        if (VpnService.prepare(this)==null && (CoreBridge.ready() || CoreBridge.restorable(this))) {
            // TunnelService restores the persisted core itself. A cold process
            // is not a reason to bring the Activity in front of the user's app.
            TunnelService.start(this,false);
        } else {
            Intent open=new Intent(this,MainActivity.class).setAction(CONNECT)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK|Intent.FLAG_ACTIVITY_SINGLE_TOP);
            if (Build.VERSION.SDK_INT>=34) startActivityAndCollapse(PendingIntent.getActivity(this,3,open,
                PendingIntent.FLAG_IMMUTABLE|PendingIntent.FLAG_UPDATE_CURRENT));
            else openLegacy(open);
        }
    }
}
