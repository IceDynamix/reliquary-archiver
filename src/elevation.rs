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

fn escalate_to_admin() -> Result<(), Box<dyn std::error::Error>> {
    use std::os::windows::ffi::OsStrExt;

    use windows::Win32::System::Console::GetConsoleWindow;
    use windows::Win32::UI::Shell::{SEE_MASK_NO_CONSOLE, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW};
    use windows::Win32::UI::WindowsAndMessaging::{GW_OWNER, GetWindow, SW_SHOWNORMAL};
    use windows::core::{PCWSTR, w};

    let args_str = std::env::args().skip(1).collect::<Vec<_>>().join(" ");

    let exe_path = std::env::current_exe()
        .expect("Failed to get current exe")
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
