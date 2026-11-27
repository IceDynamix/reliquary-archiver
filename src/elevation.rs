//! Administrator checks for the pktmon backend, which can't capture without elevated privileges.

use tracing::error;

pub fn is_admin() -> bool {
    unsafe { windows::Win32::UI::Shell::IsUserAnAdmin().into() }
}

/// Explains why administrative privileges are needed and offers to restart the application with them
pub fn prompt_and_escalate() {
    println!();
    println!("===========================================================================================================");
    println!("                       Administrative privileges are required to capture packets");
    println!("===========================================================================================================");
    println!();
    println!("Reliquary Archiver now uses PacketMonitor (pktmon) to capture the game traffic instead of npcap on Windows");
    println!("Due to the way pktmon works, it requires running the application as an administrator");
    println!();
    println!("If you don't feel comfortable running the application as an administrator, you can download the pcap");
    println!("version of Reliquary Archiver from the GitHub releases page.");
    println!();
    if confirm("Would you like to restart the application with elevated privileges? (Y/n): ") {
        if let Err(e) = escalate_to_admin() {
            error!("Failed to escalate privileges: {}", e);
        }
    }
}

fn confirm(msg: &str) -> bool {
    use std::io::Write;

    print!("{}", msg);
    if std::io::stdout().flush().is_err() {
        return false;
    }

    let mut input = String::new();
    if std::io::stdin().read_line(&mut input).is_err() {
        return false;
    }

    input = input.trim().to_lowercase();
    input.starts_with("y") || input.is_empty()
}

pub fn escalate_to_admin() -> Result<(), Box<dyn std::error::Error>> {
    use std::os::windows::ffi::OsStrExt;

    use windows::Win32::System::Console::GetConsoleWindow;
    use windows::Win32::UI::Shell::{SEE_MASK_NO_CONSOLE, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW};
    use windows::Win32::UI::WindowsAndMessaging::{GW_OWNER, GetWindow, SW_SHOWNORMAL};
    use windows::core::{PCWSTR, w};

    let args_str = command_line(std::env::args_os().skip(1).map(|arg| arg.to_string_lossy().into_owned()));

    let exe_path = std::env::current_exe()?
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let args = args_str.encode_utf16().chain(Some(0)).collect::<Vec<_>>();

    unsafe {
        let mut options = SHELLEXECUTEINFOW {
            cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NO_CONSOLE,
            hwnd: GetWindow(GetConsoleWindow(), GW_OWNER).unwrap_or(GetConsoleWindow()),
            lpVerb: w!("runas"),
            lpFile: PCWSTR(exe_path.as_ptr()),
            lpParameters: PCWSTR(args.as_ptr()),
            lpDirectory: PCWSTR::null(),
            nShow: SW_SHOWNORMAL.0,
            lpIDList: std::ptr::null_mut(),
            lpClass: PCWSTR::null(),
            dwHotKey: 0,
            ..Default::default()
        };

        ShellExecuteExW(&mut options)?;
    };

    // Exit the current process since we launched a new elevated one
    std::process::exit(0);
}

/// Joins arguments into a command line that `CommandLineToArgvW` splits back into the same arguments
fn command_line(args: impl IntoIterator<Item = String>) -> String {
    let mut line = String::new();
    for arg in args {
        if !line.is_empty() {
            line.push(' ');
        }
        quote_argument(&arg, &mut line);
    }
    line
}

/// Quotes an argument if it needs it. Backslashes only escape when they come right before a quote,
/// so only those (and the ones before the closing quote) are doubled.
fn quote_argument(arg: &str, line: &mut String) {
    if !arg.is_empty() && !arg.contains([' ', '\t', '\n', '\u{b}', '"']) {
        line.push_str(arg);
        return;
    }

    line.push('"');
    let mut backslashes = 0;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                line.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                line.push('"');
                backslashes = 0;
            }
            c => {
                line.extend(std::iter::repeat_n('\\', backslashes));
                line.push(c);
                backslashes = 0;
            }
        }
    }
    line.extend(std::iter::repeat_n('\\', backslashes * 2));
    line.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(args: &[&str]) -> String {
        command_line(args.iter().map(|arg| arg.to_string()))
    }

    #[test]
    fn arguments_are_quoted_the_way_command_line_to_argv_reads_them() {
        assert_eq!(line(&["--etl", "capture.etl", "-v"]), "--etl capture.etl -v");
        assert_eq!(line(&["--log-path", r"C:\My Logs\log.json"]), r#"--log-path "C:\My Logs\log.json""#);
        assert_eq!(line(&[""]), r#""""#);
        // Backslashes are doubled only where they end up before a quote
        assert_eq!(line(&[r"C:\My Folder\"]), r#""C:\My Folder\\""#);
        assert_eq!(line(&[r"a\\b"]), r"a\\b");
        assert_eq!(line(&[r#"say "hi""#]), r#""say \"hi\"""#);
        assert_eq!(line(&[r#"a\"b"#]), r#""a\\\"b""#);
    }
}
