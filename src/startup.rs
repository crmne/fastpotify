//! Register Fastpotify to start when the user signs in.

use std::io;
use std::path::Path;

use crate::settings::StartupMode;

/// Updates the current user's startup entry for the current executable.
pub fn configure(mode: StartupMode) -> io::Result<()> {
    let executable = match mode {
        StartupMode::No => None,
        StartupMode::Minimized | StartupMode::Yes => Some(std::env::current_exe()?),
    };
    configure_for(mode, executable.as_deref())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn remove_if_present(path: &Path) -> io::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn write_atomic(path: &Path, text: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("tmp");
    std::fs::write(&temporary, text)?;
    if let Err(error) = std::fs::rename(&temporary, path) {
        let _ = std::fs::remove_file(&temporary);
        return Err(error);
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn configure_for(mode: StartupMode, executable: Option<&Path>) -> io::Result<()> {
    let base = directories::BaseDirs::new()
        .map(|dirs| dirs.config_dir().to_path_buf())
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "home directory not found"))?;
    let path = base.join("autostart").join("fastpotify.desktop");
    match mode {
        StartupMode::No => remove_if_present(&path),
        StartupMode::Minimized | StartupMode::Yes => {
            let executable = executable.expect("enabled startup mode has an executable");
            write_atomic(&path, &desktop_entry(executable, mode))
        }
    }
}

#[cfg(target_os = "linux")]
fn desktop_entry(executable: &Path, mode: StartupMode) -> String {
    let mut text = String::from("[Desktop Entry]\nType=Application\nName=Fastpotify\nExec=");
    text.push_str(&desktop_exec_arg(executable));
    if mode == StartupMode::Minimized {
        text.push_str(" --start-minimized");
    }
    text.push_str("\nTerminal=false\n");
    text
}

#[cfg(target_os = "linux")]
fn desktop_exec_arg(path: &Path) -> String {
    let mut quoted = String::from("\"");
    for character in path.to_string_lossy().chars() {
        if matches!(character, '\\' | '"' | '`' | '$') {
            quoted.push('\\');
        }
        quoted.push(character);
    }
    quoted.push('"');
    quoted
}

#[cfg(target_os = "macos")]
fn configure_for(mode: StartupMode, executable: Option<&Path>) -> io::Result<()> {
    let home = directories::BaseDirs::new()
        .map(|dirs| dirs.home_dir().to_path_buf())
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "home directory not found"))?;
    let path = home
        .join("Library")
        .join("LaunchAgents")
        .join("me.paolino.fastpotify.plist");
    match mode {
        StartupMode::No => remove_if_present(&path),
        StartupMode::Minimized | StartupMode::Yes => {
            let executable = executable.expect("enabled startup mode has an executable");
            write_atomic(&path, &launch_agent(executable, mode))
        }
    }
}

#[cfg(target_os = "macos")]
fn launch_agent(executable: &Path, mode: StartupMode) -> String {
    let mut text = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict>\n<key>Label</key><string>me.paolino.fastpotify</string>\n<key>ProgramArguments</key><array><string>",
    );
    text.push_str(&xml_text(executable));
    if mode == StartupMode::Minimized {
        text.push_str("</string><string>--start-minimized");
    }
    text.push_str("</string></array>\n<key>RunAtLoad</key><true/>\n</dict></plist>\n");
    text
}

#[cfg(target_os = "macos")]
fn xml_text(path: &Path) -> String {
    path.to_string_lossy()
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(windows)]
fn configure_for(mode: StartupMode, executable: Option<&Path>) -> io::Result<()> {
    use std::ffi::OsStr;
    use std::iter::once;
    use std::os::windows::ffi::OsStrExt;

    use windows_sys::Win32::Foundation::ERROR_FILE_NOT_FOUND;
    use windows_sys::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_SZ, RegCloseKey, RegDeleteValueW,
        RegOpenKeyExW, RegSetValueExW,
    };

    const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const VALUE_NAME: &str = "Fastpotify";

    let key_name: Vec<u16> = OsStr::new(RUN_KEY).encode_wide().chain(once(0)).collect();
    let value_name: Vec<u16> = OsStr::new(VALUE_NAME)
        .encode_wide()
        .chain(once(0))
        .collect();
    let mut key: HKEY = std::ptr::null_mut();
    let status = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            key_name.as_ptr(),
            0,
            KEY_SET_VALUE,
            &mut key,
        )
    };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(status as i32));
    }

    let result = match mode {
        StartupMode::No => {
            let status = unsafe { RegDeleteValueW(key, value_name.as_ptr()) };
            if status == 0 || status == ERROR_FILE_NOT_FOUND {
                Ok(())
            } else {
                Err(io::Error::from_raw_os_error(status as i32))
            }
        }
        StartupMode::Minimized | StartupMode::Yes => {
            let executable = executable.expect("enabled startup mode has an executable");
            let mut command = vec![b'"' as u16];
            command.extend(executable.as_os_str().encode_wide());
            command.push(b'"' as u16);
            if mode == StartupMode::Minimized {
                command.extend(OsStr::new(" --start-minimized").encode_wide());
            }
            command.push(0);
            let bytes = command.len() * std::mem::size_of::<u16>();
            let status = unsafe {
                RegSetValueExW(
                    key,
                    value_name.as_ptr(),
                    0,
                    REG_SZ,
                    command.as_ptr().cast(),
                    bytes as u32,
                )
            };
            if status == 0 {
                Ok(())
            } else {
                Err(io::Error::from_raw_os_error(status as i32))
            }
        }
    };
    let close_status = unsafe { RegCloseKey(key) };
    result.and(if close_status == 0 {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(close_status as i32))
    })
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn configure_for(_mode: StartupMode, _executable: Option<&Path>) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "automatic startup is not supported on this platform",
    ))
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "linux")]
    #[test]
    fn desktop_entry_only_passes_the_minimize_flag_for_minimized_startup() {
        use super::desktop_entry;
        use crate::settings::StartupMode;
        use std::path::Path;

        let visible = desktop_entry(Path::new("/opt/Fastpotify/fastpotify"), StartupMode::Yes);
        let minimized = desktop_entry(
            Path::new("/opt/Fastpotify/fastpotify"),
            StartupMode::Minimized,
        );
        assert!(!visible.contains("--start-minimized"));
        assert!(minimized.contains("--start-minimized"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn launch_agent_only_passes_the_minimize_flag_for_minimized_startup() {
        use super::launch_agent;
        use crate::settings::StartupMode;
        use std::path::Path;

        let visible = launch_agent(
            Path::new("/Applications/Fastpotify.app/Contents/MacOS/fastpotify"),
            StartupMode::Yes,
        );
        let minimized = launch_agent(
            Path::new("/Applications/Fastpotify.app/Contents/MacOS/fastpotify"),
            StartupMode::Minimized,
        );
        assert!(!visible.contains("--start-minimized"));
        assert!(minimized.contains("--start-minimized"));
    }
}
