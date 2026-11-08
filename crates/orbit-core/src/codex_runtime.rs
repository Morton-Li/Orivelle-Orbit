//! The reviewed macOS standard-local stdio profile, before OS collection.
//! No launch data, environment values or executable paths enter public errors.

use crate::{codex_catalog::SUPPORTED_CLI_VERSION, identity::sha256, system::CodexRuntimeIdentity};
use serde::Serialize;
use std::path::{Component, Path, PathBuf};

const MAX_LAUNCH_BYTES: usize = 1024 * 1024;
const MAX_ARGS: usize = 16;
const MAX_ENV: usize = 4096;
const MAX_PATH_BYTES: usize = 4096;
const CLI_RELATIVE: &str = "Contents/Resources/codex-cli/CodexCLI.app/Contents/MacOS/codex";
const CLI_BYTES: u64 = 249_954_368;
const CLI_SHA256: &str = "1afb28c01d7d49e892bd75f11076b10f1451e34e0ee308a95e5ab7f69f9c84fe";
const CLI_CDHASH: [u8; 20] = [
    0xdf, 0xfd, 0x79, 0x2d, 0x4a, 0xf0, 0x2d, 0x71, 0x43, 0xfd, 0x94, 0x05, 0x5c, 0xf9, 0xe3, 0x70,
    0x1b, 0xca, 0x74, 0x6c,
];
const PINS: [(&str, u64, &str); 4] = [
    (
        "Contents/Resources/app.asar",
        582_430_926,
        "5ddc4e48259aa6f979e04529b1a5a4ae54b0f5c4ad6db5c6e004924c10916ddb",
    ),
    (
        "Contents/Info.plist",
        20_738,
        "8520b12f2bcf41fe7bb2d7d0fca7043ed495497c5a5405aafea01933f1b53058",
    ),
    (
        "Contents/Resources/codex-cli/codex-package.json",
        211,
        "558c7aabb05ccc87bdb8c8514f87659fec72929a39919c7156f8b8889f7589b3",
    ),
    (
        "Contents/Resources/codex-cli/CodexCLI.app/Contents/Info.plist",
        525,
        "c40b0e6b87b94ef5a232ab8312678c5db2156428f57c3f3d6c32525373be7f59",
    ),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Unverified {
    Read,
    Launch,
    Argument,
    Environment,
    Root,
    Process,
    Image,
    Connection,
    Ambiguous,
    Profile,
    Serialization,
}
type Observed<T> = Result<T, Unverified>;

#[derive(Clone, PartialEq, Eq, Serialize)]
struct Launch {
    home: PathBuf,
    codex_home: Option<PathBuf>,
    app_tools: bool,
    code_review: bool,
    secret_auth_storage: bool,
}
impl Launch {
    fn requested_root(&self) -> PathBuf {
        self.codex_home
            .clone()
            .unwrap_or_else(|| self.home.join(".codex"))
    }
}

fn path_value(bytes: &[u8]) -> Observed<PathBuf> {
    let text = std::str::from_utf8(bytes).map_err(|_| Unverified::Environment)?;
    if bytes.is_empty() || bytes.len() > MAX_PATH_BYTES || text.chars().any(char::is_control) {
        return Err(Unverified::Environment);
    }
    let path = PathBuf::from(text);
    if !path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, Component::ParentDir | Component::CurDir))
    {
        return Err(Unverified::Environment);
    }
    Ok(path)
}

fn nul_item<'a>(raw: &'a [u8], cursor: &mut usize) -> Observed<&'a [u8]> {
    let rest = raw.get(*cursor..).ok_or(Unverified::Launch)?;
    let end = rest
        .iter()
        .position(|byte| *byte == 0)
        .ok_or(Unverified::Launch)?;
    *cursor += end + 1;
    Ok(&rest[..end])
}

/// Parse only the established role and the named safe environment fields.
/// The collector must keep the bounded raw kernel buffer in Zeroizing storage.
fn parse_launch(raw: &[u8], executable: &Path) -> Observed<Launch> {
    if raw.len() < 4 || raw.len() > MAX_LAUNCH_BYTES {
        return Err(Unverified::Read);
    }
    let argc = i32::from_ne_bytes(raw[..4].try_into().map_err(|_| Unverified::Launch)?);
    if argc < 1 || argc as usize > MAX_ARGS {
        return Err(Unverified::Launch);
    }
    let expected = executable.to_str().ok_or(Unverified::Launch)?.as_bytes();
    let mut cursor = 4;
    if nul_item(raw, &mut cursor)? != expected {
        return Err(Unverified::Launch);
    }
    while raw.get(cursor) == Some(&0) {
        cursor += 1;
    }
    let mut args = Vec::with_capacity(argc as usize);
    for _ in 0..argc {
        args.push(nul_item(raw, &mut cursor)?);
    }
    if args[0] != expected {
        return Err(Unverified::Launch);
    }
    let mut configs = [None; 4];
    let mut app_server = false;
    let mut analytics = false;
    let mut index = 1;
    while index < args.len() {
        match args[index] {
            b"-c" => {
                index += 1;
                let config = args.get(index).ok_or(Unverified::Argument)?;
                let split = config
                    .iter()
                    .position(|byte| *byte == b'=')
                    .ok_or(Unverified::Argument)?;
                let key = &config[..split];
                let value = match &config[split + 1..] {
                    b"true" => true,
                    b"false" => false,
                    _ => return Err(Unverified::Argument),
                };
                let slot = match key {
                    b"features.code_mode_host" if value => 0,
                    b"plugins.codex-app-tools@openai-bundled.mcp_servers.codex_app.enabled" => 1,
                    b"plugins.code-review@openai-bundled.mcp_servers.code-review.enabled" => 2,
                    b"features.secret_auth_storage" if value => 3,
                    _ => return Err(Unverified::Argument),
                };
                if configs[slot].replace(value).is_some() {
                    return Err(Unverified::Argument);
                }
            }
            b"app-server" if !app_server => app_server = true,
            b"--analytics-default-enabled" if !analytics => analytics = true,
            _ => return Err(Unverified::Argument),
        }
        index += 1;
    }
    if !app_server || !analytics || configs[..3].iter().any(Option::is_none) {
        return Err(Unverified::Argument);
    }
    let mut home = None;
    let mut codex_home = None;
    let mut originator = false;
    let mut cli_path = false;
    let mut count = 0;
    let mut env_terminated = false;
    while cursor < raw.len() {
        let item = nul_item(raw, &mut cursor)?;
        if item.is_empty() {
            if raw[cursor..].iter().any(|byte| *byte != 0) {
                return Err(Unverified::Environment);
            }
            env_terminated = true;
            break;
        }
        count += 1;
        if count > MAX_ENV {
            return Err(Unverified::Environment);
        }
        let split = item
            .iter()
            .position(|byte| *byte == b'=')
            .ok_or(Unverified::Environment)?;
        // Unrecognized values are neither decoded, copied nor formatted.
        match &item[..split] {
            b"HOME" if home.is_none() => home = Some(path_value(&item[split + 1..])?),
            b"CODEX_HOME" if codex_home.is_none() => {
                codex_home = Some(path_value(&item[split + 1..])?)
            }
            b"CODEX_INTERNAL_ORIGINATOR_OVERRIDE" if !originator => {
                if &item[split + 1..] != b"Codex Desktop" {
                    return Err(Unverified::Environment);
                }
                originator = true;
            }
            b"CODEX_CLI_PATH" if !cli_path => {
                if &item[split + 1..] != expected {
                    return Err(Unverified::Environment);
                }
                cli_path = true;
            }
            b"HOME" | b"CODEX_HOME" | b"CODEX_INTERNAL_ORIGINATOR_OVERRIDE" | b"CODEX_CLI_PATH" => {
                return Err(Unverified::Environment);
            }
            _ => {}
        }
    }
    if !env_terminated || !originator || !cli_path {
        return Err(Unverified::Environment);
    }
    Ok(Launch {
        home: home.ok_or(Unverified::Environment)?,
        codex_home,
        app_tools: configs[1].ok_or(Unverified::Argument)?,
        code_review: configs[2].ok_or(Unverified::Argument)?,
        secret_auth_storage: configs[3].unwrap_or(false),
    })
}

#[derive(Clone, PartialEq, Eq, Serialize)]
struct Process {
    pid: u32,
    parent: u32,
    uid: u32,
    started: (u64, u64),
    executable: PathBuf,
}
#[derive(Clone, PartialEq, Eq, Serialize)]
struct FileEntity {
    path: PathBuf,
    device: u64,
    inode: u64,
    bytes: u64,
    modified: (i64, i64),
    changed: (i64, i64),
}
#[derive(Clone, Serialize)]
struct Image {
    before: FileEntity,
    after: FileEntity,
    sha256: String,
    cdhash: [u8; 20],
    signature_valid: bool,
    running_matches_disk: bool,
}
#[derive(Clone, Serialize)]
struct SignedProcess {
    before: Process,
    after: Process,
    image: Image,
}
#[derive(Clone, Serialize)]
struct Material {
    before: FileEntity,
    after: FileEntity,
    sha256: String,
}
#[derive(Clone, PartialEq, Eq, Serialize)]
struct Root {
    requested: PathBuf,
    canonical: PathBuf,
    device: u64,
    inode: u64,
}
#[derive(Clone, PartialEq, Eq, Serialize)]
struct Endpoint {
    fd: i32,
    unix_stream: bool,
    connected: bool,
    handle: u64,
    peer: u64,
}
#[derive(Clone)]
struct Stdio {
    desktop: Vec<Endpoint>,
    stdin: Endpoint,
    stdout: Endpoint,
}
#[derive(Clone, PartialEq, Eq, Serialize)]
struct Binding {
    stdin: (Endpoint, Endpoint),
    stdout: (Endpoint, Endpoint),
}
#[derive(Clone)]
struct Backend {
    process: SignedProcess,
    launch_before: Launch,
    launch_after: Launch,
    root_before: Root,
    root_after: Root,
    stdio_before: Stdio,
    stdio_after: Stdio,
}
struct Capture {
    desktop: SignedProcess,
    bundle: PathBuf,
    materials: Vec<Material>,
    cli_version: String,
    backends: Vec<Backend>,
}
struct Target<'a> {
    bundle: &'a Path,
    desktop_executable: &'a Path,
    configuration_root: &'a Path,
    uid: u32,
}

fn signed_process(process: &SignedProcess, path: &Path, uid: u32) -> Observed<()> {
    if process.before != process.after
        || process.before.pid == 0
        || process.before.uid != uid
        || process.before.started.0 == 0
        || process.before.started.1 >= 1_000_000
        || process.before.executable != path
    {
        return Err(Unverified::Process);
    }
    let image = &process.image;
    if image.before != image.after
        || image.before.path != path
        || image.before.bytes == 0
        || !image.signature_valid
        || !image.running_matches_disk
        || image.cdhash == [0; 20]
        || image.sha256.len() != 64
        || !image
            .sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(Unverified::Image);
    }
    Ok(())
}

fn stdio_binding(stdio: &Stdio) -> Observed<Binding> {
    if stdio.desktop.len() > 2048
        || stdio.stdin.fd != 0
        || stdio.stdout.fd != 1
        || stdio.stdin.handle == stdio.stdout.handle
    {
        return Err(Unverified::Connection);
    }
    let pair = |child: &Endpoint| -> Observed<(Endpoint, Endpoint)> {
        if !child.unix_stream || !child.connected || child.handle == 0 || child.peer == 0 {
            return Err(Unverified::Connection);
        }
        let mut peers = stdio.desktop.iter().filter(|parent| {
            parent.fd >= 0
                && parent.unix_stream
                && parent.connected
                && parent.handle == child.peer
                && parent.peer == child.handle
        });
        let parent = peers.next().ok_or(Unverified::Connection)?;
        if peers.next().is_some() {
            return Err(Unverified::Connection);
        }
        Ok((parent.clone(), child.clone()))
    };
    let binding = Binding {
        stdin: pair(&stdio.stdin)?,
        stdout: pair(&stdio.stdout)?,
    };
    if binding.stdin.0.fd == binding.stdout.0.fd {
        return Err(Unverified::Connection);
    }
    Ok(binding)
}

fn decide(target: &Target<'_>, capture: &Capture) -> Observed<CodexRuntimeIdentity> {
    if capture.bundle != target.bundle
        || capture.cli_version != SUPPORTED_CLI_VERSION
        || capture.materials.len() != PINS.len()
    {
        return Err(Unverified::Profile);
    }
    signed_process(&capture.desktop, target.desktop_executable, target.uid)?;
    for (material, (relative, bytes, hash)) in capture.materials.iter().zip(PINS) {
        if material.before != material.after
            || material.before.path != target.bundle.join(relative)
            || material.before.bytes != bytes
            || material.sha256 != hash
        {
            return Err(Unverified::Profile);
        }
    }
    if capture.backends.len() != 1 {
        return Err(Unverified::Ambiguous);
    }
    let backend = &capture.backends[0];
    signed_process(
        &backend.process,
        &target.bundle.join(CLI_RELATIVE),
        target.uid,
    )?;
    if backend.process.before.pid == capture.desktop.before.pid
        || backend.process.before.parent != capture.desktop.before.pid
        || backend.process.before.started < capture.desktop.before.started
    {
        return Err(Unverified::Process);
    }
    if backend.process.image.before.bytes != CLI_BYTES
        || backend.process.image.sha256 != CLI_SHA256
        || backend.process.image.cdhash != CLI_CDHASH
    {
        return Err(Unverified::Image);
    }
    if backend.launch_before != backend.launch_after {
        return Err(Unverified::Launch);
    }
    let root = &backend.root_before;
    if root != &backend.root_after
        || root.requested != backend.launch_before.requested_root()
        || root.canonical != target.configuration_root
        || root.inode == 0
    {
        return Err(Unverified::Root);
    }
    let connection = stdio_binding(&backend.stdio_before)?;
    if connection != stdio_binding(&backend.stdio_after)? {
        return Err(Unverified::Connection);
    }
    let relevant = (
        &capture.bundle,
        &capture.materials,
        &capture.desktop.before,
        &capture.desktop.image,
        &backend.process.before,
        &backend.process.image,
        &backend.launch_before,
        root,
        connection,
    );
    let bytes = serde_json::to_vec(&(1u32, SUPPORTED_CLI_VERSION, relevant))
        .map_err(|_| Unverified::Serialization)?;
    Ok(CodexRuntimeIdentity {
        cli_version: SUPPORTED_CLI_VERSION.into(),
        revision: sha256(&bytes),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use zeroize::Zeroizing;

    const BUNDLE: &str = "/Applications/ChatGPT.app";
    const DESKTOP: &str = "/Applications/ChatGPT.app/Contents/MacOS/ChatGPT";
    const CLI: &str =
        "/Applications/ChatGPT.app/Contents/Resources/codex-cli/CodexCLI.app/Contents/MacOS/codex";
    const HOME: &str = "/Users/synthetic";
    const CONFIG: &str = "/Users/synthetic/.codex";

    fn arguments() -> Vec<Vec<u8>> {
        [
            CLI,
            "-c",
            "features.code_mode_host=true",
            "app-server",
            "--analytics-default-enabled",
            "-c",
            "plugins.codex-app-tools@openai-bundled.mcp_servers.codex_app.enabled=false",
            "-c",
            "plugins.code-review@openai-bundled.mcp_servers.code-review.enabled=true",
        ]
        .map(|value| value.as_bytes().to_vec())
        .to_vec()
    }
    fn environment() -> Vec<Vec<u8>> {
        [
            format!("HOME={HOME}"),
            format!("CODEX_CLI_PATH={CLI}"),
            "CODEX_INTERNAL_ORIGINATOR_OVERRIDE=Codex Desktop".into(),
        ]
        .map(String::into_bytes)
        .to_vec()
    }
    fn kernel(args: &[Vec<u8>], env: &[Vec<u8>]) -> Zeroizing<Vec<u8>> {
        let mut raw = Zeroizing::new(Vec::new());
        raw.extend_from_slice(&(args.len() as i32).to_ne_bytes());
        raw.extend_from_slice(CLI.as_bytes());
        raw.extend_from_slice(&[0, 0, 0]);
        for value in args.iter().chain(env) {
            raw.extend_from_slice(value);
            raw.push(0);
        }
        raw.extend_from_slice(&[0, 0]);
        raw
    }
    fn launch() -> Launch {
        parse_launch(&kernel(&arguments(), &environment()), Path::new(CLI))
            .ok()
            .expect("synthetic launch")
    }
    fn entity(path: &Path, bytes: u64, inode: u64) -> FileEntity {
        FileEntity {
            path: path.into(),
            device: 7,
            inode,
            bytes,
            modified: (11, 1),
            changed: (12, 1),
        }
    }
    fn process(path: &Path, pid: u32, parent: u32, bytes: u64, hash: &str) -> SignedProcess {
        let stamp = Process {
            pid,
            parent,
            uid: 501,
            started: (pid as u64, 44),
            executable: path.into(),
        };
        let file = entity(path, bytes, pid as u64);
        SignedProcess {
            before: stamp.clone(),
            after: stamp,
            image: Image {
                before: file.clone(),
                after: file,
                sha256: hash.into(),
                cdhash: CLI_CDHASH,
                signature_valid: true,
                running_matches_disk: true,
            },
        }
    }
    fn endpoint(fd: i32, handle: u64, peer: u64) -> Endpoint {
        Endpoint {
            fd,
            handle,
            peer,
            unix_stream: true,
            connected: true,
        }
    }
    fn fixture() -> (Target<'static>, Capture) {
        let target = Target {
            bundle: Path::new(BUNDLE),
            desktop_executable: Path::new(DESKTOP),
            configuration_root: Path::new(CONFIG),
            uid: 501,
        };
        let stdio = Stdio {
            desktop: vec![endpoint(50, 10, 20), endpoint(51, 11, 21)],
            stdin: endpoint(0, 20, 10),
            stdout: endpoint(1, 21, 11),
        };
        let root = Root {
            requested: CONFIG.into(),
            canonical: CONFIG.into(),
            device: 9,
            inode: 2000,
        };
        let backend = Backend {
            process: process(Path::new(CLI), 200, 100, CLI_BYTES, CLI_SHA256),
            launch_before: launch(),
            launch_after: launch(),
            root_before: root.clone(),
            root_after: root,
            stdio_before: stdio.clone(),
            stdio_after: stdio,
        };
        let materials = PINS
            .iter()
            .enumerate()
            .map(|(index, (path, bytes, hash))| {
                let file = entity(&Path::new(BUNDLE).join(path), *bytes, 1000 + index as u64);
                Material {
                    before: file.clone(),
                    after: file,
                    sha256: (*hash).into(),
                }
            })
            .collect();
        (
            target,
            Capture {
                desktop: process(Path::new(DESKTOP), 100, 1, 4096, &"a".repeat(64)),
                bundle: BUNDLE.into(),
                materials,
                cli_version: SUPPORTED_CLI_VERSION.into(),
                backends: vec![backend],
            },
        )
    }

    #[test]
    fn codex_runtime_parser_accepts_only_the_standard_local_role_and_safe_booleans() {
        let ordinary = launch();
        assert!(!ordinary.app_tools);
        assert!(ordinary.code_review);
        assert!(!ordinary.secret_auth_storage);
        assert!(ordinary.requested_root() == Path::new(CONFIG));
        let mut args = arguments();
        args.extend([
            b"-c".to_vec(),
            b"features.secret_auth_storage=true".to_vec(),
        ]);
        let mut env = environment();
        env.push(format!("CODEX_HOME={CONFIG}").into_bytes());
        let explicit = parse_launch(&kernel(&args, &env), Path::new(CLI))
            .ok()
            .expect("explicit root");
        assert!(explicit.secret_auth_storage);
        assert!(explicit.requested_root() == ordinary.requested_root());
    }

    #[test]
    fn codex_runtime_parser_rejects_unknown_role_overrides_without_retaining_secrets() {
        let sentinel = "synthetic-private-value-never-in-output";
        let mut env = environment();
        env.push(format!("UNRELATED_CREDENTIAL={sentinel}").into_bytes());
        env.push(b"UNKNOWN_NON_UTF8=\xff\xfe".to_vec());
        let parsed = parse_launch(&kernel(&arguments(), &env), Path::new(CLI))
            .ok()
            .expect("ignored private env");
        assert!(!serde_json::to_string(&parsed).unwrap().contains(sentinel));
        for tail in [
            vec![b"--listen".to_vec(), b"ws://remote.invalid".to_vec()],
            vec![b"--profile".to_vec(), b"custom".to_vec()],
            vec![b"-c".to_vec(), format!("model=\"{sentinel}\"").into_bytes()],
            vec![
                b"-c".to_vec(),
                b"features.secret_auth_storage=false".to_vec(),
            ],
            vec![b"-c".to_vec(), b"features.code_mode_host=true".to_vec()],
        ] {
            let mut args = arguments();
            args.extend(tail);
            let error = parse_launch(&kernel(&args, &env), Path::new(CLI))
                .err()
                .expect("rejected role");
            assert!(error == Unverified::Argument);
            assert!(!format!("{error:?}").contains(sentinel));
        }
        for remove in [2usize, 6, 8] {
            let mut args = arguments();
            args.remove(remove);
            args.remove(remove - 1);
            assert!(parse_launch(&kernel(&args, &env), Path::new(CLI)).is_err());
        }
    }

    #[test]
    fn codex_runtime_home_and_originator_are_explicit_unique_and_never_guessed() {
        for value in [
            b"CODEX_HOME=".as_slice(),
            b"CODEX_HOME=relative",
            b"CODEX_HOME=\xff",
            b"HOME=",
            b"HOME=relative",
            b"CODEX_CLI_PATH=/tmp/custom",
            b"CODEX_INTERNAL_ORIGINATOR_OVERRIDE=codex_latex_preview",
        ] {
            let mut env = environment();
            let key = value.split(|byte| *byte == b'=').next().unwrap();
            env.retain(|entry| !entry.starts_with(&[key, b"="].concat()));
            env.push(value.to_vec());
            assert!(parse_launch(&kernel(&arguments(), &env), Path::new(CLI)).is_err());
        }
        for key in [
            b"HOME=".as_slice(),
            b"CODEX_CLI_PATH=",
            b"CODEX_INTERNAL_ORIGINATOR_OVERRIDE=",
        ] {
            let mut env = environment();
            env.retain(|item| !item.starts_with(key));
            assert!(parse_launch(&kernel(&arguments(), &env), Path::new(CLI)).is_err());
        }
        let mut env = environment();
        env.push(format!("HOME={HOME}").into_bytes());
        assert!(parse_launch(&kernel(&arguments(), &env), Path::new(CLI)).is_err());
    }

    #[test]
    fn codex_runtime_capture_binds_real_role_entities_and_only_relevant_connection_state() {
        let (target, mut capture) = fixture();
        let first = decide(&target, &capture)
            .ok()
            .expect("complete synthetic evidence");
        assert!(first.cli_version == SUPPORTED_CLI_VERSION && first.revision.len() == 64);
        capture.backends[0]
            .stdio_after
            .desktop
            .push(endpoint(99, 900, 901));
        let unrelated = decide(&target, &capture)
            .ok()
            .expect("unrelated fd is not the connection");
        assert!(first == unrelated);
        capture.backends[0].process.before.started.1 += 1;
        capture.backends[0].process.after.started.1 += 1;
        let restarted = decide(&target, &capture)
            .ok()
            .expect("another coherent generation");
        assert!(first != restarted);
    }

    #[test]
    fn codex_runtime_capture_rejects_standalone_reparented_reused_or_replaced_processes() {
        for change in 0..9 {
            let (target, mut capture) = fixture();
            let child = &mut capture.backends[0].process;
            match change {
                0 => {
                    child.before.parent = 1;
                    child.after.parent = 1;
                }
                1 => child.after.started.1 += 1,
                2 => {
                    child.before.uid = 502;
                    child.after.uid = 502;
                }
                3 => child.image.after.inode += 1,
                4 => child.image.running_matches_disk = false,
                5 => child.image.signature_valid = false,
                6 => child.image.sha256 = "b".repeat(64),
                7 => child.image.cdhash[0] ^= 1,
                8 => capture.desktop.after.started.1 += 1,
                _ => unreachable!(),
            }
            assert!(decide(&target, &capture).is_err());
        }
    }

    #[test]
    fn codex_runtime_stdio_requires_unique_reciprocal_unix_socketpairs() {
        for change in 0..6 {
            let (target, mut capture) = fixture();
            let connection = &mut capture.backends[0].stdio_after;
            match change {
                0 => {
                    connection.desktop.pop();
                }
                1 => connection.desktop.push(endpoint(52, 10, 20)),
                2 => connection.stdin.unix_stream = false,
                3 => connection.stdout.connected = false,
                4 => connection.stdout.peer = 777,
                5 => {
                    connection.desktop[0].fd = 70;
                }
                _ => unreachable!(),
            }
            assert!(decide(&target, &capture).err() == Some(Unverified::Connection));
        }
    }

    #[test]
    fn codex_runtime_profile_ambiguity_and_root_changes_fail_closed() {
        for change in 0..7 {
            let (target, mut capture) = fixture();
            match change {
                0 => capture.materials[0].sha256 = "0".repeat(64),
                1 => capture.materials[1].after.inode += 1,
                2 => capture.cli_version = "unsupported".into(),
                3 => capture.backends.clear(),
                4 => capture.backends.push(capture.backends[0].clone()),
                5 => capture.backends[0].root_after.inode += 1,
                6 => {
                    capture.backends[0].root_before.canonical = "/tmp/other".into();
                    capture.backends[0].root_after.canonical = "/tmp/other".into();
                }
                _ => unreachable!(),
            }
            assert!(decide(&target, &capture).is_err());
        }
    }

    #[test]
    fn codex_runtime_launch_read_failures_bounds_and_truncation_are_unknown() {
        assert!(parse_launch(&[], Path::new(CLI)).err() == Some(Unverified::Read));
        assert!(
            parse_launch(&vec![0; MAX_LAUNCH_BYTES + 1], Path::new(CLI)).err()
                == Some(Unverified::Read)
        );
        let raw = kernel(&arguments(), &environment());
        for end in [4, 12, raw.len() / 2, raw.len() - 3, raw.len() - 2] {
            assert!(parse_launch(&raw[..end], Path::new(CLI)).is_err());
        }
        // A complete last known field is not proof that no later CODEX_HOME
        // exists: the kernel buffer must also contain the environment terminator.
        let mut env = environment();
        env.push(b"CODEX_HOME=/tmp/actual-later-root".to_vec());
        let later_root = kernel(&arguments(), &env);
        assert!(parse_launch(&later_root[..raw.len() - 2], Path::new(CLI)).is_err());
        let complete = parse_launch(&later_root, Path::new(CLI))
            .ok()
            .expect("complete later root");
        assert!(complete.requested_root() == Path::new("/tmp/actual-later-root"));
        let mut invalid_padding = kernel(&arguments(), &environment());
        invalid_padding.extend_from_slice(b"CODEX_HOME=/tmp/after-terminator\0");
        assert!(parse_launch(&invalid_padding, Path::new(CLI)).is_err());
        let mut argc = kernel(&arguments(), &environment());
        argc[..4].copy_from_slice(&i32::MAX.to_ne_bytes());
        assert!(parse_launch(&argc, Path::new(CLI)).err() == Some(Unverified::Launch));
    }
}
