package org.clashofrust.android;

import android.content.ContentProvider;
import android.content.ContentValues;
import android.database.Cursor;
import android.database.MatrixCursor;
import android.net.Uri;
import android.os.ParcelFileDescriptor;
import android.provider.OpenableColumns;
import java.io.File;
import java.io.FileNotFoundException;

/** Grants the system installer read access to exactly one private, verified APK. */
public final class UpdateProvider extends ContentProvider {
    static File apk(android.content.Context context) { return new File(context.getCacheDir(),"updates/update.apk"); }
    static Uri uri(android.content.Context context) { return Uri.parse("content://"+context.getPackageName()+".updates/update.apk"); }
    private File checked(Uri uri) throws FileNotFoundException {
        if (!uri.equals(uri(getContext()))) throw new FileNotFoundException("Unknown update resource");
        File file=apk(getContext());
        if (!file.isFile()) throw new FileNotFoundException("Update APK is missing");
        return file;
    }
    @Override public boolean onCreate() { return true; }
    @Override public String getType(Uri uri) { return "application/vnd.android.package-archive"; }
    @Override public ParcelFileDescriptor openFile(Uri uri,String mode) throws FileNotFoundException {
        if (!"r".equals(mode)) throw new FileNotFoundException("Read access only");
        return ParcelFileDescriptor.open(checked(uri),ParcelFileDescriptor.MODE_READ_ONLY);
    }
    @Override public Cursor query(Uri uri,String[] projection,String selection,String[] arguments,String sort) {
        try {
            File file=checked(uri);
            String[] columns=projection==null?new String[]{OpenableColumns.DISPLAY_NAME,OpenableColumns.SIZE}:projection;
            MatrixCursor result=new MatrixCursor(columns);Object[] row=new Object[columns.length];
            for (int i=0;i<columns.length;i++) {
                if (OpenableColumns.DISPLAY_NAME.equals(columns[i])) row[i]="Clash-of-Rust-update.apk";
                else if (OpenableColumns.SIZE.equals(columns[i])) row[i]=file.length();
            }
            result.addRow(row);return result;
        } catch (FileNotFoundException missing) { return null; }
    }
    @Override public Uri insert(Uri uri,ContentValues values) { throw new UnsupportedOperationException(); }
    @Override public int update(Uri uri,ContentValues values,String selection,String[] arguments) { throw new UnsupportedOperationException(); }
    @Override public int delete(Uri uri,String selection,String[] arguments) { throw new UnsupportedOperationException(); }
}
