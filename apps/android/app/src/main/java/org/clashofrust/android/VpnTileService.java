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
    static final String CONNECT="org.clashofrust.android.CONNECT";
    static void refresh(Context context) {
        TileService.requestListeningState(context,new ComponentName(context,VpnTileService.class));
    }
    @Override public void onStartListening() { super.onStartListening(); update(); }
    private void update() {
        Tile tile=getQsTile();
        if (tile!=null) {
            tile.setState(TunnelService.connected()?Tile.STATE_ACTIVE:Tile.STATE_INACTIVE);
            tile.setLabel("Clash of Rust");
            if (Build.VERSION.SDK_INT>=29) tile.setSubtitle(TunnelService.connected()?"VPN 已连接":"VPN 未连接");
            tile.updateTile();
        }
    }
    @Override public void onClick() {
        super.onClick();
        if (isLocked()) { unlockAndRun(this::toggle); } else toggle();
    }
    @SuppressWarnings("deprecation")
    private void toggle() {
        if (TunnelService.connected()) { TunnelService.stop(this); return; }
        if (CoreBridge.ready() && VpnService.prepare(this)==null) {
            startForegroundService(new Intent(this,TunnelService.class));
        } else {
            Intent open=new Intent(this,MainActivity.class).setAction(CONNECT)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK|Intent.FLAG_ACTIVITY_SINGLE_TOP);
            if (Build.VERSION.SDK_INT>=34) startActivityAndCollapse(PendingIntent.getActivity(this,3,open,
                PendingIntent.FLAG_IMMUTABLE|PendingIntent.FLAG_UPDATE_CURRENT));
            else startActivityAndCollapse(open);
        }
    }
}
