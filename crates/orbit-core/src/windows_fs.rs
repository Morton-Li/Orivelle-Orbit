//! Handle-based Windows configuration scope. Reparse points are rejected and
//! directory handles deny delete-sharing while a path is being published.
use crate::error::{Result, fail};
use serde::{Deserialize, Serialize};
use std::{
    ffi::c_void,
    fs::{File, OpenOptions},
    os::windows::{
        ffi::OsStrExt,
        fs::{MetadataExt, OpenOptionsExt},
        io::AsRawHandle,
    },
    path::{Component, Path},
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, LocalFree},
    Security::{Authorization::*, *},
    Storage::FileSystem::*,
    System::Threading::{GetCurrentProcess, OpenProcessToken},
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Identity {
    volume: u64,
    file_id: [u8; 16],
    owner_sid: String,
}
fn unverified() -> crate::error::PublicError {
    fail("codex_configuration_target_unverified")
}
fn wide(value: &std::ffi::OsStr) -> Vec<u16> {
    value.encode_wide().chain(Some(0)).collect()
}
struct Allocation(*mut c_void);
impl Drop for Allocation {
    fn drop(&mut self) {
        unsafe {
            LocalFree(self.0);
        }
    }
}
fn sid_string(sid: PSID) -> Result<String> {
    let mut text = null_mut();
    if sid.is_null()
        || unsafe { IsValidSid(sid) } == 0
        || unsafe { ConvertSidToStringSidW(sid, &mut text) } == 0
    {
        return Err(unverified());
    }
    let _allocation = Allocation(text.cast());
    let mut len = 0;
    while len < 256 && unsafe { *text.add(len) } != 0 {
        len += 1;
    }
    if len == 256 {
        return Err(unverified());
    }
    String::from_utf16(unsafe { std::slice::from_raw_parts(text, len) }).map_err(|_| unverified())
}
pub(crate) fn current_sid() -> Result<String> {
    let mut token: HANDLE = null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(unverified());
    }
    struct Token(HANDLE);
    impl Drop for Token {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
    let token = Token(token);
    let mut len = 0;
    unsafe {
        GetTokenInformation(token.0, TokenUser, null_mut(), 0, &mut len);
    }
    if len == 0 || len > 65536 {
        return Err(unverified());
    }
    // usize storage gives TOKEN_USER its required alignment.
    let mut data = vec![0usize; (len as usize).div_ceil(size_of::<usize>())];
    if unsafe { GetTokenInformation(token.0, TokenUser, data.as_mut_ptr().cast(), len, &mut len) }
        == 0
    {
        return Err(unverified());
    }
    sid_string(unsafe { (*(data.as_ptr().cast::<TOKEN_USER>())).User.Sid })
}
fn valid_path(path: &Path) -> bool {
    let mut parts = path.components();
    matches!(parts.next(), Some(Component::Prefix(p)) if matches!(p.kind(), std::path::Prefix::Disk(_) | std::path::Prefix::VerbatimDisk(_)))
        && matches!(parts.next(), Some(Component::RootDir))
        && parts.all(|part| {
            matches!(part, Component::Normal(name) if {
                let s = name.to_string_lossy();
                !s.contains([':', '\0']) && !s.ends_with(['.', ' '])
            })
        })
        && path.as_os_str().encode_wide().count() < 32700
}
fn open_one(path: &Path, directory: bool, access: u32, share_delete: bool) -> Result<File> {
    let file = OpenOptions::new()
        .access_mode(access)
        .share_mode(
            FILE_SHARE_READ | FILE_SHARE_WRITE | if share_delete { FILE_SHARE_DELETE } else { 0 },
        )
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
        .map_err(|_| unverified())?;
    let meta = file.metadata().map_err(|_| unverified())?;
    if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
        || meta.is_dir() != directory
        || (!directory && !meta.is_file())
    {
        return Err(unverified());
    }
    Ok(file)
}
/// Hold each existing ancestor against rename/delete. No UNC, ADS or reparse traversal.
pub(crate) fn lock_directory(path: &Path) -> Result<Vec<File>> {
    if !valid_path(path) {
        return Err(unverified());
    }
    let mut chain = path.ancestors().collect::<Vec<_>>();
    chain.reverse();
    chain
        .into_iter()
        .map(|p| open_one(p, true, FILE_READ_ATTRIBUTES, false))
        .collect()
}
pub(crate) fn directory(path: &Path) -> Result<File> {
    let _parents = lock_directory(path)?;
    open_one(path, true, FILE_READ_ATTRIBUTES | READ_CONTROL, false)
}
pub(crate) fn file(path: &Path) -> Result<File> {
    if !valid_path(path) {
        return Err(unverified());
    }
    let _parents = lock_directory(path.parent().ok_or_else(unverified)?)?;
    open_one(path, false, FILE_GENERIC_READ, true)
}
pub(crate) fn lock_installed_file(path: &Path) -> Result<File> {
    if !valid_path(path) {
        return Err(unverified());
    }
    let file = OpenOptions::new()
        .access_mode(FILE_GENERIC_READ)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .map_err(|_| unverified())?;
    let metadata = file.metadata().map_err(|_| unverified())?;
    if !metadata.is_file() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(unverified());
    }
    Ok(file)
}
pub(crate) fn identity(file: &File, require_private: bool) -> Result<Identity> {
    let mut info: FILE_ID_INFO = unsafe { std::mem::zeroed() };
    if unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileIdInfo,
            (&mut info as *mut FILE_ID_INFO).cast(),
            size_of::<FILE_ID_INFO>() as u32,
        )
    } == 0
    {
        return Err(unverified());
    }
    let (mut owner, mut acl, mut descriptor) = (null_mut(), null_mut(), null_mut());
    if unsafe {
        GetSecurityInfo(
            file.as_raw_handle(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            null_mut(),
            &mut acl,
            null_mut(),
            &mut descriptor,
        )
    } != 0
    {
        return Err(unverified());
    }
    let _descriptor = Allocation(descriptor);
    let owner_sid = sid_string(owner)?;
    if require_private {
        let user = current_sid()?;
        // Windows-created profiles can be owned by SYSTEM; elevated creators
        // can use Administrators. Both are already privileged writers;
        // still require every granting ACE to exclude other users.
        if (owner_sid != user && owner_sid != "S-1-5-32-544" && owner_sid != "S-1-5-18")
            || acl.is_null()
            || unsafe { IsValidAcl(acl) } == 0
        {
            return Err(unverified());
        }
        for index in 0..unsafe { (*acl).AceCount } as u32 {
            let mut raw = null_mut();
            if unsafe { GetAce(acl, index, &mut raw) } == 0 {
                return Err(unverified());
            }
            let header = unsafe { &*raw.cast::<ACE_HEADER>() };
            // Deny/audit ACEs cannot grant writes. Unknown granting ACE forms
            // cannot establish a writable user destination.
            if header.AceType == 1 || header.AceType == 2 {
                continue;
            }
            if header.AceType != 0 || (header.AceSize as usize) < size_of::<ACCESS_ALLOWED_ACE>() {
                return Err(unverified());
            }
            let ace = unsafe { &*raw.cast::<ACCESS_ALLOWED_ACE>() };
            const WRITE: u32 = FILE_WRITE_DATA
                | FILE_APPEND_DATA
                | FILE_WRITE_EA
                | FILE_WRITE_ATTRIBUTES
                | FILE_DELETE_CHILD
                | DELETE
                | WRITE_DAC
                | WRITE_OWNER
                | 0x10000000
                | 0x40000000;
            if ace.Mask & WRITE != 0 {
                let sid = sid_string((&ace.SidStart as *const u32).cast_mut().cast())?;
                if sid != user && sid != "S-1-5-18" && sid != "S-1-5-32-544" {
                    return Err(unverified());
                }
            }
        }
    }
    Ok(Identity {
        volume: info.VolumeSerialNumber,
        file_id: info.FileId.Identifier,
        owner_sid,
    })
}
pub(crate) fn stamp(file: &File) -> Result<String> {
    let mut basic: FILE_BASIC_INFO = unsafe { std::mem::zeroed() };
    if unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileBasicInfo,
            (&mut basic as *mut FILE_BASIC_INFO).cast(),
            size_of::<FILE_BASIC_INFO>() as u32,
        )
    } == 0
    {
        return Err(unverified());
    }
    Ok(format!(
        "{}:{}:{}:{}:{}",
        serde_json::to_string(&identity(file, false)?).map_err(|_| unverified())?,
        file.metadata().map_err(|_| unverified())?.len(),
        basic.LastWriteTime,
        basic.ChangeTime,
        basic.FileAttributes
    ))
}
fn descriptor(directory: bool) -> Result<Allocation> {
    let sid = current_sid()?;
    let inheritance = if directory { "OICI" } else { "" };
    let sddl = format!("O:{sid}D:P(A;{inheritance};FA;;;SY)(A;{inheritance};FA;;;{sid})");
    let mut descriptor = null_mut();
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            wide(std::ffi::OsStr::new(&sddl)).as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            null_mut(),
        )
    } == 0
    {
        return Err(unverified());
    }
    Ok(Allocation(descriptor))
}
/// Create one leaf with its private DACL in the same operation.
pub(crate) fn create_directory(path: &Path) -> Result<()> {
    let _parents = lock_directory(path.parent().ok_or_else(unverified)?)?;
    let descriptor = descriptor(true)?;
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: 0,
    };
    if unsafe { CreateDirectoryW(wide(path.as_os_str()).as_ptr(), &attributes) } == 0 {
        return Err(fail("configuration_conflict"));
    }
    identity(&directory(path)?, true)?;
    Ok(())
}
/// Apply a protected user/SYSTEM DACL through a verified handle, without spawning
/// PowerShell. The owner is set to the current user, including when an elevated
/// creator initially assigned ownership to Administrators.
pub(crate) fn make_private(path: &Path, is_directory: bool) -> Result<()> {
    if !valid_path(path) {
        return Err(unverified());
    }
    let _parents = lock_directory(path.parent().ok_or_else(unverified)?)?;
    let file = open_one(
        path,
        is_directory,
        FILE_READ_ATTRIBUTES | READ_CONTROL | WRITE_DAC | WRITE_OWNER,
        true,
    )?;
    let descriptor = descriptor(is_directory)?;
    let mut owner = null_mut();
    let mut owner_defaulted = 0;
    if unsafe { GetSecurityDescriptorOwner(descriptor.0, &mut owner, &mut owner_defaulted) } == 0
        || owner.is_null()
    {
        return Err(unverified());
    }
    let (mut present, mut defaulted, mut acl) = (0, 0, null_mut());
    if unsafe { GetSecurityDescriptorDacl(descriptor.0, &mut present, &mut acl, &mut defaulted) }
        == 0
        || present == 0
        || acl.is_null()
    {
        return Err(unverified());
    }
    if unsafe {
        SetSecurityInfo(
            file.as_raw_handle(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION
                | DACL_SECURITY_INFORMATION
                | PROTECTED_DACL_SECURITY_INFORMATION,
            owner,
            null_mut(),
            acl,
            null(),
        )
    } != 0
    {
        return Err(unverified());
    }
    identity(&file, true)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    #[test]
    fn private_identity_is_stable_and_replacement_is_distinct() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("user data with spaces");
        create_directory(&root).unwrap();
        let before = identity(&directory(&root).unwrap(), true).unwrap();
        fs::write(root.join("ordinary.json"), b"{}").unwrap();
        assert_eq!(identity(&directory(&root).unwrap(), true).unwrap(), before);
        fs::remove_dir_all(&root).unwrap();
        create_directory(&root).unwrap();
        assert_ne!(identity(&directory(&root).unwrap(), true).unwrap(), before);
    }
    #[test]
    fn junctions_and_unprivileged_write_grants_are_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let outside = temp.path().join("outside");
        create_directory(&outside).unwrap();
        let link = temp.path().join("junction");
        let status = std::process::Command::new("cmd.exe")
            .args(["/d", "/c", "mklink", "/J"])
            .arg(&link)
            .arg(&outside)
            .status()
            .unwrap();
        assert!(status.success());
        assert!(directory(&link).is_err());
        fs::remove_dir(&link).unwrap();
        let status = std::process::Command::new("icacls.exe")
            .arg(&outside)
            .args(["/grant", "*S-1-5-32-545:(OI)(CI)(M)"])
            .status()
            .unwrap();
        assert!(status.success());
        assert!(identity(&directory(&outside).unwrap(), true).is_err());
    }
}
