package org.clashofrust.android;

import android.app.Activity;
import android.content.Context;
import android.content.ContextWrapper;
import android.content.SharedPreferences;
import android.content.pm.ProviderInfo;
import android.net.Uri;
import android.os.Bundle;
import android.os.ParcelFileDescriptor;
import androidx.test.core.app.ActivityScenario;
import androidx.test.ext.junit.runners.AndroidJUnit4;
import androidx.test.platform.app.InstrumentationRegistry;
import java.io.ByteArrayInputStream;
import java.io.DataInputStream;
import java.io.File;
import java.io.FileNotFoundException;
import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.StandardCopyOption;
import java.util.UUID;
import java.util.concurrent.atomic.AtomicInteger;
import org.junit.Test;
import org.junit.runner.RunWith;
import static org.junit.Assert.*;

@RunWith(AndroidJUnit4.class)
public class UpdateInstallerTest {
    @Test public void updateInstallConsentRestoresOnceAndRejectsReplacedApk() throws Exception {
        Context context = InstrumentationRegistry.getInstrumentation().getTargetContext();
        byte[] confirmed = {1, 2, 3};
        String source = UpdateInstaller.fingerprint(new ByteArrayInputStream(confirmed));
        verifyRecreatedActivity(source);
        verifyRestoredRequest(context, source);
        verifyProvider(context, confirmed, source);
    }

    private static void verifyRecreatedActivity(String source) {
        try (ActivityScenario<MainActivity> scenario = ActivityScenario.launch(MainActivity.class)) {
            AtomicInteger code = new AtomicInteger();
            scenario.onActivity(activity -> {
                try {
                    code.set(activity.installPermission.begin(source, false));
                    assertTrue(UpdateInstaller.PermissionRequest.handles(code.get()));
                    MainActivity.FilePicker picker = new MainActivity.FilePicker();
                    assertTrue(picker.reserve(42));
                    assertFalse(UpdateInstaller.PermissionRequest.handles(picker.launch()));
                } catch (IOException error) {
                    throw new AssertionError(error);
                }
            });
            scenario.recreate();
            scenario.onActivity(activity -> {
                UpdateInstaller.PermissionRequest restored = activity.installPermission;
                try {
                    assertEquals(source, restored.source());
                    activity.onActivityResult(code.get() + 1, Activity.RESULT_OK, null);
                    assertEquals(0, restored.readyCode());
                    activity.onActivityResult(code.get(), Activity.RESULT_CANCELED, null);
                    assertEquals(code.get(), restored.readyCode());
                    assertEquals(source, restored.source());
                    assertTrue(restored.finish(code.get()));
                    activity.onActivityResult(code.get(), Activity.RESULT_OK, null);
                    assertEquals(0, restored.readyCode());
                    assertFalse(restored.finish(code.get()));
                } catch (IOException error) {
                    throw new AssertionError(error);
                }
            });
        }
    }

    private static void verifyRestoredRequest(Context context, String source) throws IOException {
        String name = "update-permission-test-" + UUID.randomUUID();
        SharedPreferences store = context.getSharedPreferences(name, Context.MODE_PRIVATE);
        try {
            UpdateInstaller.PermissionRequest original = new UpdateInstaller.PermissionRequest(store, null);
            int first = original.begin(source, false);
            assertEquals(0, original.begin(source, true));
            Bundle saved = new Bundle();
            original.save(saved);

            UpdateInstaller.PermissionRequest returned = new UpdateInstaller.PermissionRequest(store, saved);
            returned.returned(first);
            assertEquals(0, new UpdateInstaller.PermissionRequest(store, null).readyCode());
            UpdateInstaller.PermissionRequest rechecking = new UpdateInstaller.PermissionRequest(store, saved);
            assertEquals(first, rechecking.readyCode());
            assertEquals(source, rechecking.source());
            assertTrue(rechecking.finish(first));
            assertFalse(returned.finish(first));
            assertNull(new UpdateInstaller.PermissionRequest(store, saved).source());

            UpdateInstaller.PermissionRequest next = new UpdateInstaller.PermissionRequest(store, null);
            int second = next.begin(source, false);
            assertNotEquals(first, second);
            original.returned(first);
            next.returned(first);
            assertEquals(0, next.readyCode());
            assertFalse(original.finish(first));
            assertTrue(next.finish(second));
        } finally {
            context.deleteSharedPreferences(name);
        }
    }

    private static void verifyProvider(Context context, byte[] confirmed, String source) throws IOException {
        File cache = Files.createTempDirectory(context.getCacheDir().toPath(), "update-provider-").toFile();
        Context fixture = new ContextWrapper(context) {
            @Override public File getCacheDir() { return cache; }
        };
        UpdateProvider provider = new UpdateProvider();
        ProviderInfo info = new ProviderInfo();
        info.authority = fixture.getPackageName() + ".updates";
        provider.attachInfo(fixture, info);
        File apk = UpdateProvider.apk(fixture);
        File replacement = new File(apk.getParentFile(), "download.tmp");
        try {
            assertTrue(apk.getParentFile().mkdir());
            Files.write(apk.toPath(), confirmed);
            Uri uri = UpdateProvider.uri(fixture, source);
            assertDenied(provider, uri, "rw", "Read access only");
            assertDenied(provider, uri.buildUpon().appendQueryParameter("extra", "1").build(),
                "r", "Unknown update resource");
            try (var input = new DataInputStream(new ParcelFileDescriptor.AutoCloseInputStream(provider.openFile(uri, "r")))) {
                Files.write(replacement.toPath(), new byte[]{1, 2, 4});
                Files.move(replacement.toPath(), apk.toPath(), StandardCopyOption.ATOMIC_MOVE,
                    StandardCopyOption.REPLACE_EXISTING);
                assertDenied(provider, uri, "r", "APK SHA-256 mismatch");
                byte[] handedOff = new byte[confirmed.length];
                input.readFully(handedOff);
                assertArrayEquals(confirmed, handedOff);
                assertEquals(-1, input.read());
            }
            assertTrue(apk.delete());
            assertDenied(provider, uri, "r", null);
        } finally {
            Files.deleteIfExists(replacement.toPath());
            Files.deleteIfExists(apk.toPath());
            Files.deleteIfExists(apk.getParentFile().toPath());
            Files.deleteIfExists(cache.toPath());
        }
    }

    private static void assertDenied(UpdateProvider provider, Uri uri, String mode, String message) throws IOException {
        try (ParcelFileDescriptor ignored = provider.openFile(uri, mode)) {
            fail("Provider opened an unauthorized APK");
        } catch (FileNotFoundException expected) {
            if (message != null) assertEquals(message, expected.getMessage());
        }
    }
}
