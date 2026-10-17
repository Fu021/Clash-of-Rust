package org.clashofrust.android;

public final class ClientApplication extends android.app.Application {
    @Override public void onCreate() {
        super.onCreate();
        Appearance.persistSystemMode(this, Appearance.mode(this));
    }
}
