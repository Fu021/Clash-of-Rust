package org.clashofrust.android;

import android.content.ContentProvider;
import android.content.ContentValues;
import android.content.Context;
import android.database.Cursor;
import android.database.MatrixCursor;
import android.net.Uri;
import android.os.ParcelFileDescriptor;
import android.provider.OpenableColumns;
import android.system.ErrnoException;
import android.system.Os;
import android.system.OsConstants;
import java.io.File;
import java.io.FileNotFoundException;
import java.io.IOException;
import java.security.NoSuchAlgorithmException;
import java.util.List;
import java.util.regex.Pattern;

/** Grants the system installer read access to exactly one private, verified APK. */
public final class UpdateProvider extends ContentProvider {
    private static final Pattern SHA256 = Pattern.compile("[A-Za-z0-9_-]{43}");

    static File apk(Context context) { return new File(context.getCacheDir(), "updates/update.apk"); }

    static Uri uri(Context context, String source) {
        return new Uri.Builder().scheme("content").authority(context.getPackageName() + ".updates")
            .appendPath(source).appendPath("update.apk").build();
    }

    private String checkedSource(Uri uri) throws FileNotFoundException {
        List<String> segments = uri.getPathSegments();
        if (segments.size() == 2) {
            String source = segments.get(0);
            if (SHA256.matcher(source).matches() && uri.equals(uri(getContext(), source))) return source;
        }
        throw new FileNotFoundException("Unknown update resource");
    }

    @Override public boolean onCreate() { return true; }
    @Override public String getType(Uri uri) { return "application/vnd.android.package-archive"; }
    @Override public ParcelFileDescriptor openFile(Uri uri,String mode) throws FileNotFoundException {
        if (!"r".equals(mode)) throw new FileNotFoundException("Read access only");
        String source = checkedSource(uri);
        try (ParcelFileDescriptor opened = ParcelFileDescriptor.open(apk(getContext()), ParcelFileDescriptor.MODE_READ_ONLY)) {
            try (var input = new ParcelFileDescriptor.AutoCloseInputStream(opened.dup())) {
                if (!source.equals(UpdateInstaller.fingerprint(input))) throw new FileNotFoundException("APK SHA-256 mismatch");
            }
            Os.lseek(opened.getFileDescriptor(), 0, OsConstants.SEEK_SET);
            return opened.dup();
        } catch (FileNotFoundException error) {
            throw error;
        } catch (IOException | ErrnoException | NoSuchAlgorithmException error) {
            FileNotFoundException denied = new FileNotFoundException("Could not verify the update APK");
            denied.initCause(error);
            throw denied;
        }
    }

    @Override public Cursor query(Uri uri,String[] projection,String selection,String[] arguments,String sort) {
        try {
            checkedSource(uri);
            if (!apk(getContext()).isFile()) return null;
            String[] columns=projection==null?new String[]{OpenableColumns.DISPLAY_NAME,OpenableColumns.SIZE}:projection;
            MatrixCursor result=new MatrixCursor(columns);Object[] row=new Object[columns.length];
            for (int i=0;i<columns.length;i++) {
                if (OpenableColumns.DISPLAY_NAME.equals(columns[i])) row[i]="Clash-of-Rust-update.apk";
            }
            result.addRow(row);return result;
        } catch (FileNotFoundException missing) { return null; }
    }
    @Override public Uri insert(Uri uri,ContentValues values) { throw new UnsupportedOperationException(); }
    @Override public int update(Uri uri,ContentValues values,String selection,String[] arguments) { throw new UnsupportedOperationException(); }
    @Override public int delete(Uri uri,String selection,String[] arguments) { throw new UnsupportedOperationException(); }
}
