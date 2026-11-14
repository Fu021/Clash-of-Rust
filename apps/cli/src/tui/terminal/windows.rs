use crate::tui::input::Utf16Decoder;
use anyhow::{Result, ensure};
use windows_sys::Win32::{
    Foundation::HANDLE,
    System::Console::{
        CONSOLE_SCREEN_BUFFER_INFO, ENABLE_ECHO_INPUT, ENABLE_EXTENDED_FLAGS, ENABLE_LINE_INPUT,
        ENABLE_PROCESSED_INPUT, ENABLE_QUICK_EDIT_MODE, ENABLE_VIRTUAL_TERMINAL_PROCESSING,
        GetConsoleMode, GetConsoleScreenBufferInfo, GetNumberOfConsoleInputEvents, GetStdHandle,
        INPUT_RECORD, KEY_EVENT, ReadConsoleInputW, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
        SetConsoleMode,
    },
};

pub struct Mode {
    input: HANDLE,
    output: HANDLE,
    input_mode: u32,
    output_mode: u32,
    characters: Utf16Decoder,
}

impl Mode {
    pub fn enter() -> Result<Self> {
        let input = unsafe { GetStdHandle(STD_INPUT_HANDLE) };
        let output = unsafe { GetStdHandle(STD_OUTPUT_HANDLE) };
        let mut input_mode = 0;
        let mut output_mode = 0;
        ensure!(
            unsafe { GetConsoleMode(input, &mut input_mode) } != 0
                && unsafe { GetConsoleMode(output, &mut output_mode) } != 0,
            "无法读取控制台模式：{}",
            std::io::Error::last_os_error()
        );
        let mode = Self {
            input,
            output,
            input_mode,
            output_mode,
            characters: Utf16Decoder::default(),
        };
        ensure!(
            unsafe {
                SetConsoleMode(
                    input,
                    (input_mode | ENABLE_EXTENDED_FLAGS)
                        & !(ENABLE_ECHO_INPUT
                            | ENABLE_LINE_INPUT
                            | ENABLE_PROCESSED_INPUT
                            | ENABLE_QUICK_EDIT_MODE),
                )
            } != 0,
            "无法设置控制台输入：{}",
            std::io::Error::last_os_error()
        );
        ensure!(
            unsafe { SetConsoleMode(output, output_mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING) }
                != 0,
            "当前控制台不支持 ANSI 输出：{}",
            std::io::Error::last_os_error()
        );
        Ok(mode)
    }

    pub fn size(&self) -> Result<(usize, usize)> {
        let mut info = std::mem::MaybeUninit::<CONSOLE_SCREEN_BUFFER_INFO>::uninit();
        ensure!(
            unsafe { GetConsoleScreenBufferInfo(self.output, info.as_mut_ptr()) } != 0,
            "无法读取控制台尺寸：{}",
            std::io::Error::last_os_error()
        );
        let info = unsafe { info.assume_init() };
        Ok((
            (info.srWindow.Right - info.srWindow.Left + 1) as usize,
            (info.srWindow.Bottom - info.srWindow.Top + 1) as usize,
        ))
    }

    pub fn read(&mut self) -> Result<Vec<u8>> {
        let mut available = 0;
        ensure!(
            unsafe { GetNumberOfConsoleInputEvents(self.input, &mut available) } != 0,
            "控制台输入已断开：{}",
            std::io::Error::last_os_error()
        );
        let mut bytes = Vec::new();
        for _ in 0..available.min(64) {
            let mut record = std::mem::MaybeUninit::<INPUT_RECORD>::uninit();
            let mut read = 0;
            ensure!(
                unsafe { ReadConsoleInputW(self.input, record.as_mut_ptr(), 1, &mut read) } != 0,
                "读取控制台输入失败：{}",
                std::io::Error::last_os_error()
            );
            if read == 0 {
                break;
            }
            let record = unsafe { record.assume_init() };
            if u32::from(record.EventType) != KEY_EVENT {
                continue;
            }
            let key = unsafe { record.Event.KeyEvent };
            if key.bKeyDown == 0 {
                continue;
            }
            match key.wVirtualKeyCode {
                0x26 => bytes.extend_from_slice(b"\x1b[A"),
                0x28 => bytes.extend_from_slice(b"\x1b[B"),
                _ => {
                    if let Some(character) = self.characters.feed(unsafe { key.uChar.UnicodeChar })
                    {
                        if character != '\0' {
                            bytes.extend_from_slice(character.encode_utf8(&mut [0; 4]).as_bytes());
                        }
                    }
                }
            }
        }
        Ok(bytes)
    }
}

impl Drop for Mode {
    fn drop(&mut self) {
        unsafe {
            SetConsoleMode(self.input, self.input_mode);
            SetConsoleMode(self.output, self.output_mode);
        }
    }
}
