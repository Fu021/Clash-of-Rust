package org.clashofrust.android;

import android.app.Activity;
import android.os.Bundle;
import android.util.Log;
import android.view.WindowInsets;
import android.view.inputmethod.InputMethodManager;
import android.widget.EditText;
import org.json.JSONObject;
import java.io.File;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;

@android.annotation.TargetApi(31)
public final class PlatformImeControlActivity extends Activity {
    private EditText editor;
    private int showRequests;
    private boolean started;

    @Override public void onCreate(Bundle state) {
        super.onCreate(state);
        editor=new EditText(this);
        editor.setSingleLine(true);
        editor.setHint("Native Android input-method control");
        editor.setText("PlatformImeProbe");
        setContentView(editor);
        editor.setOnApplyWindowInsetsListener((view,insets)-> {
            editor.post(this::record);
            return view.onApplyWindowInsets(insets);
        });
        editor.requestFocus();
    }

    @Override public void onWindowFocusChanged(boolean focused) {
        super.onWindowFocusChanged(focused);
        if (!focused || started || editor==null) return;
        started=true;
        editor.postOnAnimation(()-> {
            show();
            if (getIntent().getIntExtra("show_requests",1)==2) editor.postDelayed(this::show,500);
        });
    }

    private void show() {
        showRequests++;
        getSystemService(InputMethodManager.class).showSoftInput(editor,InputMethodManager.SHOW_IMPLICIT);
        record();
    }

    private void record() {
        try {
            WindowInsets insets=editor.getRootWindowInsets();
            boolean visible=android.os.Build.VERSION.SDK_INT>=30 && insets!=null
                && insets.isVisible(WindowInsets.Type.ime());
            JSONObject state=new JSONObject()
                .put("schema",1).put("api",android.os.Build.VERSION.SDK_INT)
                .put("editor","android.widget.EditText")
                .put("window_focus",hasWindowFocus()).put("editor_focus",editor.hasFocus())
                .put("show_requests",showRequests).put("ime_visible",visible)
                .put("ime_bottom",visible?insets.getInsets(WindowInsets.Type.ime()).bottom:0)
                .put("text_length",editor.length());
            File directory=new File(getExternalFilesDir(null),"platform-ime-control");
            if (!directory.isDirectory() && !directory.mkdirs()) throw new java.io.IOException("Control directory");
            Files.write(new File(directory,"state.json").toPath(),state.toString(2).getBytes(StandardCharsets.UTF_8));
            Log.i("CorImeControl",state.toString());
        } catch (Exception error) {
            Log.e("CorImeControl","Could not save native editor evidence",error);
        }
    }
}
