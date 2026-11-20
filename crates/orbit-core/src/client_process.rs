//! Read-only application lifecycle hints. They do not prove configuration load
//! or network connectivity, and never authorize configuration or credential use.
use crate::{dto::ClientView, system::SystemPaths};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Process {
    pub pid: u32,
    pub started_at_micros: i64,
    pub desktop: bool,
}

impl SystemPaths {
    pub(crate) fn client_processes(&self, client: &ClientView) -> Option<Vec<Process>> {
        #[cfg(test)]
        if let Some(observe) = &self.fixture_client_processes {
            return observe(client);
        }
        if self.isolated || !client.installed || !client.platform_supported {
            return None;
        }
        #[cfg(target_os = "macos")]
        return macos::collect(client);
        #[cfg(windows)]
        return windows::collect(client);
        #[cfg(not(any(target_os = "macos", windows)))]
        None
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use crate::dto::ClientKind;
    use std::{fs::File, io::Read, path::PathBuf};
    const PROC_ALL_PIDS: u32 = 1;

    pub(super) fn collect(client: &ClientView) -> Option<Vec<Process>> {
        let root = PathBuf::from(client.app_path.as_ref()?)
            .canonicalize()
            .ok()?;
        let mut info = Vec::new();
        File::open(root.join("Contents/Info.plist"))
            .ok()?
            .take(1024 * 1024 + 1)
            .read_to_end(&mut info)
            .ok()?;
        if info.len() > 1024 * 1024 {
            return None;
        }
        let plist = plist::Value::from_reader(std::io::Cursor::new(info)).ok()?;
        let dict = plist.as_dictionary()?;
        if dict.get("CFBundleIdentifier")?.as_string()? != client.bundle_id.as_deref()? {
            return None;
        }
        let name = dict.get("CFBundleExecutable")?.as_string()?;
        if name.is_empty() || name.contains('/') || name == "." || name == ".." {
            return None;
        }
        let executable = root.join("Contents/MacOS").join(name).canonicalize().ok()?;
        // Reserve room for concurrent launches; refuse a truncated enumeration.
        let count = unsafe { libc::proc_listpids(PROC_ALL_PIDS, 0, std::ptr::null_mut(), 0) };
        if count <= 0 || count > 4 * 32768 {
            return None;
        }
        let mut pids = vec![0i32; count as usize / 4 + 1024];
        let bytes = unsafe {
            libc::proc_listpids(
                PROC_ALL_PIDS,
                0,
                pids.as_mut_ptr().cast(),
                (pids.len() * 4) as i32,
            )
        };
        if bytes <= 0 || bytes as usize >= pids.len() * 4 {
            return None;
        }
        let mut processes = Vec::new();
        for pid in pids
            .into_iter()
            .take(bytes as usize / 4)
            .filter(|pid| *pid > 0)
        {
            let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
            if unsafe {
                libc::proc_pidinfo(
                    pid,
                    libc::PROC_PIDTBSDINFO,
                    0,
                    (&mut info as *mut libc::proc_bsdinfo).cast(),
                    std::mem::size_of_val(&info) as i32,
                )
            } != std::mem::size_of_val(&info) as i32
                || info.pbi_uid != unsafe { libc::geteuid() }
            {
                continue;
            }
            let mut path = [0u8; 4096];
            if unsafe { libc::proc_pidpath(pid, path.as_mut_ptr().cast(), path.len() as u32) } <= 0
            {
                let command: Vec<u8> = info
                    .pbi_comm
                    .iter()
                    .map(|byte| *byte as u8)
                    .take_while(|byte| *byte != 0)
                    .collect();
                if command.starts_with(b"Codex")
                    || command.starts_with(b"Claude")
                    || command.as_slice() == b"codex"
                {
                    return None;
                }
                continue;
            }
            let len = path.iter().position(|b| *b == 0)?;
            let path = PathBuf::from(std::ffi::OsString::from_vec(path[..len].to_vec()));
            let desktop = path == executable;
            let related = path.starts_with(root.join("Contents/Frameworks"))
                && path
                    .file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with(&format!("{name} Helper")));
            let backend = client.client == ClientKind::Codex
                && path.starts_with(root.join("Contents/Resources"))
                && path.file_name().is_some_and(|n| n == "codex");
            if desktop || related || backend {
                processes.push(Process {
                    pid: pid as u32,
                    started_at_micros: i64::try_from(info.pbi_start_tvsec)
                        .ok()?
                        .checked_mul(1_000_000)?
                        .checked_add(i64::try_from(info.pbi_start_tvusec).ok()?)?,
                    desktop,
                });
            }
        }
        Some(processes)
    }
    use std::os::unix::ffi::OsStringExt;

    #[cfg(test)]
    mod tests {
        use super::*;
        struct Child(std::process::Child);
        impl Drop for Child {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        fn executable(path: &std::path::Path) {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::copy("/bin/sleep", path).unwrap();
        }
        #[test]
        fn native_macos_process_observer_uses_bundle_path_and_launch_time() {
            for (kind, name, identity) in [
                (ClientKind::Codex, "Codex", "com.openai.codex"),
                (
                    ClientKind::Claude,
                    "Claude",
                    "com.anthropic.claudefordesktop",
                ),
            ] {
                let dir = tempfile::tempdir().unwrap();
                let app = dir.path().join(format!("{name}.app"));
                let main = app.join("Contents/MacOS").join(name);
                executable(&main);
                std::fs::write(app.join("Contents/Info.plist"), format!(r#"<?xml version="1.0"?><plist version="1.0"><dict><key>CFBundleIdentifier</key><string>{identity}</string><key>CFBundleExecutable</key><string>{name}</string></dict></plist>"#)).unwrap();
                let unrelated = dir.path().join("unrelated").join(name);
                executable(&unrelated);
                let unrelated = Child(
                    std::process::Command::new(unrelated)
                        .arg("30")
                        .spawn()
                        .unwrap(),
                );
                let mut client: ClientView = serde_json::from_value(serde_json::json!({
                    "client":kind,"installed":true,"display_name":name,"version":"synthetic","bundle_id":identity,
                    "app_path":app,"config_paths":[],"platform_supported":true,"managed":"no",
                    "configuration_state":"saved","loaded_state":"restart_required","connection_state":"not_tested",
                    "configuration_revision":null,"loaded_evidence":null,"connection_evidence":null,"manual_steps":[]
                })).unwrap();
                assert!(
                    collect(&client).unwrap().is_empty(),
                    "same-name programs elsewhere must not count"
                );
                let before = chrono::Utc::now().timestamp_micros();
                let mut desktop =
                    Child(std::process::Command::new(&main).arg("30").spawn().unwrap());
                let until = std::time::Instant::now() + std::time::Duration::from_secs(2);
                let observed = loop {
                    let observed = collect(&client).unwrap();
                    if observed.iter().any(|p| p.pid == desktop.0.id()) {
                        break observed;
                    }
                    assert!(
                        std::time::Instant::now() < until,
                        "owned child did not become observable"
                    );
                    std::thread::sleep(std::time::Duration::from_millis(10));
                };
                assert_eq!(observed.len(), 1);
                assert!(observed[0].desktop);
                assert!(observed[0].started_at_micros >= before);
                assert!(observed.iter().all(|p| p.pid != unrelated.0.id()));
                desktop.0.kill().unwrap();
                desktop.0.wait().unwrap();
                assert!(collect(&client).unwrap().is_empty());
                client.bundle_id = Some("unrelated.identity".into());
                assert!(collect(&client).is_none());
            }
        }
    }
}

// Also typecheck the Windows collector in macOS tests. Unused FFI calls are not
// linked on that host; native Windows execution remains a separate check.
#[cfg(any(windows, test))]
#[path = "client_process_windows.rs"]
mod windows;
