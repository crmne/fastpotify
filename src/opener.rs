//! Handing a link or a folder to the desktop.
//!
//! Everywhere but Windows this is the `open` crate, which picks the right
//! launcher for the desktop it finds. Windows gets `ShellExecuteW`, the
//! call Explorer itself makes when someone clicks a link, for two
//! reasons. The crate's Windows launcher runs PowerShell, and a Windows
//! 10 without PowerShell on its path answers with "the system cannot
//! find the file specified" and no browser, which is a sign-in button
//! that does nothing (#107). Its usual alternative, `cmd /c start`, has
//! a worse problem: `cmd` expands `%name%` inside quotes, and a sign-in
//! URL is full of percent escapes, so `redirect_uri=http%3A%2F%2F...`
//! reaches the browser with a piece eaten out of it.

use std::ffi::OsStr;
use std::io;

/// Opens a URL or a path in whatever the desktop uses for it.
#[cfg(not(windows))]
pub fn open(target: impl AsRef<OsStr>) -> io::Result<()> {
    open::that(target.as_ref())
}

/// Opens a URL or a path in whatever the desktop uses for it.
#[cfg(windows)]
pub fn open(target: impl AsRef<OsStr>) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    fn wide(text: &OsStr) -> Vec<u16> {
        text.encode_wide().chain(std::iter::once(0)).collect()
    }

    // A verb can be handed to a COM shell extension, which wants the
    // thread ready for one. Callers are worker threads that have done
    // nothing of the sort. A thread already initialised says so and
    // keeps the mode it has, which is not a failure to open anything.
    unsafe { CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32) };

    let file = wide(target.as_ref());
    let verb = wide(OsStr::new("open"));
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            file.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    // Anything above 32 opened. At or below it the value is one of the
    // old SE_ERR_ codes, which the thread's last error carries too.
    if result as isize > 32 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}
