package org.clashofrust.android;

public final class ClientApplication extends android.app.Application {
    private static volatile ClientApplication instance;
    static ClientApplication instance() { return instance; }

    @Override public void onCreate() {
        super.onCreate();
        instance = this;
        Appearance.persistSystemMode(this, Appearance.mode(this));
    }
}
