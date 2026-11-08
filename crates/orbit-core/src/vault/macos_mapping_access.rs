//! Create-only ACLs for new Claude mapping local tokens. No existing item ACL
//! is changed, and no helper path, service, account or ACL arrives through IPC.
use crate::{
    error::{Result, fail},
    helper_artifact::{MAX_HELPER_BYTES, binary_name},
    identity::sha256,
    updates::exclusive_helper_lock,
};
use core_foundation::{
    array::{
        CFArrayCreate, CFArrayGetCount, CFArrayGetTypeID, CFArrayGetValueAtIndex, CFArrayRef,
        kCFTypeArrayCallBacks,
    },
    base::{CFGetTypeID, CFRelease, CFRetain, CFTypeRef, TCFType, kCFAllocatorDefault},
    data::{CFDataGetBytePtr, CFDataGetLength, CFDataGetTypeID, CFDataRef},
    string::{CFString, CFStringGetTypeID, CFStringRef},
    url::CFURL,
};
use security_framework::os::macos::{
    code_signing::{Flags, SecRequirement, SecStaticCode},
    keychain::{SecKeychain, SecPreferencesDomain},
};
use std::{
    collections::BTreeSet,
    ffi::{CString, c_char, c_void},
    fs::{self, File, OpenOptions},
    os::unix::{
        ffi::OsStrExt,
        fs::{FileExt, MetadataExt, OpenOptionsExt},
    },
    path::{Path, PathBuf},
    ptr,
};

const SERVICE: &str = "io.orivelle.orbit";
const GENERIC_PASSWORD: u32 = u32::from_be_bytes(*b"genp");
const SERVICE_ATTRIBUTE: u32 = u32::from_be_bytes(*b"svce");
const ACCOUNT_ATTRIBUTE: u32 = u32::from_be_bytes(*b"acct");
const ENCRYPT: &str = "ACLAuthorizationEncrypt";
const OWNER: &str = "ACLAuthorizationChangeACL";
const DECRYPT: &str = "ACLAuthorizationDecrypt";
const INTEGRITY: &str = "ACLAuthorizationIntegrity";
const PARTITION: &str = "ACLAuthorizationPartitionID";
const RESTRICTED: [&str; 6] = [
    DECRYPT,
    "ACLAuthorizationDerive",
    "ACLAuthorizationExportClear",
    "ACLAuthorizationExportWrapped",
    "ACLAuthorizationMAC",
    "ACLAuthorizationSign",
];
type SecRef = *mut c_void;

// The cached Rust crates do not expose these legacy, still-public macOS APIs.
// Signatures match the MacOSX15.4 SDK. Retained CF results are owned below.
#[link(name = "Security", kind = "framework")]
unsafe extern "C" {
    fn SecTrustedApplicationCreateFromPath(path: *const c_char, app: *mut SecRef) -> i32;
    fn SecTrustedApplicationCopyData(app: SecRef, data: *mut CFDataRef) -> i32;
    fn SecTrustedApplicationGetTypeID() -> usize;
    fn SecAccessCreate(description: CFStringRef, apps: CFArrayRef, access: *mut SecRef) -> i32;
    fn SecAccessCopyACLList(access: SecRef, acls: *mut CFArrayRef) -> i32;
    fn SecACLCopyContents(
        acl: SecRef,
        apps: *mut CFArrayRef,
        description: *mut CFStringRef,
        prompt: *mut u16,
    ) -> i32;
    fn SecACLCopyAuthorizations(acl: SecRef) -> CFArrayRef;
    fn SecACLSetContents(
        acl: SecRef,
        apps: CFArrayRef,
        description: CFStringRef,
        prompt: u16,
    ) -> i32;
    fn SecACLCreateWithSimpleContents(
        access: SecRef,
        apps: CFArrayRef,
        description: CFStringRef,
        prompt: u16,
        acl: *mut SecRef,
    ) -> i32;
    fn SecACLUpdateAuthorizations(acl: SecRef, authorizations: CFArrayRef) -> i32;
    fn SecCodeCopyDesignatedRequirement(code: SecRef, flags: u32, requirement: *mut SecRef) -> i32;
    fn SecRequirementCopyString(requirement: SecRef, flags: u32, text: *mut CFStringRef) -> i32;
    fn SecKeychainItemCreateFromContent(
        class: u32,
        attrs: *mut AttributeList,
        length: u32,
        data: *const c_void,
        keychain: SecRef,
        access: SecRef,
        item: *mut SecRef,
    ) -> i32;
    fn SecKeychainItemCopyAccess(item: SecRef, access: *mut SecRef) -> i32;
    fn SecKeychainItemCopyContent(
        item: SecRef,
        class: *mut u32,
        attrs: *mut AttributeList,
        length: *mut u32,
        data: *mut *mut c_void,
    ) -> i32;
    fn SecKeychainItemFreeContent(attrs: *mut AttributeList, data: *mut c_void) -> i32;
    fn SecKeychainItemDelete(item: SecRef) -> i32;
}

#[repr(C)]
struct Attribute {
    tag: u32,
    length: u32,
    data: *mut c_void,
}
#[repr(C)]
struct AttributeList {
    count: u32,
    attrs: *mut Attribute,
}

struct Cf(CFTypeRef);
impl Cf {
    fn created(status: i32, value: CFTypeRef) -> Result<Self> {
        let value = (!value.is_null()).then(|| Self(value));
        if status != 0 {
            return Err(fail("vault_unavailable"));
        }
        value.ok_or_else(|| fail("vault_unavailable"))
    }
    fn borrowed(value: CFTypeRef) -> Result<Self> {
        if value.is_null() {
            return Err(fail("vault_unavailable"));
        }
        // SAFETY: the caller holds the parent CF object and passes a CF value.
        unsafe {
            CFRetain(value);
        }
        Ok(Self(value))
    }
    fn sec(&self) -> SecRef {
        self.0.cast_mut()
    }
    fn array(&self) -> CFArrayRef {
        self.0.cast()
    }
}
impl Drop for Cf {
    fn drop(&mut self) {
        unsafe {
            CFRelease(self.0);
        }
    }
}
fn status(value: i32) -> Result<()> {
    if value == 0 {
        Ok(())
    } else {
        Err(fail("vault_unavailable"))
    }
}
fn array(values: &[CFTypeRef]) -> Result<Cf> {
    // CFType callbacks retain each member for the array's lifetime.
    Cf::created(
        0,
        unsafe {
            CFArrayCreate(
                kCFAllocatorDefault,
                values.as_ptr(),
                values.len() as isize,
                &kCFTypeArrayCallBacks,
            )
        }
        .cast(),
    )
}
fn members(value: CFArrayRef, limit: isize) -> Result<Vec<CFTypeRef>> {
    if value.is_null() || unsafe { CFGetTypeID(value.cast()) } != unsafe { CFArrayGetTypeID() } {
        return Err(fail("vault_unavailable"));
    }
    let count = unsafe { CFArrayGetCount(value) };
    if !(0..=limit).contains(&count) {
        return Err(fail("vault_unavailable"));
    }
    (0..count)
        .map(|index| {
            let member = unsafe { CFArrayGetValueAtIndex(value, index) };
            if member.is_null() {
                Err(fail("vault_unavailable"))
            } else {
                Ok(member)
            }
        })
        .collect()
}
fn string(value: CFStringRef) -> Result<String> {
    if value.is_null() || unsafe { CFGetTypeID(value.cast()) } != unsafe { CFStringGetTypeID() } {
        return Err(fail("vault_unavailable"));
    }
    Ok(unsafe { CFString::wrap_under_get_rule(value) }.to_string())
}

struct TrustedApplication {
    object: Cf,
    snapshot: Vec<u8>,
}
impl TrustedApplication {
    fn create(path: Option<&Path>) -> Result<Self> {
        let path = path
            .map(|p| {
                CString::new(p.as_os_str().as_bytes()).map_err(|_| fail("helper_artifact_invalid"))
            })
            .transpose()?;
        let mut object = ptr::null_mut();
        let result = unsafe {
            SecTrustedApplicationCreateFromPath(
                path.as_ref().map_or(ptr::null(), |p| p.as_ptr()),
                &mut object,
            )
        };
        let object = Cf::created(result, object.cast())?;
        let snapshot = application_snapshot(object.0)?;
        Ok(Self { object, snapshot })
    }
}
fn application_snapshot(object: CFTypeRef) -> Result<Vec<u8>> {
    if object.is_null()
        || unsafe { CFGetTypeID(object) } != unsafe { SecTrustedApplicationGetTypeID() }
    {
        return Err(fail("vault_unavailable"));
    }
    let mut data = ptr::null();
    let result = unsafe { SecTrustedApplicationCopyData(object.cast_mut(), &mut data) };
    let data = Cf::created(result, data.cast())?;
    if unsafe { CFGetTypeID(data.0) } != unsafe { CFDataGetTypeID() } {
        return Err(fail("vault_unavailable"));
    }
    let length = unsafe { CFDataGetLength(data.0.cast()) };
    if !(1..=64 * 1024).contains(&length) {
        return Err(fail("vault_unavailable"));
    }
    let bytes = unsafe { CFDataGetBytePtr(data.0.cast()) };
    if bytes.is_null() {
        return Err(fail("vault_unavailable"));
    }
    // Opaque application data is compared only, never interpreted as a code
    // requirement or exposed as proof of an actual Keychain process decision.
    Ok(unsafe { std::slice::from_raw_parts(bytes, length as usize) }.to_vec())
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct AclShape {
    authorizations: BTreeSet<String>,
    apps: Option<Vec<Vec<u8>>>,
    prompt: u16,
}
struct Acl {
    object: Cf,
    description: Cf,
    description_text: String,
    shape: AclShape,
}
fn acls(access: &Cf) -> Result<Vec<Acl>> {
    let mut list = ptr::null();
    let result = unsafe { SecAccessCopyACLList(access.sec(), &mut list) };
    let list = Cf::created(result, list.cast())?;
    members(list.array(), 8)?
        .into_iter()
        .map(|object| {
            let object = Cf::borrowed(object)?;
            let mut apps = ptr::null();
            let mut description = ptr::null();
            let mut prompt = 0;
            let result = unsafe {
                SecACLCopyContents(object.sec(), &mut apps, &mut description, &mut prompt)
            };
            let applications = (!apps.is_null()).then(|| Cf(apps.cast()));
            let description = Cf::created(result, description.cast())?;
            let description_text = string(description.0.cast())?;
            let apps = applications
                .as_ref()
                .map(|list| {
                    members(list.array(), 2)?
                        .into_iter()
                        .map(application_snapshot)
                        .collect::<Result<Vec<_>>>()
                })
                .transpose()?;
            let authorizations =
                Cf::created(0, unsafe { SecACLCopyAuthorizations(object.sec()) }.cast())?;
            let values = members(authorizations.array(), 8)?
                .into_iter()
                .map(|v| string(v.cast()))
                .collect::<Result<Vec<_>>>()?;
            let authorizations: BTreeSet<_> = values.iter().cloned().collect();
            if authorizations.len() != values.len() {
                return Err(fail("vault_unavailable"));
            }
            Ok(Acl {
                object,
                description,
                description_text,
                shape: AclShape {
                    authorizations,
                    apps,
                    prompt,
                },
            })
        })
        .collect()
}
fn tags(values: &[&str]) -> BTreeSet<String> {
    values.iter().map(|v| (*v).into()).collect()
}
fn shape(auth: &[&str], apps: Option<Vec<Vec<u8>>>) -> AclShape {
    AclShape {
        authorizations: tags(auth),
        apps,
        prompt: 0,
    }
}
fn verify_shapes(actual: &[AclShape], expected: &[AclShape]) -> Result<()> {
    if actual.len() != expected.len()
        || expected
            .iter()
            .any(|value| actual.iter().filter(|entry| *entry == value).count() != 1)
    {
        return Err(fail("vault_unavailable"));
    }
    Ok(())
}
fn verify_persisted_shapes(
    actual: impl IntoIterator<Item = (AclShape, String)>,
    expected: &[AclShape],
) -> Result<()> {
    let mut permissions = Vec::new();
    let mut integrity = false;
    let mut partition = false;
    for (shape, description) in actual {
        let singleton = shape
            .authorizations
            .first()
            .filter(|_| shape.authorizations.len() == 1)
            .map(String::as_str);
        let length = description.len();
        let (seen, valid_length) = match singleton {
            Some(INTEGRITY) => (&mut integrity, length == 64),
            Some(PARTITION) => (
                &mut partition,
                (2..=64 * 1024).contains(&length) && length % 2 == 0,
            ),
            _ => {
                permissions.push(shape);
                continue;
            }
        };
        // Protected keychains add both records on persistence. They carry system
        // integrity and partition data, separate from the four app ACLs.
        if *seen
            || shape.apps.is_some()
            || shape.prompt != 0
            || !valid_length
            || !description.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(fail("vault_unavailable"));
        }
        *seen = true;
    }
    if integrity != partition {
        return Err(fail("vault_unavailable"));
    }
    verify_shapes(&permissions, expected)
}
struct MappingAccess {
    object: Cf,
    expected: Vec<AclShape>,
}
impl MappingAccess {
    fn create(writer: &TrustedApplication, reader: &TrustedApplication) -> Result<Self> {
        if writer.snapshot == reader.snapshot {
            return Err(fail("vault_unavailable"));
        }
        let trusted = array(&[writer.object.0, reader.object.0])?;
        let description = CFString::new("Orivelle Orbit Claude mapping local token");
        let mut object = ptr::null_mut();
        let result = unsafe {
            SecAccessCreate(
                description.as_concrete_TypeRef(),
                trusted.array(),
                &mut object,
            )
        };
        let object = Cf::created(result, object.cast())?;
        let initial = acls(&object)?;
        let writer_only = Some(vec![writer.snapshot.clone()]);
        let reader_only = Some(vec![reader.snapshot.clone()]);
        let initial_expected = vec![
            shape(&[ENCRYPT], None),
            shape(&[OWNER], Some(vec![])),
            shape(
                &RESTRICTED,
                Some(vec![writer.snapshot.clone(), reader.snapshot.clone()]),
            ),
        ];
        // Only the already observed, documented three-entry shape is accepted.
        // Any, unknown authorizations or unexpected wildcard entries fail here.
        verify_shapes(
            &initial.iter().map(|a| a.shape.clone()).collect::<Vec<_>>(),
            &initial_expected,
        )?;
        let self_apps = array(&[writer.object.0])?;
        for acl in &initial {
            if acl.shape.authorizations != tags(&[OWNER]) {
                status(unsafe {
                    SecACLSetContents(
                        acl.object.sec(),
                        self_apps.array(),
                        acl.description.0.cast(),
                        acl.shape.prompt,
                    )
                })?;
            }
        }
        let reader_apps = array(&[reader.object.0])?;
        let mut helper_acl = ptr::null_mut();
        let result = unsafe {
            SecACLCreateWithSimpleContents(
                object.sec(),
                reader_apps.array(),
                description.as_concrete_TypeRef(),
                0,
                &mut helper_acl,
            )
        };
        let helper_acl = Cf::created(result, helper_acl.cast())?;
        let decrypt = CFString::new(DECRYPT);
        let decrypt = array(&[decrypt.as_CFTypeRef()])?;
        status(unsafe { SecACLUpdateAuthorizations(helper_acl.sec(), decrypt.array()) })?;
        let access = Self {
            object,
            expected: vec![
                shape(&[ENCRYPT], writer_only.clone()),
                shape(&[OWNER], Some(vec![])),
                shape(&RESTRICTED, writer_only),
                shape(&[DECRYPT], reader_only),
            ],
        };
        access.verify(&access.object)?;
        Ok(access)
    }
    fn verify(&self, actual: &Cf) -> Result<()> {
        verify_shapes(
            &acls(actual)?
                .into_iter()
                .map(|a| a.shape)
                .collect::<Vec<_>>(),
            &self.expected,
        )
    }
    fn verify_persisted(&self, actual: &Cf) -> Result<()> {
        verify_persisted_shapes(
            acls(actual)?
                .into_iter()
                .map(|acl| (acl.shape, acl.description_text)),
            &self.expected,
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FileIdentity {
    dev: u64,
    ino: u64,
    len: u64,
    uid: u32,
    mode: u32,
    links: u64,
    mtime: i64,
    mtime_nsec: i64,
    ctime: i64,
    ctime_nsec: i64,
}
impl FileIdentity {
    fn from(metadata: &fs::Metadata) -> Self {
        Self {
            dev: metadata.dev(),
            ino: metadata.ino(),
            len: metadata.len(),
            uid: metadata.uid(),
            mode: metadata.mode(),
            links: metadata.nlink(),
            mtime: metadata.mtime(),
            mtime_nsec: metadata.mtime_nsec(),
            ctime: metadata.ctime(),
            ctime_nsec: metadata.ctime_nsec(),
        }
    }
}
fn private_directory(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path).map_err(|_| fail("helper_artifact_invalid"))?;
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o077 != 0
    {
        return Err(fail("helper_artifact_invalid"));
    }
    Ok(())
}
struct HelperFile {
    root: PathBuf,
    path: PathBuf,
    file: File,
    identity: FileIdentity,
    expected_sha256: String,
}
impl HelperFile {
    fn open(root: &Path, expected_sha256: &str) -> Result<Self> {
        if !root.is_absolute()
            || fs::canonicalize(root).map_err(|_| fail("helper_artifact_invalid"))? != root
            || expected_sha256.len() != 64
            || !expected_sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(fail("helper_artifact_invalid"));
        }
        private_directory(root)?;
        private_directory(&root.join("bin"))?;
        let path = root.join("bin").join(binary_name());
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&path)
            .map_err(|_| fail("helper_artifact_invalid"))?;
        let metadata = file
            .metadata()
            .map_err(|_| fail("helper_artifact_invalid"))?;
        if !metadata.is_file()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o077 != 0
            || metadata.mode() & 0o100 == 0
            || metadata.nlink() != 1
            || !(32..=MAX_HELPER_BYTES).contains(&metadata.len())
        {
            return Err(fail("helper_artifact_invalid"));
        }
        let helper = Self {
            root: root.into(),
            path,
            file,
            identity: FileIdentity::from(&metadata),
            expected_sha256: expected_sha256.into(),
        };
        helper.verify()?;
        Ok(helper)
    }
    fn verify(&self) -> Result<()> {
        private_directory(&self.root)?;
        private_directory(&self.root.join("bin"))?;
        let path = fs::symlink_metadata(&self.path).map_err(|_| fail("helper_artifact_invalid"))?;
        let before = self
            .file
            .metadata()
            .map_err(|_| fail("helper_artifact_invalid"))?;
        if path.file_type().is_symlink()
            || FileIdentity::from(&path) != self.identity
            || FileIdentity::from(&before) != self.identity
        {
            return Err(fail("helper_artifact_invalid"));
        }
        let mut bytes = vec![0; self.identity.len as usize];
        self.file
            .read_exact_at(&mut bytes, 0)
            .map_err(|_| fail("helper_artifact_invalid"))?;
        if sha256(&bytes) != self.expected_sha256
            || FileIdentity::from(
                &self
                    .file
                    .metadata()
                    .map_err(|_| fail("helper_artifact_invalid"))?,
            ) != self.identity
            || FileIdentity::from(
                &fs::symlink_metadata(&self.path).map_err(|_| fail("helper_artifact_invalid"))?,
            ) != self.identity
        {
            return Err(fail("helper_artifact_invalid"));
        }
        Ok(())
    }
    fn trusted_application(&self) -> Result<TrustedApplication> {
        self.verify()?;
        let url =
            CFURL::from_path(&self.path, false).ok_or_else(|| fail("helper_artifact_invalid"))?;
        let code = SecStaticCode::from_path(&url, Flags::NONE)
            .map_err(|_| fail("helper_artifact_invalid"))?;
        let mut requirement = ptr::null_mut();
        let result = unsafe {
            SecCodeCopyDesignatedRequirement(code.as_concrete_TypeRef().cast(), 0, &mut requirement)
        };
        let requirement =
            Cf::created(result, requirement.cast()).map_err(|_| fail("helper_artifact_invalid"))?;
        let mut text = ptr::null();
        let result = unsafe { SecRequirementCopyString(requirement.sec(), 0, &mut text) };
        let text = Cf::created(result, text.cast()).map_err(|_| fail("helper_artifact_invalid"))?;
        let text = string(text.0.cast()).map_err(|_| fail("helper_artifact_invalid"))?;
        // This feature deliberately does not accept a broad team/identifier DR
        // or assume that an ad-hoc helper upgrade inherits old ACL permission.
        exact_cdhash_requirement(&text)?;
        let requirement: SecRequirement =
            text.parse().map_err(|_| fail("helper_artifact_invalid"))?;
        code.check_validity(Flags::NONE, &requirement)
            .map_err(|_| fail("helper_artifact_invalid"))?;
        self.verify()?;
        let application = TrustedApplication::create(Some(&self.path))?;
        self.verify()?;
        Ok(application)
    }
}
fn exact_cdhash_requirement(text: &str) -> Result<()> {
    let hash = text
        .strip_prefix("cdhash H\"")
        .and_then(|text| text.strip_suffix('"'))
        .ok_or_else(|| fail("helper_artifact_invalid"))?;
    if !matches!(hash.len(), 40 | 64) || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(fail("helper_artifact_invalid"));
    }
    Ok(())
}

struct ItemScope {
    account: String,
}
impl ItemScope {
    fn new(binding_id: &str, value: &[u8]) -> Result<Self> {
        let id = uuid::Uuid::parse_str(binding_id).map_err(|_| fail("invalid_identifier"))?;
        if id.to_string() != binding_id {
            return Err(fail("invalid_identifier"));
        }
        if value.is_empty() || value.len() > 64 * 1024 {
            return Err(fail("invalid_secret"));
        }
        Ok(Self {
            account: format!("claude-map/{binding_id}"),
        })
    }
}
trait NewItemBackend {
    type Item;
    fn create(&mut self, scope: &ItemScope, value: &[u8]) -> Result<Self::Item>;
    fn verify(&mut self, item: &Self::Item, scope: &ItemScope) -> Result<()>;
    fn delete_new(&mut self, item: &Self::Item) -> Result<()>;
}
fn create_checked<B: NewItemBackend>(
    backend: &mut B,
    scope: &ItemScope,
    value: &[u8],
    mut identity_guard: impl FnMut() -> Result<()>,
) -> Result<()> {
    identity_guard()?;
    let item = backend.create(scope, value)?;
    let verified = (|| {
        identity_guard()?;
        backend.verify(&item, scope)?;
        identity_guard()
    })();
    if let Err(error) = verified {
        // Only the object returned by successful creation is eligible for this
        // cleanup. No lookup, generic put, update or existing-item ACL fallback.
        backend.delete_new(&item)?;
        return Err(error);
    }
    Ok(())
}
struct NativeItems<'a> {
    access: &'a MappingAccess,
}
impl NewItemBackend for NativeItems<'_> {
    type Item = Cf;
    fn create(&mut self, scope: &ItemScope, value: &[u8]) -> Result<Cf> {
        // Match apple-native-keyring-store's explicit User-domain keychain,
        // rather than assuming NULL/default selects that same keychain.
        let keychain = SecKeychain::default_for_domain(SecPreferencesDomain::User)
            .map_err(|_| fail("vault_unavailable"))?;
        let mut attrs = [
            Attribute {
                tag: SERVICE_ATTRIBUTE,
                length: SERVICE.len() as u32,
                data: SERVICE.as_ptr().cast_mut().cast(),
            },
            Attribute {
                tag: ACCOUNT_ATTRIBUTE,
                length: scope.account.len() as u32,
                data: scope.account.as_ptr().cast_mut().cast(),
            },
        ];
        let mut attrs = AttributeList {
            count: 2,
            attrs: attrs.as_mut_ptr(),
        };
        let mut item = ptr::null_mut();
        let result = unsafe {
            SecKeychainItemCreateFromContent(
                GENERIC_PASSWORD,
                &mut attrs,
                value.len() as u32,
                value.as_ptr().cast(),
                keychain.as_concrete_TypeRef().cast(),
                self.access.object.sec(),
                &mut item,
            )
        };
        // In particular, errSecDuplicateItem is an error, never an update.
        Cf::created(result, item.cast())
    }
    fn verify(&mut self, item: &Cf, scope: &ItemScope) -> Result<()> {
        verify_item_scope(item, scope)?;
        let mut access = ptr::null_mut();
        let result = unsafe { SecKeychainItemCopyAccess(item.sec(), &mut access) };
        let access = Cf::created(result, access.cast())?;
        self.access.verify_persisted(&access)
    }
    fn delete_new(&mut self, item: &Cf) -> Result<()> {
        status(unsafe { SecKeychainItemDelete(item.sec()) })
    }
}
fn verify_item_scope(item: &Cf, scope: &ItemScope) -> Result<()> {
    let mut values = [
        Attribute {
            tag: SERVICE_ATTRIBUTE,
            length: 0,
            data: ptr::null_mut(),
        },
        Attribute {
            tag: ACCOUNT_ATTRIBUTE,
            length: 0,
            data: ptr::null_mut(),
        },
    ];
    let mut attrs = AttributeList {
        count: 2,
        attrs: values.as_mut_ptr(),
    };
    let mut class = 0;
    // Request attributes only. Neither the secret nor any existing item is read.
    let result = unsafe {
        SecKeychainItemCopyContent(
            item.sec(),
            &mut class,
            &mut attrs,
            ptr::null_mut(),
            ptr::null_mut(),
        )
    };
    let verified = (|| {
        status(result)?;
        if class != GENERIC_PASSWORD || attrs.count != 2 || attrs.attrs != values.as_mut_ptr() {
            return Err(fail("vault_unavailable"));
        }
        for (attribute, tag, expected) in [
            (&values[0], SERVICE_ATTRIBUTE, SERVICE.as_bytes()),
            (&values[1], ACCOUNT_ATTRIBUTE, scope.account.as_bytes()),
        ] {
            if attribute.tag != tag
                || attribute.length as usize != expected.len()
                || attribute.data.is_null()
            {
                return Err(fail("vault_unavailable"));
            }
            if unsafe {
                std::slice::from_raw_parts(attribute.data.cast::<u8>(), attribute.length as usize)
            } != expected
            {
                return Err(fail("vault_unavailable"));
            }
        }
        Ok(())
    })();
    let freed = status(unsafe { SecKeychainItemFreeContent(&mut attrs, ptr::null_mut()) });
    verified.and(freed)
}

pub(super) struct MappingSecretCreator {
    root: PathBuf,
    helper_sha256: String,
}
impl MappingSecretCreator {
    pub(super) fn new(root: PathBuf, helper_sha256: String) -> Self {
        Self {
            root,
            helper_sha256,
        }
    }
    pub(super) fn create(&self, binding_id: &str, value: &[u8]) -> Result<()> {
        let scope = ItemScope::new(binding_id, value)?;
        private_directory(&self.root)?;
        private_directory(&self.root.join("locks"))?;
        let lock_path = self.root.join("locks/helper-update.lock");
        let metadata =
            fs::symlink_metadata(&lock_path).map_err(|_| fail("helper_artifact_invalid"))?;
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o077 != 0
        {
            return Err(fail("helper_artifact_invalid"));
        }
        let lock = exclusive_helper_lock(&self.root)?;
        if FileIdentity::from(
            &lock
                .metadata()
                .map_err(|_| fail("helper_artifact_invalid"))?,
        ) != FileIdentity::from(
            &fs::symlink_metadata(&lock_path).map_err(|_| fail("helper_artifact_invalid"))?,
        ) {
            return Err(fail("helper_artifact_invalid"));
        }
        let helper = HelperFile::open(&self.root, &self.helper_sha256)?;
        let reader = helper.trusted_application()?;
        let writer = TrustedApplication::create(None)?;
        let access = MappingAccess::create(&writer, &reader)?;
        // Cooperative deployment is excluded until creation and readback end.
        // The public path-based OS APIs still cannot make this atomic against
        // a non-cooperating same-UID attacker; this is not an updater identity
        // continuity contract or proof of a real helper-read decision.
        create_checked(&mut NativeItems { access: &access }, &scope, value, || {
            helper.verify()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};

    #[test]
    fn claude_mapping_access_memory_acl_has_only_self_and_helper_decrypt() {
        // Memory-only access objects: this test has no NativeItems or Keychain
        // item calls. /usr/bin/true is an identity fixture and is not executed.
        let writer = TrustedApplication::create(None).unwrap();
        let reader = TrustedApplication::create(Some(Path::new("/usr/bin/true"))).unwrap();
        let access = MappingAccess::create(&writer, &reader).unwrap();
        let actual = acls(&access.object)
            .unwrap()
            .into_iter()
            .map(|a| a.shape)
            .collect::<Vec<_>>();
        assert_eq!(actual.len(), 4);
        assert!(actual.iter().all(|a| a.apps.is_some()));
        assert_eq!(
            actual
                .iter()
                .filter(|a| a.apps == Some(vec![reader.snapshot.clone()]))
                .count(),
            1
        );
        assert!(actual.iter().any(|a| a.authorizations == tags(&[DECRYPT])
            && a.apps == Some(vec![reader.snapshot.clone()])));
        assert!(actual.iter().any(|a| a.authorizations == tags(&[OWNER])
            && a.apps == Some(vec![])
            && a.prompt == 0));
        for fault in ["wildcard", "any", "helper_sign", "owner", "duplicate"] {
            let mut changed = actual.clone();
            let decrypt = changed
                .iter()
                .position(|a| a.authorizations == tags(&[DECRYPT]))
                .unwrap();
            match fault {
                "wildcard" => changed[decrypt].apps = None,
                "any" => {
                    changed[decrypt]
                        .authorizations
                        .insert("ACLAuthorizationAny".into());
                }
                "helper_sign" => {
                    changed[decrypt]
                        .authorizations
                        .insert("ACLAuthorizationSign".into());
                }
                "owner" => {
                    changed
                        .iter_mut()
                        .find(|a| a.authorizations == tags(&[OWNER]))
                        .unwrap()
                        .apps = Some(vec![reader.snapshot.clone()]);
                }
                "duplicate" => changed.push(changed[decrypt].clone()),
                _ => unreachable!(),
            }
            assert!(
                verify_shapes(&changed, &access.expected).is_err(),
                "{fault}"
            );
        }
    }

    #[test]
    fn claude_mapping_access_readback_accepts_only_exact_system_metadata_pair() {
        let writer = Some(vec![b"writer".to_vec()]);
        let reader = Some(vec![b"reader".to_vec()]);
        let expected = vec![
            shape(&[ENCRYPT], writer.clone()),
            shape(&[OWNER], Some(vec![])),
            shape(&RESTRICTED, writer),
            shape(&[DECRYPT], reader),
        ];
        let legacy: Vec<_> = expected
            .iter()
            .cloned()
            .map(|shape| (shape, "Orbit mapping token".to_owned()))
            .collect();
        verify_persisted_shapes(legacy.clone(), &expected).unwrap();
        let integrity = (shape(&[INTEGRITY], None), "a5".repeat(32));
        let partition = (shape(&[PARTITION], None), "a5".repeat(297));
        let mut protected = legacy.clone();
        protected.extend([integrity.clone(), partition.clone()]);
        verify_persisted_shapes(protected.clone(), &expected).unwrap();
        assert!(verify_persisted_shapes(protected[..5].to_vec(), &expected).is_err());
        let mut changed_reader = protected.clone();
        changed_reader[3].0.apps = None;
        assert!(verify_persisted_shapes(changed_reader, &expected).is_err());

        for extra in [
            integrity,
            partition,
            (shape(&["ACLAuthorizationAny"], None), "00".into()),
        ] {
            let mut changed = protected.clone();
            changed.push(extra);
            assert!(verify_persisted_shapes(changed, &expected).is_err());
        }
        for invalid in [
            (shape(&[PARTITION, DECRYPT], None), "00".into()),
            (shape(&[PARTITION], Some(vec![])), "00".into()),
            (shape(&[PARTITION], None), "0x".into()),
            (shape(&[PARTITION], None), "123".into()),
        ] {
            let mut changed = protected.clone();
            changed[5] = invalid;
            assert!(verify_persisted_shapes(changed, &expected).is_err());
        }
    }

    fn helper_fixture() -> (tempfile::TempDir, PathBuf, String) {
        let dir = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(dir.path()).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        for child in ["bin", "locks"] {
            fs::create_dir(root.join(child)).unwrap();
            fs::set_permissions(root.join(child), fs::Permissions::from_mode(0o700)).unwrap();
        }
        fs::write(root.join("locks/helper-update.lock"), b"").unwrap();
        fs::set_permissions(
            root.join("locks/helper-update.lock"),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        let bytes = b"project-only helper file identity fixture, never executed";
        fs::write(root.join("bin").join(binary_name()), bytes).unwrap();
        fs::set_permissions(
            root.join("bin").join(binary_name()),
            fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        (dir, root, sha256(bytes))
    }

    #[test]
    fn claude_mapping_access_fixed_helper_rejects_replacement_hash_symlink_and_lock_conflict() {
        let (_dir, root, hash) = helper_fixture();
        let helper = HelperFile::open(&root, &hash).unwrap();
        helper.verify().unwrap();
        assert!(HelperFile::open(&root, &"0".repeat(64)).is_err());
        let path = root.join("bin").join(binary_name());
        let replacement = root.join("bin/replacement");
        fs::copy(&path, &replacement).unwrap();
        fs::rename(&replacement, &path).unwrap();
        assert!(
            helper.verify().is_err(),
            "even identical replacement bytes change frozen FD identity"
        );
        let helper = HelperFile::open(&root, &hash).unwrap();
        fs::write(&path, b"different bytes at the same reviewed helper path").unwrap();
        assert!(helper.verify().is_err());
        let (_other, safe_root, hash) = helper_fixture();
        let safe_path = safe_root.join("bin").join(binary_name());
        fs::set_permissions(&safe_path, fs::Permissions::from_mode(0o744)).unwrap();
        assert!(HelperFile::open(&safe_root, &hash).is_err());
        fs::set_permissions(&safe_path, fs::Permissions::from_mode(0o700)).unwrap();
        let renamed = safe_root.join("bin/original");
        fs::rename(&safe_path, &renamed).unwrap();
        symlink(&renamed, &safe_path).unwrap();
        assert!(HelperFile::open(&safe_root, &hash).is_err());
        let held = exclusive_helper_lock(&safe_root).unwrap();
        let creator = MappingSecretCreator::new(safe_root, hash);
        assert_eq!(
            creator
                .create(
                    "7317cf98-8437-4666-aff6-2aac3d1e9b6e",
                    b"isolated local token"
                )
                .unwrap_err()
                .code,
            "update_busy"
        );
        drop(held);
        exact_cdhash_requirement("cdhash H\"08f8fb17f997590edae5df3cd16d79d07e7986d6\"").unwrap();
        for bad in [
            "identifier helper",
            "anchor apple",
            "cdhash H\"aa\"",
            "cdhash H\"08f8fb17f997590edae5df3cd16d79d07e7986d6\" or true",
        ] {
            assert!(exact_cdhash_requirement(bad).is_err());
        }
    }

    #[derive(Default)]
    struct FakeItems {
        events: Vec<&'static str>,
        create_error: bool,
        verify_error: bool,
        item_ids_deleted: Vec<u64>,
    }
    impl NewItemBackend for FakeItems {
        type Item = u64;
        fn create(&mut self, scope: &ItemScope, value: &[u8]) -> Result<u64> {
            assert_eq!(
                scope.account,
                "claude-map/7317cf98-8437-4666-aff6-2aac3d1e9b6e"
            );
            assert_eq!(value, b"isolated local token");
            self.events.push("create");
            if self.create_error {
                Err(fail("vault_unavailable"))
            } else {
                Ok(17)
            }
        }
        fn verify(&mut self, item: &u64, _: &ItemScope) -> Result<()> {
            assert_eq!(*item, 17);
            self.events.push("verify");
            if self.verify_error {
                Err(fail("vault_unavailable"))
            } else {
                Ok(())
            }
        }
        fn delete_new(&mut self, item: &u64) -> Result<()> {
            self.events.push("delete_new");
            self.item_ids_deleted.push(*item);
            Ok(())
        }
    }

    #[test]
    fn claude_mapping_access_create_only_failures_never_update_or_delete_an_existing_item() {
        let value = b"isolated local token";
        let scope = ItemScope::new("7317cf98-8437-4666-aff6-2aac3d1e9b6e", value).unwrap();
        for id in [
            "CLAUDE-MAP/7317cf98-8437-4666-aff6-2aac3d1e9b6e",
            "7317CF98-8437-4666-AFF6-2AAC3D1E9B6E",
            "backup/key",
            "",
        ] {
            assert!(ItemScope::new(id, value).is_err());
        }
        assert!(ItemScope::new("7317cf98-8437-4666-aff6-2aac3d1e9b6e", b"").is_err());
        assert!(
            ItemScope::new("7317cf98-8437-4666-aff6-2aac3d1e9b6e", &vec![0; 65 * 1024]).is_err()
        );
        for fault in [
            "success",
            "denied_or_duplicate",
            "readback",
            "identity_before",
            "identity_after_create",
            "identity_after_readback",
        ] {
            let mut backend = FakeItems {
                create_error: fault == "denied_or_duplicate",
                verify_error: fault == "readback",
                ..Default::default()
            };
            let mut guards = 0;
            let result = create_checked(&mut backend, &scope, value, || {
                guards += 1;
                if (fault == "identity_before" && guards == 1)
                    || (fault == "identity_after_create" && guards == 2)
                    || (fault == "identity_after_readback" && guards == 3)
                {
                    Err(fail("helper_artifact_invalid"))
                } else {
                    Ok(())
                }
            });
            assert_eq!(result.is_ok(), fault == "success", "{fault}");
            let expected = match fault {
                "success" => vec!["create", "verify"],
                "denied_or_duplicate" => vec!["create"],
                "readback" | "identity_after_readback" => vec!["create", "verify", "delete_new"],
                "identity_before" => vec![],
                "identity_after_create" => vec!["create", "delete_new"],
                _ => unreachable!(),
            };
            assert_eq!(backend.events, expected, "{fault}");
            assert_eq!(
                backend.item_ids_deleted,
                if expected.contains(&"delete_new") {
                    vec![17]
                } else {
                    vec![]
                }
            );
        }
        // This reader-only instance fails before any native item API.
        use crate::vault::{NativeVault, Vault};
        assert_eq!(
            NativeVault::default()
                .create_claude_mapping_secret("7317cf98-8437-4666-aff6-2aac3d1e9b6e", value)
                .unwrap_err()
                .code,
            "vault_unavailable"
        );
    }
}
