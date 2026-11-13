package com.atlascode.xr;

import android.app.Activity;
import android.content.Context;
import android.text.Editable;
import android.text.InputType;
import android.text.TextWatcher;
import android.view.KeyEvent;
import android.view.View;
import android.view.inputmethod.EditorInfo;
import android.view.inputmethod.InputMethodManager;
import android.widget.EditText;
import android.widget.FrameLayout;
import android.widget.TextView;

/**
 * Gives the app a real text field so that Meta's system keyboard (the overlay) shows
 * up: without a text field the keyboard refuses to come up (inputType 0). The field is tiny and
 * almost transparent; what is typed piles up in a queue that Rust empties with {@link #poll()}.
 *
 * Queue encoding: the characters as they are, '\b' to delete and '\n' for Enter.
 */
public final class AtlasInput {
    private static EditText field;
    private static final StringBuilder pending = new StringBuilder();

    private AtlasInput() {}

    private static void push(String s) {
        synchronized (pending) {
            pending.append(s);
        }
    }

    /** Shows the system keyboard, creating the text field the first time. */
    public static void show(final Activity activity) {
        activity.runOnUiThread(new Runnable() {
            @Override
            public void run() {
                if (field == null) {
                    field = new EditText(activity);
                    field.setInputType(InputType.TYPE_CLASS_TEXT
                            | InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS
                            | InputType.TYPE_TEXT_VARIATION_VISIBLE_PASSWORD);
                    field.setImeOptions(EditorInfo.IME_ACTION_DONE | EditorInfo.IME_FLAG_NO_EXTRACT_UI);
                    field.setAlpha(0.02f);
                    field.addTextChangedListener(new TextWatcher() {
                        @Override
                        public void beforeTextChanged(CharSequence s, int start, int count, int after) {}

                        @Override
                        public void onTextChanged(CharSequence s, int start, int before, int count) {}

                        @Override
                        public void afterTextChanged(Editable e) {
                            // The field stays empty: what is typed goes to the queue and is cleared.
                            if (e.length() > 0) {
                                push(e.toString());
                                e.clear();
                            }
                        }
                    });
                    field.setOnKeyListener(new View.OnKeyListener() {
                        @Override
                        public boolean onKey(View v, int keyCode, KeyEvent event) {
                            if (event.getAction() != KeyEvent.ACTION_DOWN) {
                                return false;
                            }
                            if (keyCode == KeyEvent.KEYCODE_DEL) {
                                push("\b");
                                return true;
                            }
                            if (keyCode == KeyEvent.KEYCODE_ENTER) {
                                push("\n");
                                return true;
                            }
                            return false;
                        }
                    });
                    field.setOnEditorActionListener(new TextView.OnEditorActionListener() {
                        @Override
                        public boolean onEditorAction(TextView v, int actionId, KeyEvent event) {
                            push("\n");
                            return true;
                        }
                    });
                    activity.addContentView(field, new FrameLayout.LayoutParams(2, 2));
                }
                field.requestFocus();
                InputMethodManager imm = (InputMethodManager) activity.getSystemService(Context.INPUT_METHOD_SERVICE);
                imm.showSoftInput(field, 0);
            }
        });
    }

    /** Hides the system keyboard. */
    public static void hide(final Activity activity) {
        activity.runOnUiThread(new Runnable() {
            @Override
            public void run() {
                if (field == null) {
                    return;
                }
                InputMethodManager imm = (InputMethodManager) activity.getSystemService(Context.INPUT_METHOD_SERVICE);
                imm.hideSoftInputFromWindow(field.getWindowToken(), 0);
                field.clearFocus();
            }
        });
    }

    /** Returns and clears what was typed since the last time. */
    public static String poll() {
        synchronized (pending) {
            String s = pending.toString();
            pending.setLength(0);
            return s;
        }
    }
}
