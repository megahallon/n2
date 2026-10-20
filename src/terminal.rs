#[cfg(unix)]
mod unix {
    pub fn use_fancy() -> bool {
        unsafe {
            libc::isatty(/* stdout */ 1) == 1
        }
    }
}

#[cfg(unix)]
pub use unix::*;

#[cfg(windows)]
mod windows {
    use windows_sys::Win32::{Foundation::*, System::Console::*};

    pub fn use_fancy() -> bool {
        unsafe {
            let handle = GetStdHandle(STD_OUTPUT_HANDLE);
            let mut mode = 0;
            // Note: GetConsoleMode itself fails when not attached to a console.
            let ok = GetConsoleMode(handle, &mut mode) != 0;
            if ok {
                // Enable terminal processing so we can overwrite previous content.
                // Ignore errors.
                _ = SetConsoleMode(handle, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING);
            }
            ok
        }
    }
}

#[cfg(windows)]
pub use windows::*;

#[cfg(target_arch = "wasm32")]
mod wasm {
    pub fn use_fancy() -> bool {
        false
    }

    pub fn get_cols() -> Option<usize> {
        None
    }
}

#[cfg(target_arch = "wasm32")]
pub use wasm::*;
