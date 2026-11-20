//! Minimal native Win32 observation; no shell, WMI or visible child console.
#![cfg_attr(not(windows), allow(dead_code))]
use super::*;
use crate::dto::ClientKind;
use std::{collections::HashSet, ffi::c_void, mem::size_of};
type Handle = *mut c_void;
#[repr(C)]
struct Entry {
    size: u32,
    usage: u32,
    pid: u32,
    heap: usize,
    module: u32,
    threads: u32,
    parent: u32,
    priority: i32,
    flags: u32,
    name: [u16; 260],
}
#[repr(C)]
#[derive(Default)]
struct FileTime {
    low: u32,
    high: u32,
}
#[cfg_attr(windows, link(name = "kernel32"))]
unsafe extern "system" {
    fn CreateToolhelp32Snapshot(flags: u32, pid: u32) -> Handle;
    fn Process32FirstW(snapshot: Handle, entry: *mut Entry) -> i32;
    fn Process32NextW(snapshot: Handle, entry: *mut Entry) -> i32;
    fn CloseHandle(handle: Handle) -> i32;
    fn OpenProcess(access: u32, inherit: i32, pid: u32) -> Handle;
    fn QueryFullProcessImageNameW(
        process: Handle,
        flags: u32,
        name: *mut u16,
        length: *mut u32,
    ) -> i32;
    fn GetProcessTimes(
        process: Handle,
        created: *mut FileTime,
        exited: *mut FileTime,
        kernel: *mut FileTime,
        user: *mut FileTime,
    ) -> i32;
    fn GetCurrentProcessId() -> u32;
    fn ProcessIdToSessionId(pid: u32, session: *mut u32) -> i32;
    fn GetLastError() -> u32;
    fn WaitForSingleObject(handle: Handle, millis: u32) -> u32;
}
#[cfg_attr(windows, link(name = "user32"))]
unsafe extern "system" {
    fn EnumWindows(callback: unsafe extern "system" fn(Handle, isize) -> i32, data: isize) -> i32;
    fn GetWindowThreadProcessId(window: Handle, pid: *mut u32) -> u32;
    fn GetClassNameW(window: Handle, name: *mut u16, length: i32) -> i32;
}
struct Owned(Handle);
impl Drop for Owned {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
fn same_path(a: &str, b: &str) -> bool {
    a.replace('/', "\\")
        .trim_start_matches("\\\\?\\")
        .eq_ignore_ascii_case(b.replace('/', "\\").trim_start_matches("\\\\?\\"))
}
fn unix_micros(time: FileTime) -> Option<i64> {
    let ticks = ((time.high as u64) << 32) | time.low as u64;
    i64::try_from(ticks.checked_sub(116_444_736_000_000_000)? / 10).ok()
}
unsafe extern "system" fn window(window: Handle, data: isize) -> i32 {
    let mut class = [0u16; 64];
    let length = unsafe { GetClassNameW(window, class.as_mut_ptr(), class.len() as i32) };
    if length > 0
        && String::from_utf16(&class[..length as usize])
            .ok()
            .as_deref()
            == Some("Chrome_WidgetWin_1")
    {
        let mut pid = 0;
        unsafe {
            GetWindowThreadProcessId(window, &mut pid);
        }
        unsafe { &mut *(data as *mut HashSet<u32>) }.insert(pid);
    }
    1
}
pub(super) fn collect(client: &ClientView) -> Option<Vec<Process>> {
    let root = client.app_path.as_deref()?.trim_end_matches(['/', '\\']);
    let leaf = if client.client == ClientKind::Codex {
        "Codex.exe"
    } else {
        "Claude.exe"
    };
    let main = format!("{root}\\{leaf}");
    let backend = format!("{root}\\resources\\codex.exe");
    let mut session = 0;
    if unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut session) } == 0 {
        return None;
    }
    let mut desktop_pids = HashSet::<u32>::new();
    if unsafe { EnumWindows(window, (&mut desktop_pids as *mut HashSet<u32>) as isize) } == 0 {
        return None;
    }
    let snapshot = unsafe { CreateToolhelp32Snapshot(2, 0) };
    if snapshot as isize == -1 {
        return None;
    }
    let snapshot = Owned(snapshot);
    let mut entry: Entry = unsafe { std::mem::zeroed() };
    entry.size = size_of::<Entry>() as u32;
    let mut next = unsafe { Process32FirstW(snapshot.0, &mut entry) };
    let mut result = Vec::new();
    let mut count = 0;
    while next != 0 {
        count += 1;
        if count > 32768 {
            return None;
        }
        let end = entry.name.iter().position(|n| *n == 0)?;
        let name = String::from_utf16(&entry.name[..end]).ok()?;
        if name.eq_ignore_ascii_case(leaf)
            || (client.client == ClientKind::Codex && name.eq_ignore_ascii_case("codex.exe"))
        {
            let mut other_session = 0;
            let session_known = unsafe { ProcessIdToSessionId(entry.pid, &mut other_session) } != 0;
            if !session_known && unsafe { GetLastError() } != 87 {
                return None;
            }
            if session_known && other_session == session {
                let handle = unsafe { OpenProcess(0x1000 | 0x0010_0000, 0, entry.pid) };
                if handle.is_null() {
                    if unsafe { GetLastError() } != 87 {
                        return None;
                    }
                } else {
                    let handle = Owned(handle);
                    let mut path = vec![0u16; 32768];
                    let mut length = path.len() as u32;
                    if unsafe {
                        QueryFullProcessImageNameW(handle.0, 0, path.as_mut_ptr(), &mut length)
                    } == 0
                    {
                        if unsafe { WaitForSingleObject(handle.0, 0) } != 0 {
                            return None;
                        }
                    } else {
                        let path = String::from_utf16(&path[..length as usize]).ok()?;
                        let is_main = same_path(&path, &main);
                        if is_main
                            || (client.client == ClientKind::Codex && same_path(&path, &backend))
                        {
                            let (mut created, mut exited, mut kernel, mut user) = (
                                FileTime::default(),
                                FileTime::default(),
                                FileTime::default(),
                                FileTime::default(),
                            );
                            if unsafe {
                                GetProcessTimes(
                                    handle.0,
                                    &mut created,
                                    &mut exited,
                                    &mut kernel,
                                    &mut user,
                                )
                            } == 0
                            {
                                return None;
                            }
                            result.push(Process {
                                pid: entry.pid,
                                started_at_micros: unix_micros(created)?,
                                desktop: is_main && desktop_pids.contains(&entry.pid),
                            });
                        }
                    }
                }
            }
        }
        next = unsafe { Process32NextW(snapshot.0, &mut entry) };
    }
    if unsafe { GetLastError() } != 18 {
        return None;
    }
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn windows_process_identity_and_clock_are_not_name_or_pid_only() {
        assert_eq!(size_of::<Entry>(), 568);
        assert!(same_path(
            r"\\?\C:\Users\Tester\Codex.exe",
            r"c:/Users/Tester/codex.exe"
        ));
        assert!(!same_path(r"C:\Other\Codex.exe", r"C:\Apps\Codex.exe"));
        assert_eq!(unix_micros(FileTime { low: 0, high: 0 }), None);
        let ticks = 116_444_736_000_000_000u64 + 1_234_567 * 10;
        assert_eq!(
            unix_micros(FileTime {
                low: ticks as u32,
                high: (ticks >> 32) as u32
            }),
            Some(1_234_567)
        );
    }
}
