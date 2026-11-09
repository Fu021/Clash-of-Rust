package org.clashofrust.android;

import android.app.Activity;
import android.content.pm.ApplicationInfo;
import android.os.Bundle;
import android.text.Editable;
import android.text.TextWatcher;
import android.view.View;
import android.view.ViewGroup;
import android.widget.*;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.HashSet;
import java.util.List;
import java.util.Locale;
import java.util.Set;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;

public final class ApplicationRoutingActivity extends Activity {
    private static final String[] MODES = {ApplicationRouting.ALL, ApplicationRouting.ONLY, ApplicationRouting.EXCLUDE};
    private final ExecutorService worker = Executors.newSingleThreadExecutor();
    private final List<Entry> installed = new ArrayList<>();
    private final List<Entry> visible = new ArrayList<>();
    private final Set<String> selected = new HashSet<>();
    private final AppsAdapter adapter = new AppsAdapter();
    private Spinner modes;
    private EditText search;
    private CheckBox systemApps;
    private TextView status;
    private Button saveButton;
    private boolean loaded;

    private static final class Entry {
        final ApplicationInfo application;
        final String label;
        Entry(ApplicationInfo application, String label) {
            this.application = application;
            this.label = label;
        }
    }

    @Override public void onCreate(Bundle state) {
        setTheme(Appearance.dark(this, Appearance.mode(this)) ? R.style.ClientDark : R.style.ClientLight);
        super.onCreate(state);
        if (!editable()) return;
        ApplicationRouting saved;
        try { saved = ApplicationRouting.load(this); }
        catch (Exception error) { failed(error); return; }
        selected.addAll(saved.packages);
        if (state != null) {
            selected.clear();
            ArrayList<String> restored = state.getStringArrayList("selected");
            if (restored != null) selected.addAll(restored);
        }
        LinearLayout layout = column();
        layout.setPadding(dp(16), dp(12), dp(16), dp(8));
        LinearLayout heading = new LinearLayout(this);
        heading.setGravity(android.view.Gravity.CENTER_VERTICAL);
        Button back = button("返回", this::finish);
        heading.addView(back);
        TextView title = new TextView(this);
        title.setText("应用分流");
        title.setTextSize(20);
        heading.addView(title, new LinearLayout.LayoutParams(0, -2, 1));
        saveButton = button("保存", this::save);
        saveButton.setEnabled(false);
        heading.addView(saveButton);
        layout.addView(heading);
        TextView explanation = new TextView(this);
        explanation.setText(R.string.application_routing_explanation);
        layout.addView(explanation);
        modes = new Spinner(this);
        ArrayAdapter<String> choices = new ArrayAdapter<>(this, android.R.layout.simple_spinner_item,
            new String[]{"全部应用", "仅代理选中应用", "排除选中应用"});
        choices.setDropDownViewResource(android.R.layout.simple_spinner_dropdown_item);
        modes.setAdapter(choices);
        int activeMode = java.util.Arrays.asList(MODES).indexOf(saved.mode);
        modes.setSelection(state == null ? activeMode : state.getInt("mode", activeMode));
        layout.addView(modes, new LinearLayout.LayoutParams(-1, dp(48)));
        search = new EditText(this);
        search.setSingleLine(true);
        search.setHint("搜索应用名称或包名");
        search.setInputType(android.text.InputType.TYPE_CLASS_TEXT);
        layout.addView(search, new LinearLayout.LayoutParams(-1, dp(48)));
        systemApps = new CheckBox(this);
        systemApps.setText("显示系统应用");
        systemApps.setChecked(state != null && state.getBoolean("system"));
        layout.addView(systemApps);
        LinearLayout actions = new LinearLayout(this);
        actions.addView(button("全选结果", () -> selectVisible(1)), new LinearLayout.LayoutParams(0, dp(48), 1));
        actions.addView(button("反选结果", () -> selectVisible(2)), new LinearLayout.LayoutParams(0, dp(48), 1));
        actions.addView(button("清空全部", () -> { selected.clear(); refresh(); }), new LinearLayout.LayoutParams(0, dp(48), 1));
        layout.addView(actions);
        status = new TextView(this);
        status.setText("正在读取已安装应用…");
        layout.addView(status);
        ListView list = new ListView(this);
        list.setAdapter(adapter);
        list.setOnItemClickListener((parent, view, position, id) -> {
            String name = visible.get(position).application.packageName;
            if (!selected.remove(name)) selected.add(name);
            refresh();
        });
        layout.addView(list, new LinearLayout.LayoutParams(-1, 0, 1));
        layout.setOnApplyWindowInsetsListener((view, insets) -> {
            applyInsets(layout, insets);
            return insets;
        });
        setContentView(layout);
        layout.requestApplyInsets();
        systemApps.setOnCheckedChangeListener((button, checked) -> filter());
        search.addTextChangedListener(new TextWatcher() {
            public void beforeTextChanged(CharSequence text, int start, int count, int after) { }
            public void onTextChanged(CharSequence text, int start, int before, int count) { filter(); }
            public void afterTextChanged(Editable text) { }
        });
        if (state != null) search.setText(state.getString("query", ""));
        worker.execute(() -> {
            try {
                List<Entry> entries = new ArrayList<>();
                for (ApplicationInfo application : getPackageManager().getInstalledApplications(0))
                    entries.add(new Entry(application, application.loadLabel(getPackageManager()).toString()));
                entries.sort(Comparator.comparing((Entry entry) -> entry.label, String.CASE_INSENSITIVE_ORDER)
                    .thenComparing(entry -> entry.application.packageName));
                runOnUiThread(() -> {
                    if (isDestroyed()) return;
                    installed.addAll(entries);
                    Set<String> available = new HashSet<>();
                    for (Entry entry : entries) available.add(entry.application.packageName);
                    selected.retainAll(available);
                    loaded = true;
                    saveButton.setEnabled(true);
                    filter();
                });
            } catch (Exception error) { runOnUiThread(() -> { if (!isDestroyed()) failed(error); }); }
        });
    }

    @Override protected void onResume() {
        super.onResume();
        editable();
    }

    private boolean editable() {
        try {
            if (!CoreBridge.protectedMode(this)) return true;
            Toast.makeText(this, "请先关闭 Claude 保护，再修改应用分流", Toast.LENGTH_LONG).show();
        } catch (Exception error) { failed(error); return false; }
        finish();
        return false;
    }

    private void save() {
        if (!loaded || !editable()) return;
        try {
            new ApplicationRouting(MODES[modes.getSelectedItemPosition()], selected).save(this);
            MainActivity.emit("notice", TunnelService.connected()
                ? "应用分流已保存，重新连接 VPN 后生效" : "应用分流已保存，下次连接时生效");
            finish();
        } catch (IllegalArgumentException error) { status.setText(error.getMessage()); }
        catch (Exception error) { failed(error); }
    }

    private void failed(Exception error) {
        MainActivity.emit("error", "应用分流失败：" + error);
        Toast.makeText(this, "应用分流失败，可返回日志页导出诊断", Toast.LENGTH_LONG).show();
        finish();
    }

    private void filter() {
        if (!loaded) return;
        String query = search.getText().toString().trim().toLowerCase(Locale.ROOT);
        visible.clear();
        for (Entry entry : installed) {
            if (!systemApps.isChecked() && (entry.application.flags & ApplicationInfo.FLAG_SYSTEM) != 0) continue;
            if (query.isEmpty() || entry.label.toLowerCase(Locale.ROOT).contains(query)
                    || entry.application.packageName.toLowerCase(Locale.ROOT).contains(query)) visible.add(entry);
        }
        refresh();
    }

    private void selectVisible(int action) {
        for (Entry entry : visible) {
            String name = entry.application.packageName;
            if (action == 1 || !selected.remove(name)) selected.add(name);
        }
        refresh();
    }

    private void refresh() {
        if (!loaded) return;
        status.setText(getString(R.string.application_routing_selection_count, selected.size(), visible.size()));
        adapter.notifyDataSetChanged();
    }

    @SuppressWarnings("deprecation")
    private void applyInsets(LinearLayout layout, android.view.WindowInsets insets) {
        int left, top, right, bottom;
        if (android.os.Build.VERSION.SDK_INT >= 30) {
            android.graphics.Insets bounds = insets.getInsets(android.view.WindowInsets.Type.systemBars()
                | android.view.WindowInsets.Type.displayCutout() | android.view.WindowInsets.Type.ime());
            left = bounds.left; top = bounds.top; right = bounds.right; bottom = bounds.bottom;
        } else {
            left = insets.getSystemWindowInsetLeft(); top = insets.getSystemWindowInsetTop();
            right = insets.getSystemWindowInsetRight(); bottom = insets.getSystemWindowInsetBottom();
        }
        layout.setPadding(dp(16) + left, dp(12) + top, dp(16) + right, dp(8) + bottom);
    }

    @Override protected void onSaveInstanceState(Bundle state) {
        super.onSaveInstanceState(state);
        state.putStringArrayList("selected", new ArrayList<>(selected));
        if (modes == null) return;
        state.putInt("mode", modes.getSelectedItemPosition());
        state.putString("query", search.getText().toString());
        state.putBoolean("system", systemApps.isChecked());
    }

    @Override protected void onDestroy() {
        worker.shutdownNow();
        super.onDestroy();
    }

    private int dp(int value) { return Math.round(value * getResources().getDisplayMetrics().density); }
    private LinearLayout column() {
        LinearLayout layout = new LinearLayout(this);
        layout.setOrientation(LinearLayout.VERTICAL);
        return layout;
    }
    private Button button(String text, Runnable action) {
        Button button = new Button(this);
        button.setText(text);
        button.setTextSize(13);
        button.setMinimumWidth(dp(48));
        button.setMinWidth(dp(48));
        button.setOnClickListener(view -> action.run());
        return button;
    }

    private final class AppsAdapter extends BaseAdapter {
        public int getCount() { return visible.size(); }
        public Entry getItem(int position) { return visible.get(position); }
        public long getItemId(int position) { return position; }
        public View getView(int position, View recycled, ViewGroup parent) {
            LinearLayout row;
            if (recycled instanceof LinearLayout) row = (LinearLayout) recycled;
            else {
                row = new LinearLayout(ApplicationRoutingActivity.this);
                row.setPadding(dp(4), dp(6), dp(4), dp(6));
                row.setGravity(android.view.Gravity.CENTER_VERTICAL);
                ImageView icon = new ImageView(ApplicationRoutingActivity.this);
                icon.setImportantForAccessibility(View.IMPORTANT_FOR_ACCESSIBILITY_NO);
                row.addView(icon, new LinearLayout.LayoutParams(dp(36), dp(36)));
                LinearLayout labels = column();
                labels.setPadding(dp(12), 0, dp(8), 0);
                TextView name = new TextView(ApplicationRoutingActivity.this);
                name.setTextSize(15);
                name.setSingleLine(true);
                name.setEllipsize(android.text.TextUtils.TruncateAt.END);
                labels.addView(name);
                TextView identifier = new TextView(ApplicationRoutingActivity.this);
                identifier.setTextSize(11);
                identifier.setSingleLine(true);
                identifier.setEllipsize(android.text.TextUtils.TruncateAt.END);
                labels.addView(identifier);
                row.addView(labels, new LinearLayout.LayoutParams(0, -2, 1));
                CheckBox checked = new CheckBox(ApplicationRoutingActivity.this);
                checked.setClickable(false);
                checked.setFocusable(false);
                checked.setImportantForAccessibility(View.IMPORTANT_FOR_ACCESSIBILITY_NO);
                row.addView(checked);
                row.setMinimumHeight(dp(56));
            }
            Entry entry = getItem(position);
            ((ImageView) row.getChildAt(0)).setImageDrawable(entry.application.loadIcon(getPackageManager()));
            LinearLayout labels = (LinearLayout) row.getChildAt(1);
            ((TextView) labels.getChildAt(0)).setText(entry.label);
            ((TextView) labels.getChildAt(1)).setText(entry.application.packageName);
            boolean checked = selected.contains(entry.application.packageName);
            ((CheckBox) row.getChildAt(2)).setChecked(checked);
            row.setContentDescription(entry.label + "，" + entry.application.packageName + (checked ? "，已选择" : "，未选择"));
            return row;
        }
    }
}
