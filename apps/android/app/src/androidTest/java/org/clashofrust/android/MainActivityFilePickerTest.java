package org.clashofrust.android;

import androidx.test.ext.junit.runners.AndroidJUnit4;
import org.junit.Test;
import org.junit.runner.RunWith;
import static org.junit.Assert.*;

@RunWith(AndroidJUnit4.class)
public class MainActivityFilePickerTest {
    @Test public void occupiedPickerKeepsTheOriginalRequestAndIgnoresDuplicateResults() {
        MainActivity.FilePicker picker = new MainActivity.FilePicker();
        assertTrue(picker.reserve(42));
        int first = picker.launch();
        assertTrue(first > 0);
        assertFalse(picker.reserve(43));
        assertTrue(picker.reserve(42));
        assertEquals(0, picker.launch());
        assertEquals(42, picker.complete(first));

        assertTrue(picker.reserve(43));
        int second = picker.launch();
        assertNotEquals(first, second);
        assertEquals(0, picker.complete(first));
        assertEquals(43, picker.complete(second));
        assertEquals(0, picker.complete(second));
    }

    @Test public void activityRecreationKeepsAnUnlaunchedOrDisplayedPicker() {
        MainActivity.FilePicker picker = new MainActivity.FilePicker();
        assertTrue(picker.reserve(42));
        picker.restoreCode(picker.lastCode());
        int code = picker.launch();
        assertTrue(code > 0);
        picker.restoreCode(code);
        assertEquals(0, picker.launch());
        assertEquals(42, picker.complete(code));
    }

    @Test public void resultFromADeadProcessCannotConsumeANewRequest() {
        MainActivity.FilePicker original = new MainActivity.FilePicker();
        assertTrue(original.reserve(42));
        int previousCode = original.launch();

        MainActivity.FilePicker replacement = new MainActivity.FilePicker();
        replacement.restoreCode(previousCode);
        assertEquals(0, replacement.complete(previousCode));
        assertTrue(replacement.reserve(42));
        int currentCode = replacement.launch();
        assertNotEquals(previousCode, currentCode);
        assertEquals(0, replacement.complete(previousCode));
        assertEquals(42, replacement.complete(currentCode));
    }
}
