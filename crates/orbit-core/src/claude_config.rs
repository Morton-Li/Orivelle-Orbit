//! Pure file planning for the Claude Desktop local configuration library dialect.
//!
//! Inputs assert an intended user root and an observed configuration dialect. This
//! module performs no I/O and establishes neither local write permission nor
//! client loading, connectivity or model authorization. Production writes remain
//! gated by the Service; this compiler cannot supply an installed-target receipt.

use crate::{
    config::FileChange,
    error::{Result, fail},
    identity::sha256,
};
use serde::{
    Deserialize,
    de::{MapAccess, Visitor},
};
use serde_json::{Map, Value, json, value::RawValue};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    io::{self, Write},
    path::{Component, Path},
};
use url::Url;
use uuid::Uuid;

pub const SUPPORTED_APP_VERSION: &str = "4.4212.0";
pub const SOURCE_MEMBER_SHA256: &str =
    "926694919e5ff5d05cb804945637ffbe6ae75bdeac5e461f30e7c3d5fa043ccb";
pub const MAX_FILE_BYTES: usize = 1024 * 1024;
pub const MAX_ENTRIES: usize = 256;
const MAX_EGRESS_HOSTS: usize = 500;
// DNS name (253), optional "*." (2), and optional ":65535" (6).
const MAX_EGRESS_HOST_BYTES: usize = 261;
const PROFILE_NAME: &str = "Orivelle Orbit";

/// Fixed public reader/schema superset, including legacy and internal leaves.
/// This is a coverage inventory for a later policy observer, never a permission
/// grant or a claim that arbitrary values are safe to migrate.
pub const SCHEMA_FLAT_KEYS: &[&str] = &[
    "allowOptionalClientAuth",
    "allowedPluginMarketplaces",
    "allowedPluginMcpServers",
    "allowedWorkspaceFolders",
    "alwaysStartWithDefaultModel",
    "artifactsDashboardEnabled",
    "artifactsDesignEnabled",
    "artifactsEnabled",
    "artifactsMotionEnabled",
    "artifactsSlidesEnabled",
    "autoModeEnabled",
    "autoUpdaterEnforcementHours",
    "banner",
    "blockReadsOutsideWorkingDirectories",
    "bootstrapEnabled",
    "bootstrapHeaders",
    "bootstrapHeadersHelper",
    "bootstrapOidc",
    "bootstrapUrl",
    "builtinBrowserAllowedDomains",
    "builtinBrowserBlockedDomains",
    "builtinBrowserDefaultDomainPolicy",
    "builtinBrowserEnabled",
    "builtinToolPolicy",
    "chatAdvancedFileAnalysisEnabled",
    "chatSessionRetentionDays",
    "chatTabEnabled",
    "claudeAiImport",
    "claudeCodeProcessWrapperEnabled",
    "claudeCodeSettings",
    "claudeInChromeEnabled",
    "claudeInChromePasswordManagersEnabled",
    "codeAllowedRepositories",
    "codeSessionRetentionDays",
    "configRecheckIntervalMinutes",
    "coworkEDRIntegration",
    "coworkEDRIntegrationVendor",
    "coworkEgressAllowedHosts",
    "coworkFileReadCheck",
    "coworkSessionRetentionDays",
    "coworkTabEnabled",
    "coworkVmIpv6Enabled",
    "dangerousMaxVersion",
    "defaultModelEffort",
    "deniedPluginMcpServers",
    "deploymentDisplayName",
    "deploymentDisplaySubtitle",
    "deploymentOrganizationUuid",
    "desktopHome",
    "developerModeEnabled",
    "dictationEnabled",
    "disableAutoUpdates",
    "disableBundledSkills",
    "disableBypassPermissionsMode",
    "disableConfigDeprecationWarnings",
    "disableConnectorSignInInApp",
    "disableDeepLinkRegistration",
    "disableDeploymentModeChooser",
    "disableDesktopLocalSessions",
    "disableEssentialTelemetry",
    "disableFeatureDiscovery",
    "disableLocalConfigCache",
    "disableMultiAccount",
    "disableNonessentialServices",
    "disableNonessentialTelemetry",
    "disableWslSessions",
    "disabledBuiltinTools",
    "egressProxyPacUrl",
    "egressProxyUrl",
    "endUserAttribution",
    "enduserAttribution",
    "entraAzureCloud",
    "entraClientId",
    "entraScope",
    "forceLoginOrgUUID",
    "hardwareBuddyEnabled",
    "importThirdPartyHistoryIntoOrgUUID",
    "inferenceAnthropicApiKey",
    "inferenceBedrockAwsCliPath",
    "inferenceBedrockAwsDir",
    "inferenceBedrockBaseUrl",
    "inferenceBedrockBearerToken",
    "inferenceBedrockProfile",
    "inferenceBedrockRegion",
    "inferenceBedrockServiceTier",
    "inferenceBedrockSsoAccountId",
    "inferenceBedrockSsoRegion",
    "inferenceBedrockSsoRoleName",
    "inferenceBedrockSsoStartUrl",
    "inferenceBedrockWebIdentityRoleArn",
    "inferenceCredentialHelper",
    "inferenceCredentialHelperArgs",
    "inferenceCredentialHelperSilentRefreshEnabled",
    "inferenceCredentialHelperTimeoutSec",
    "inferenceCredentialHelperTtlSec",
    "inferenceCredentialHelperWindows",
    "inferenceCredentialKind",
    "inferenceCustomHeaders",
    "inferenceFoundryApiKey",
    "inferenceFoundryAuthFlow",
    "inferenceFoundryBaseUrl",
    "inferenceFoundryClientId",
    "inferenceFoundryResource",
    "inferenceFoundryTenantId",
    "inferenceGatewayApiKey",
    "inferenceGatewayAuthScheme",
    "inferenceGatewayBaseUrl",
    "inferenceGatewayHeaders",
    "inferenceGatewayManagedClaudeCode",
    "inferenceGatewayOidc",
    "inferenceGatewayOidcAuthFlow",
    "inferenceIdpAuthFlow",
    "inferenceIdpOidc",
    "inferenceMaxTokensPerWindow",
    "inferenceModelPricing",
    "inferenceModelPricingEnabled",
    "inferenceModelPricingMultiplier",
    "inferenceModels",
    "inferenceProvider",
    "inferenceSessionLifetimeSec",
    "inferenceStreamIdleTimeoutSec",
    "inferenceTokenWarningPercent",
    "inferenceTokenWindowHours",
    "inferenceVertexBaseUrl",
    "inferenceVertexCredentialsFile",
    "inferenceVertexOAuthClientId",
    "inferenceVertexOAuthClientSecret",
    "inferenceVertexOAuthLoginHint",
    "inferenceVertexOAuthScopes",
    "inferenceVertexProjectId",
    "inferenceVertexRegion",
    "inferenceVertexWorkforceAudience",
    "inferenceVertexWorkforceAuthFlow",
    "inferenceVertexWorkforceOidc",
    "inferenceVertexWorkforceUserProject",
    "installedUpdateEnforcementEnabled",
    "installedUpdateEnforcementHours",
    "isClaudeCodeForDesktopEnabled",
    "isDesktopExtensionDirectoryEnabled",
    "isDesktopExtensionEnabled",
    "isDesktopExtensionSignatureRequired",
    "isDxtDirectoryEnabled",
    "isDxtEnabled",
    "isDxtSignatureRequired",
    "isLocalDevMcpEnabled",
    "keepAwakeEnabled",
    "loginSsoOrgDomain",
    "managedMcpServers",
    "mcpAppWidgetsEnabled",
    "mcpPersistentAlwaysAllowEnabled",
    "mcpScheduledTaskApprovalLifetimeDays",
    "mcpToolTimeoutSec",
    "microsoftAuthBroker",
    "microsoftAuthDefaultAccount",
    "modelCatalogEnabled",
    "modelCatalogUrl",
    "modelDiscoveryEnabled",
    "modelPrefer1mContext",
    "officeAccessPolicies",
    "officeDisabledFeatures",
    "officeDisabledHosts",
    "orgPluginSettings",
    "organizationInstructions",
    "organizationPluginsUrl",
    "otlpAttrMaxChars",
    "otlpAuthMode",
    "otlpContentCapture",
    "otlpDesktopLogLevel",
    "otlpEndpoint",
    "otlpHeaders",
    "otlpHeadersHelper",
    "otlpProtocol",
    "otlpResourceAttributes",
    "otlpTracesEnabled",
    "projectsEnabled",
    "quickEntryDefaultShortcut",
    "quickEntryFileSharingEnabled",
    "quickEntryScreenshotsEnabled",
    "quickWriteEnabled",
    "relaunchEnforcementHours",
    "releaseChannel",
    "relocateUncUserData",
    "requireCoworkFullVmSandbox",
    "scheduledTasksEnabled",
    "secureVmFeaturesEnabled",
    "selfHostedToken",
    "selfHostedUrl",
    "sessionRetentionHold",
    "sessionStatusLine",
    "sharingEndpoint",
    "skillBundles",
    "skillCreationEnabled",
    "skillScanningEnabled",
    "skipWebFetchPreflight",
    "sshClientPath",
    "sshConfigs",
    "sshHostAllowlist",
    "sshTransport",
    "toolSearchEnabled",
    "trustBootstrapDelivery",
    "trustBootstrapLocalExec",
    "updateViaUpdatesHost",
    "usageMetricsEnabled",
    "userContentRendererUrl",
    "userFileUploadsEnabled",
    "userPluginMarketplacesEnabled",
    "userPluginUploadsEnabled",
];
pub const SCHEMA_KEYS_SHA256: &str =
    "9f29e8cd7f3ae31c8035dbfbe2e12d1fccba0835818efcccb97b927529306b4f";

/// The pinned Desktop's native key-at-rest sweep visits these string leaves.
/// This inventory is only a read-only migration guard, never a decryption API.
#[cfg(any(target_os = "macos", windows, test))]
const NATIVE_SEALED_STRING_KEYS: &[&str] = &[
    "inferenceAnthropicApiKey",
    "inferenceBedrockBearerToken",
    "inferenceFoundryApiKey",
    "inferenceGatewayApiKey",
    "selfHostedToken",
];

/// Inspect only field identity and a borrowed string's emptiness/sealed prefix.
/// Other values are skipped, so inactive profiles' secrets never become Values
/// or retained strings. The caller owns and zeroizes the input buffer.
#[cfg(any(target_os = "macos", windows, test))]
pub(crate) fn native_migration_required(bytes: &[u8], main_config: bool) -> Result<bool> {
    use serde::de::{DeserializeSeed, IgnoredAny, MapAccess, Visitor};
    use std::fmt;

    enum Key {
        Enterprise,
        Secret,
        Other,
    }
    struct FieldKey;
    impl<'de> DeserializeSeed<'de> for FieldKey {
        type Value = Key;
        fn deserialize<D: serde::Deserializer<'de>>(
            self,
            de: D,
        ) -> std::result::Result<Key, D::Error> {
            de.deserialize_identifier(self)
        }
    }
    impl Visitor<'_> for FieldKey {
        type Value = Key;
        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("a configuration field")
        }
        fn visit_str<E: serde::de::Error>(self, value: &str) -> std::result::Result<Key, E> {
            Ok(if value == "enterpriseConfig" {
                Key::Enterprise
            } else if NATIVE_SEALED_STRING_KEYS.contains(&value) {
                Key::Secret
            } else {
                Key::Other
            })
        }
    }
    struct SecretString;
    impl<'de> DeserializeSeed<'de> for SecretString {
        type Value = bool;
        fn deserialize<D: serde::Deserializer<'de>>(
            self,
            de: D,
        ) -> std::result::Result<bool, D::Error> {
            de.deserialize_str(self)
        }
    }
    impl Visitor<'_> for SecretString {
        type Value = bool;
        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("a credential string")
        }
        fn visit_str<E: serde::de::Error>(self, value: &str) -> std::result::Result<bool, E> {
            // Match ECMAScript trimStart exactly, including BOM but excluding
            // U+0085, which Rust's broader is_whitespace would trim.
            let trimmed = value.trim_start_matches(|c| {
                matches!(c,
                '\u{0009}'..='\u{000d}' | '\u{0020}' | '\u{00a0}' | '\u{1680}' |
                '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' |
                '\u{205f}' | '\u{3000}' | '\u{feff}')
            });
            Ok(!value.is_empty() && !trimmed.starts_with("__sealed__:v1:"))
        }
    }
    struct Document(bool);
    impl<'de> DeserializeSeed<'de> for Document {
        type Value = bool;
        fn deserialize<D: serde::Deserializer<'de>>(
            self,
            de: D,
        ) -> std::result::Result<bool, D::Error> {
            de.deserialize_map(self)
        }
    }
    impl<'de> Visitor<'de> for Document {
        type Value = bool;
        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("a configuration object")
        }
        fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> std::result::Result<bool, M::Error> {
            let mut required = false;
            while let Some(key) = map.next_key_seed(FieldKey)? {
                if self.0 && matches!(key, Key::Enterprise) {
                    required = true;
                    map.next_value::<IgnoredAny>()?;
                } else if !self.0 && matches!(key, Key::Secret) {
                    required |= map.next_value_seed(SecretString)?;
                } else {
                    map.next_value::<IgnoredAny>()?;
                }
            }
            Ok(required)
        }
    }
    let mut de =
        serde_json::Deserializer::from_slice(bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes));
    let required = Document(main_config)
        .deserialize(&mut de)
        .map_err(|_| fail("claude_configuration_target_unverified"))?;
    de.end()
        .map_err(|_| fail("claude_configuration_target_unverified"))?;
    Ok(required)
}

// Exact flat leaves in this version's inference provider/credential/model
// connection schema, including the retired custom-header alias. Their values
// never migrate from an old provider into the new profile. Unclassified leaves
// are rejected, rather than copied or guessed to be a credential.
const REPLACED_CONNECTION_KEYS: &[&str] = &[
    "inferenceAnthropicApiKey",
    "inferenceBedrockAwsCliPath",
    "inferenceBedrockAwsDir",
    "inferenceBedrockBaseUrl",
    "inferenceBedrockBearerToken",
    "inferenceBedrockProfile",
    "inferenceBedrockRegion",
    "inferenceBedrockServiceTier",
    "inferenceBedrockSsoAccountId",
    "inferenceBedrockSsoRegion",
    "inferenceBedrockSsoRoleName",
    "inferenceBedrockSsoStartUrl",
    "inferenceBedrockWebIdentityRoleArn",
    "inferenceCredentialHelper",
    "inferenceCredentialHelperArgs",
    "inferenceCredentialHelperSilentRefreshEnabled",
    "inferenceCredentialHelperTimeoutSec",
    "inferenceCredentialHelperTtlSec",
    "inferenceCredentialHelperWindows",
    "inferenceCredentialKind",
    "inferenceCustomHeaders",
    "inferenceFoundryApiKey",
    "inferenceFoundryAuthFlow",
    "inferenceFoundryBaseUrl",
    "inferenceFoundryClientId",
    "inferenceFoundryResource",
    "inferenceFoundryTenantId",
    "inferenceGatewayApiKey",
    "inferenceGatewayAuthScheme",
    "inferenceGatewayBaseUrl",
    "inferenceGatewayHeaders",
    "inferenceGatewayManagedClaudeCode",
    "inferenceGatewayOidc",
    "inferenceGatewayOidcAuthFlow",
    "inferenceIdpAuthFlow",
    "inferenceIdpOidc",
    "inferenceModels",
    "inferenceProvider",
    "inferenceSessionLifetimeSec",
    "inferenceVertexBaseUrl",
    "inferenceVertexCredentialsFile",
    "inferenceVertexOAuthClientId",
    "inferenceVertexOAuthClientSecret",
    "inferenceVertexOAuthLoginHint",
    "inferenceVertexOAuthScopes",
    "inferenceVertexProjectId",
    "inferenceVertexRegion",
    "inferenceVertexWorkforceAudience",
    "inferenceVertexWorkforceAuthFlow",
    "inferenceVertexWorkforceOidc",
    "inferenceVertexWorkforceUserProject",
    "modelDiscoveryEnabled",
];

// Deliberately finite migration support. These settings have a non-secret scalar
// type and do not select a provider, credential, source or model. The existing
// egress list has a separate bounded host-only contract below; other settings
// require an explicit migration contract before an active profile can be copied.
const PRESERVED_BOOLEANS: &[&str] = &[
    "disableAutoUpdates",
    "disableDeploymentModeChooser",
    "disableEssentialTelemetry",
    "disableNonessentialServices",
    "disableNonessentialTelemetry",
    "keepAwakeEnabled",
];
const PRESERVED_POSITIVE_INTEGERS: &[&str] = &[
    "chatSessionRetentionDays",
    "codeSessionRetentionDays",
    "coworkSessionRetentionDays",
    "inferenceMaxTokensPerWindow",
    "inferenceStreamIdleTimeoutSec",
    "inferenceTokenWindowHours",
    "mcpToolTimeoutSec",
];

/// A complete read-only witness for the current metadata's applied local file.
/// Missing or unreadable active files must not be represented by an empty object.
pub struct ActiveProfile<'a> {
    pub id: &'a str,
    pub bytes: &'a [u8],
}

/// Caller assertions for a new, previously absent profile UUID.
pub struct LibraryPlan<'a> {
    pub app_version: &'a str,
    pub source_member_sha256: &'a str,
    /// Intended default Claude-3p root; filesystem ownership is checked elsewhere.
    pub user_root: &'a Path,
    pub profile_id: &'a str,
    pub gateway_base_url: &'a str,
    pub helper: &'a Path,
    pub binding_id: &'a str,
    pub model_id: &'a str,
    pub mapping: Option<&'a crate::claude_mapping::MappingPlan>,
    pub profile_before: Option<&'a [u8]>,
    pub metadata_before: Option<&'a [u8]>,
    pub main_before: Option<&'a [u8]>,
    pub active_profile: Option<ActiveProfile<'a>>,
}

pub struct CompiledLibrary {
    files: Vec<FileChange>,
    active_profile_id: Option<String>,
    active_profile_sha256: Option<String>,
    mapping: Option<crate::claude_mapping::MappingPlan>,
}

impl CompiledLibrary {
    pub fn mapping(&self) -> Option<&crate::claude_mapping::MappingPlan> {
        self.mapping.as_ref()
    }
    /// Activation order is profile, metadata, then deployment mode. A restoring
    /// transaction must reverse this order before deleting a newly added profile.
    pub fn files(&self) -> &[FileChange] {
        &self.files
    }

    pub fn into_files(self) -> Vec<FileChange> {
        self.files
    }

    /// The unchanged source file must be frozen and rechecked by the transaction,
    /// as it is a read dependency rather than one of the three written files.
    pub fn active_profile_id(&self) -> Option<&str> {
        self.active_profile_id.as_deref()
    }

    pub fn active_profile_sha256(&self) -> Option<&str> {
        self.active_profile_sha256.as_deref()
    }
}

fn canonical_id(value: &str) -> bool {
    Uuid::parse_str(value).is_ok_and(|id| id.hyphenated().to_string() == value)
}

fn safe_path(path: &Path) -> bool {
    path.is_absolute()
        && path
            .to_str()
            .is_some_and(|value| value.len() <= 4096 && !value.chars().any(char::is_control))
        && path.components().all(|part| {
            matches!(
                part,
                Component::Prefix(_) | Component::RootDir | Component::Normal(_)
            )
        })
}

fn valid_gateway(value: &str) -> bool {
    value.len() <= 2048
        && !value.chars().any(char::is_whitespace)
        && Url::parse(value).is_ok_and(|url| {
            url.scheme() == "https"
                && url.host().is_some()
                && url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none()
                && match url.host() {
                    Some(url::Host::Domain(domain)) => {
                        domain != "localhost" && !domain.ends_with(".localhost")
                    }
                    Some(url::Host::Ipv4(address)) => !address.is_loopback(),
                    Some(url::Host::Ipv6(address)) => !address.is_loopback(),
                    None => false,
                }
        })
}

fn object(bytes: &[u8]) -> Result<Map<String, Value>> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(fail("invalid_configuration"));
    }
    match serde_json::from_slice(bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes))
        .map_err(|_| fail("invalid_configuration"))?
    {
        Value::Object(map) => Ok(map),
        _ => Err(fail("invalid_configuration")),
    }
}

// A strict subset of the pinned public egress grammar: DNS labels, localhost,
// an optional leading wildcard and an optional port. Preserve accepted strings
// and their order verbatim; never supply a default list or broaden an old list.
fn valid_egress_host(value: &str) -> bool {
    if value == "*" {
        return true;
    }
    if value.is_empty() || value.len() > MAX_EGRESS_HOST_BYTES {
        return false;
    }
    let host = if let Some((host, port)) = value.rsplit_once(':') {
        if port.len() > 5
            || port.starts_with('0')
            || !port.bytes().all(|byte| byte.is_ascii_digit())
            || !port.parse::<u16>().is_ok_and(|port| port > 0)
        {
            return false;
        }
        host
    } else {
        value
    };
    if host == "localhost" {
        return true;
    }
    let host = host.strip_prefix("*.").unwrap_or(host);
    host.len() <= 253
        && host.contains('.')
        && host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && label.as_bytes()[0].is_ascii_alphanumeric()
                && label.as_bytes()[label.len() - 1].is_ascii_alphanumeric()
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
}

fn source_profile(bytes: &[u8]) -> Result<Map<String, Value>> {
    let mut source = object(bytes).map_err(|_| fail("claude_configuration_source_unsupported"))?;
    for (key, value) in &source {
        let preserved = if PRESERVED_BOOLEANS.contains(&key.as_str()) {
            value.is_boolean()
        } else if PRESERVED_POSITIVE_INTEGERS.contains(&key.as_str()) {
            value.as_u64().is_some_and(|number| number > 0)
        } else if key == "coworkEgressAllowedHosts" {
            value.as_array().is_some_and(|hosts| {
                hosts.len() <= MAX_EGRESS_HOSTS
                    && hosts
                        .iter()
                        .all(|host| host.as_str().is_some_and(valid_egress_host))
            })
        } else {
            REPLACED_CONNECTION_KEYS.contains(&key.as_str())
        };
        if !preserved {
            return Err(fail("claude_configuration_source_unsupported"));
        }
    }
    for key in REPLACED_CONNECTION_KEYS {
        source.remove(*key);
    }
    Ok(source)
}

pub(crate) fn selected_profile(bytes: &[u8]) -> Result<Option<String>> {
    let meta = raw_object(bytes).map_err(|_| fail("claude_configuration_source_unsupported"))?;
    selected_id(&meta)
}

fn selected_id(object: &RawObject<'_>) -> Result<Option<String>> {
    match object.0.get("appliedId") {
        None => Ok(None),
        Some(raw) => serde_json::from_str::<Option<String>>(raw.get())
            .map_err(|_| fail("claude_configuration_source_unsupported")),
    }
}

pub(crate) fn review_source_revision(
    metadata: Option<&[u8]>,
    active: Option<(&str, &[u8])>,
    main: Option<&[u8]>,
) -> Result<String> {
    let selected = metadata.map(selected_profile).transpose()?.flatten();
    if selected.as_deref() != active.map(|(id, _)| id) {
        return Err(fail("configuration_conflict"));
    }
    let mode = main
        .map(raw_object)
        .transpose()?
        .and_then(|o| o.0.get("deploymentMode").copied())
        .map(|v| serde_json::from_str::<String>(v.get()))
        .transpose()
        .map_err(|_| fail("claude_configuration_source_unsupported"))?;
    Ok(sha256(
        &serde_json::to_vec(&(mode, selected, active.map(|(_, b)| sha256(b))))
            .map_err(|_| fail("serialization_failed"))?,
    ))
}

struct BoundedWriter(Vec<u8>);
impl Write for BoundedWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.0.len().saturating_add(bytes.len()) > MAX_FILE_BYTES {
            return Err(io::Error::other("configuration bound"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn encode(value: Map<String, Value>) -> Result<Vec<u8>> {
    let mut writer = BoundedWriter(Vec::new());
    serde_json::to_writer_pretty(&mut writer, &value).map_err(|_| fail("invalid_configuration"))?;
    writer
        .write_all(b"\n")
        .map_err(|_| fail("invalid_configuration"))?;
    Ok(writer.0)
}

struct RawObject<'a>(BTreeMap<String, &'a RawValue>);
impl<'de> Deserialize<'de> for RawObject<'de> {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        struct MainVisitor;
        impl<'de> Visitor<'de> for MainVisitor {
            type Value = RawObject<'de>;
            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a configuration object with unique decoded keys")
            }
            fn visit_map<A: MapAccess<'de>>(
                self,
                mut entries: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                let mut values = BTreeMap::new();
                while let Some(key) = entries.next_key::<String>()? {
                    let value = entries.next_value::<&RawValue>()?;
                    if values.insert(key, value).is_some() {
                        return Err(serde::de::Error::custom("duplicate configuration key"));
                    }
                }
                Ok(RawObject(values))
            }
        }
        deserializer.deserialize_map(MainVisitor)
    }
}

fn raw_object(bytes: &[u8]) -> Result<RawObject<'_>> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(fail("configuration_conflict"));
    }
    serde_json::from_slice(bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes))
        .map_err(|_| fail("configuration_conflict"))
}
fn raw_main(bytes: &[u8]) -> Result<RawObject<'_>> {
    let object = raw_object(bytes)?;
    if object
        .0
        .get("preferences")
        .is_some_and(|value| !value.get().starts_with('{'))
    {
        return Err(fail("configuration_conflict"));
    }
    Ok(object)
}

// Keep unowned values as raw JSON. In particular, reserializing an arbitrary
// preference or a foreign index row through Value can round decimal numbers.
fn encode_raw(value: &impl serde::Serialize) -> Result<Vec<u8>> {
    let mut writer = BoundedWriter(Vec::new());
    serde_json::to_writer_pretty(&mut writer, value).map_err(|_| fail("configuration_conflict"))?;
    writer
        .write_all(b"\n")
        .map_err(|_| fail("configuration_conflict"))?;
    Ok(writer.0)
}

struct LibraryIndex<'a> {
    fields: RawObject<'a>,
    entries: Vec<&'a RawValue>,
    ids: Vec<String>,
}
impl<'a> LibraryIndex<'a> {
    fn read(bytes: Option<&'a [u8]>) -> Result<Self> {
        let fields = raw_object(bytes.unwrap_or(br#"{"entries":[]}"#))?;
        if fields.0.contains_key("hybridPointer") {
            return Err(fail("claude_configuration_source_unsupported"));
        }
        let entries: Vec<&RawValue> = match fields.0.get("entries") {
            Some(raw) => {
                serde_json::from_str(raw.get()).map_err(|_| fail("configuration_conflict"))?
            }
            None => Vec::new(),
        };
        if entries.len() > MAX_ENTRIES {
            return Err(fail("configuration_conflict"));
        }
        let mut ids = Vec::with_capacity(entries.len());
        let mut seen = BTreeSet::new();
        for row in &entries {
            #[derive(Deserialize)]
            struct Entry {
                id: String,
                name: String,
            }
            let entry: Entry =
                serde_json::from_str(row.get()).map_err(|_| fail("configuration_conflict"))?;
            if entry.id.is_empty()
                || entry.id.len() > 256
                || entry.id.chars().any(char::is_control)
                || entry.name.trim().is_empty()
                || entry.name.len() > 256
                || !seen.insert(entry.id.clone())
            {
                return Err(fail("configuration_conflict"));
            }
            ids.push(entry.id);
        }
        Ok(Self {
            fields,
            entries,
            ids,
        })
    }

    fn selection(&self) -> Result<Option<String>> {
        selected_id(&self.fields)
    }

    fn owned_entry(&self, profile_id: &str) -> Result<usize> {
        let position = self
            .ids
            .iter()
            .position(|id| id == profile_id)
            .ok_or_else(|| fail("configuration_conflict"))?;
        let row = raw_object(self.entries[position].get().as_bytes())?;
        if row.0.len() != 2
            || row
                .0
                .get("name")
                .and_then(|v| serde_json::from_str::<String>(v.get()).ok())
                .as_deref()
                != Some(PROFILE_NAME)
        {
            return Err(fail("configuration_conflict"));
        }
        Ok(position)
    }

    fn encode(
        self,
        selected: Option<&RawValue>,
        appended: Option<&RawValue>,
        retain_empty_entries: bool,
    ) -> Result<Vec<u8>> {
        let rows: Vec<_> = self.entries.into_iter().chain(appended).collect();
        let entries =
            serde_json::value::to_raw_value(&rows).map_err(|_| fail("configuration_conflict"))?;
        let mut fields = self.fields.0;
        if rows.is_empty() && !retain_empty_entries {
            fields.remove("entries");
        } else {
            fields.insert("entries".into(), &entries);
        }
        if let Some(selected) = selected {
            fields.insert("appliedId".into(), selected);
        } else {
            fields.remove("appliedId");
        }
        encode_raw(&fields)
    }
}

fn apply_index(index: LibraryIndex<'_>, profile_id: &str) -> Result<Vec<u8>> {
    let row = serde_json::value::to_raw_value(&json!({"id":profile_id,"name":PROFILE_NAME}))
        .map_err(|_| fail("invalid_configuration"))?;
    let selected =
        serde_json::value::to_raw_value(profile_id).map_err(|_| fail("invalid_configuration"))?;
    index.encode(Some(&selected), Some(&row), true)
}

/// The profile ID remains part of the route even if a switcher copies its bytes.
pub(crate) fn profile_revision(profile_id: &str, profile_sha256: &str) -> String {
    sha256(format!("{profile_id}\n{profile_sha256}").as_bytes())
}

pub(crate) fn check_mapping_selection(
    metadata: &[u8],
    profile_id: &str,
    main: &[u8],
    require_owned_row: bool,
) -> Result<()> {
    let index = LibraryIndex::read(Some(metadata))?;
    if require_owned_row {
        index.owned_entry(profile_id)?;
    } else if !index.ids.iter().any(|id| id == profile_id) {
        return Err(fail("configuration_conflict"));
    }
    let fields = raw_object(main)?;
    if index.selection()?.as_deref() != Some(profile_id)
        || fields.0.contains_key("enterpriseConfig")
        || !fields.0.get("deploymentMode").is_some_and(|value| {
            serde_json::from_str::<String>(value.get()).is_ok_and(|v| v == "3p")
        })
    {
        return Err(fail("configuration_conflict"));
    }
    Ok(())
}

pub(crate) fn restore_index(
    original: Option<&[u8]>,
    applied: &[u8],
    current: Option<&[u8]>,
    profile_id: &str,
) -> Result<Option<Vec<u8>>> {
    if current == Some(applied) {
        return Ok(original.map(<[u8]>::to_vec));
    }
    let bytes = current.ok_or_else(|| fail("configuration_conflict"))?;
    let mut index = LibraryIndex::read(Some(bytes))?;
    if index.selection()?.as_deref() != Some(profile_id) {
        return Err(fail("configuration_conflict"));
    }
    let position = index.owned_entry(profile_id)?;
    index.entries.remove(position);
    let saved = LibraryIndex::read(original)?;
    if let Some(old_id) = saved.selection()?
        && !index.ids.contains(&old_id)
    {
        return Err(fail("configuration_conflict"));
    }
    if original.is_none()
        && index.entries.is_empty()
        && index
            .fields
            .0
            .keys()
            .all(|key| matches!(key.as_str(), "entries" | "appliedId"))
    {
        return Ok(None);
    }
    Ok(Some(index.encode(
        saved.fields.0.get("appliedId").copied(),
        None,
        original.is_some() && saved.fields.0.contains_key("entries"),
    )?))
}

pub(crate) fn restore_switch_main(
    original: Option<&[u8]>,
    applied: &[u8],
    current: Option<&[u8]>,
) -> Result<Option<Vec<u8>>> {
    if current == Some(applied) {
        return Ok(original.map(<[u8]>::to_vec));
    }
    let mut doc = raw_object(current.ok_or_else(|| fail("configuration_conflict"))?)?;
    if doc
        .0
        .get("deploymentMode")
        .and_then(|v| serde_json::from_str::<String>(v.get()).ok())
        .as_deref()
        != Some("3p")
    {
        return Err(fail("configuration_conflict"));
    }
    let before = raw_object(original.unwrap_or(b"{}"))?;
    if let Some(mode) = before.0.get("deploymentMode") {
        doc.0.insert("deploymentMode".into(), mode);
    } else {
        doc.0.remove("deploymentMode");
    }
    if original.is_none() && doc.0.is_empty() {
        return Ok(None);
    }
    Ok(Some(encode_raw(&doc.0)?))
}
/// Fixed, explicitly selected restoration of Claude's deployment mode. Every
/// current raw value, including the complete optional preferences object, is
/// retained. This is not a general JSON merge or a write authorization.
#[cfg(test)]
pub(crate) fn restore_main_preserving_preferences<'a>(
    original_before: &'a [u8],
    applied: &'a [u8],
    current: &'a [u8],
) -> Result<Vec<u8>> {
    restore_main_settings(Some(original_before), applied, current, false)
}

/// Explicit v2 coordination preserves all current unowned main settings. Only
/// deploymentMode is restored; the encrypted transaction freezes every byte.
pub(crate) fn restore_main_settings<'a>(
    original_before: Option<&'a [u8]>,
    applied: &'a [u8],
    current: &'a [u8],
    preserve_main_settings: bool,
) -> Result<Vec<u8>> {
    let original = raw_main(original_before.unwrap_or(b"{}"))?;
    let applied = raw_main(applied)?;
    let mut current = raw_main(current)?;
    let original_mode = original.0.get("deploymentMode").copied();
    if original_mode.is_some_and(|value| {
        !serde_json::from_str::<String>(value.get())
            .is_ok_and(|value| value == "1p" || value == "3p")
    }) || !applied.0.get("deploymentMode").is_some_and(|value| {
        serde_json::from_str::<String>(value.get()).is_ok_and(|value| value == "3p")
    }) {
        return Err(fail("configuration_conflict"));
    }
    let applied_keys = applied
        .0
        .keys()
        .filter(|key| key.as_str() != "preferences")
        .collect::<Vec<_>>();
    let current_keys = current
        .0
        .keys()
        .filter(|key| key.as_str() != "preferences")
        .collect::<Vec<_>>();
    if preserve_main_settings {
        if current.0.get("deploymentMode").map(|v| v.get())
            != applied.0.get("deploymentMode").map(|v| v.get())
        {
            return Err(fail("configuration_conflict"));
        }
    } else if applied_keys != current_keys
        || applied_keys
            .iter()
            .any(|key| applied.0.get(*key).unwrap().get() != current.0.get(*key).unwrap().get())
    {
        return Err(fail("configuration_conflict"));
    }
    if let Some(mode) = original_mode {
        current.0.insert("deploymentMode".into(), mode);
    } else {
        current.0.remove("deploymentMode");
    }
    let mut writer = BoundedWriter(Vec::new());
    serde_json::to_writer_pretty(&mut writer, &current.0)
        .map_err(|_| fail("configuration_conflict"))?;
    writer
        .write_all(b"\n")
        .map_err(|_| fail("configuration_conflict"))?;
    Ok(writer.0)
}

/// Preserve the supported active local settings and plan exactly three changes.
/// Unknown versions/sources and active profile semantics fail closed. Only an
/// absent active selection permits an empty starting profile. Existing profiles are
/// read dependencies and never appear among the written files.
pub fn compile(plan: &LibraryPlan<'_>) -> Result<CompiledLibrary> {
    compile_with_timeout(plan, HelperTimeout::Current, true)
}

/// Only reconstruct the fixed historical output for strict journal validation.
/// New user configurations always enter through `compile`.
pub(crate) fn compile_legacy_helper_timeout(plan: &LibraryPlan<'_>) -> Result<CompiledLibrary> {
    compile_with_timeout(plan, HelperTimeout::LegacyFiveSeconds, false)
}

pub(crate) fn compile_legacy_json(plan: &LibraryPlan<'_>) -> Result<CompiledLibrary> {
    compile_with_timeout(plan, HelperTimeout::Current, false)
}

enum HelperTimeout {
    Current,
    LegacyFiveSeconds,
}

fn compile_with_timeout(
    plan: &LibraryPlan<'_>,
    timeout: HelperTimeout,
    preserve_json: bool,
) -> Result<CompiledLibrary> {
    if plan.app_version.is_empty()
        || plan.app_version.len() > 128
        || plan.source_member_sha256.len() != 64
        || !plan
            .source_member_sha256
            .bytes()
            .all(|b| b.is_ascii_hexdigit())
    {
        return Err(fail("claude_configuration_target_unverified"));
    }
    if !safe_path(plan.user_root)
        || plan.user_root.file_name().and_then(|name| name.to_str()) != Some("Claude-3p")
        || !safe_path(plan.helper)
        || plan
            .helper
            .to_str()
            .is_some_and(|command| command.contains(['"', '%']))
        || !canonical_id(plan.profile_id)
        || !canonical_id(plan.binding_id)
        || !valid_gateway(plan.gateway_base_url)
        || plan.model_id.is_empty()
        || plan.model_id.len() > 128
        || !plan.model_id.bytes().all(|byte| byte.is_ascii_graphic())
    {
        return Err(fail("invalid_configuration"));
    }
    if plan.profile_before.is_some() {
        return Err(fail("configuration_conflict"));
    }
    if let Some(mapping) = plan.mapping {
        mapping.validate()?;
        if mapping.binding_id != plan.binding_id
            || mapping.profile_id != plan.profile_id
            || mapping.upstream_model_id != plan.model_id
        {
            return Err(fail("invalid_configuration"));
        }
    }
    // Historical snapshots used Value's decoded representation, including its duplicate-key
    // and number handling. The journal still binds the complete original bytes.
    let historical_json = |bytes: Option<&[u8]>| -> Result<Option<Vec<u8>>> {
        if preserve_json {
            return Ok(None);
        }
        bytes
            .map(|raw| serde_json::to_vec(&object(raw)?).map_err(|_| fail("invalid_configuration")))
            .transpose()
    };
    let legacy_meta = historical_json(plan.metadata_before)?;
    let legacy_main = historical_json(plan.main_before)?;
    let unsupported = || fail("claude_configuration_source_unsupported");
    let index = LibraryIndex::read(legacy_meta.as_deref().or(plan.metadata_before))
        .map_err(|_| unsupported())?;
    if index.ids.len() >= MAX_ENTRIES {
        return Err(unsupported());
    }
    if index.ids.iter().any(|id| id == plan.profile_id) {
        return Err(fail("configuration_conflict"));
    }
    let selection = index.selection()?;
    if selection.as_deref() != plan.active_profile.as_ref().map(|active| active.id)
        || selection
            .as_ref()
            .is_some_and(|id| !canonical_id(id) || !index.ids.contains(id))
    {
        return Err(unsupported());
    }
    let mut profile = plan
        .active_profile
        .as_ref()
        .map(|active| source_profile(active.bytes))
        .transpose()?
        .unwrap_or_default();
    let mut main = raw_object(legacy_main.as_deref().or(plan.main_before).unwrap_or(b"{}"))
        .map_err(|_| fail("invalid_configuration"))?;
    if main.0.get("deploymentMode").is_some_and(|mode| {
        !serde_json::from_str::<String>(mode.get()).is_ok_and(|v| matches!(v.as_str(), "1p" | "3p"))
    }) {
        return Err(unsupported());
    }
    profile.extend(Map::from_iter([
        ("inferenceProvider".into(), json!("gateway")),
        (
            "inferenceGatewayBaseUrl".into(),
            json!(
                plan.mapping
                    .map(|mapping| mapping.base_url())
                    .unwrap_or_else(|| plan.gateway_base_url.into())
            ),
        ),
        ("inferenceCredentialKind".into(), json!("helper-script")),
        ("inferenceCredentialHelper".into(), json!(plan.helper)),
        (
            "inferenceCredentialHelperArgs".into(),
            json!([
                if plan.mapping.is_some() {
                    "claude-map"
                } else {
                    "claude"
                },
                plan.binding_id
            ]),
        ),
        ("inferenceCredentialHelperTtlSec".into(), json!(3600)),
        (
            "inferenceCredentialHelperTimeoutSec".into(),
            json!(match timeout {
                HelperTimeout::Current => 60,
                HelperTimeout::LegacyFiveSeconds => 5,
            }),
        ),
        (
            "inferenceCredentialHelperSilentRefreshEnabled".into(),
            json!(true),
        ),
        ("inferenceGatewayAuthScheme".into(), json!("bearer")),
        ("modelDiscoveryEnabled".into(), json!(false)),
        (
            "inferenceModels".into(),
            plan.mapping
                .map(|mapping| json!(mapping.aliases))
                .unwrap_or_else(|| json!([{ "name": plan.model_id }])),
        ),
    ]));
    #[cfg(windows)]
    profile.insert(
        "inferenceCredentialHelperWindows".into(),
        json!(plan.helper),
    );
    let library = plan.user_root.join("configLibrary");
    let mode = serde_json::value::to_raw_value("3p").map_err(|_| fail("invalid_configuration"))?;
    main.0.insert("deploymentMode".into(), &mode);
    let mut metadata_after = apply_index(index, plan.profile_id)?;
    let mut main_after = encode_raw(&main.0).map_err(|_| fail("invalid_configuration"))?;
    if !preserve_json {
        // Reproduce the JSON encoding used by authenticated older snapshots.
        metadata_after = encode(object(&metadata_after)?)?;
        main_after = encode(object(&main_after)?)?;
    }
    let changes = [
        (
            library.join(format!("{}.json", plan.profile_id)),
            None,
            encode(profile)?,
        ),
        (
            library.join("_meta.json"),
            plan.metadata_before,
            metadata_after,
        ),
        (
            plan.user_root.join("claude_desktop_config.json"),
            plan.main_before,
            main_after,
        ),
    ];
    let mut files = Vec::with_capacity(3);
    for (path, before, after) in changes {
        files.push(FileChange {
            path: path
                .to_str()
                .ok_or_else(|| fail("invalid_configuration"))?
                .into(),
            before: before.map(<[u8]>::to_vec),
            after: Some(after),
        });
    }
    Ok(CompiledLibrary {
        files,
        mapping: plan.mapping.cloned(),
        active_profile_id: plan.active_profile.as_ref().map(|source| source.id.into()),
        active_profile_sha256: plan
            .active_profile
            .as_ref()
            .map(|source| sha256(source.bytes)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const NEW_ID: &str = "11111111-1111-4111-8111-111111111111";
    const OLD_ID: &str = "22222222-2222-4222-8222-222222222222";
    const INACTIVE_ID: &str = "33333333-3333-4333-8333-333333333333";
    const BINDING: &str = "44444444-4444-4444-8444-444444444444";

    struct Fixture {
        root: PathBuf,
        helper: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join("orbit-pure").join("Claude-3p");
            Self {
                helper: root.parent().unwrap().join("orbit-credential-helper"),
                root,
            }
        }

        fn plan(&self) -> LibraryPlan<'_> {
            LibraryPlan {
                app_version: SUPPORTED_APP_VERSION,
                source_member_sha256: SOURCE_MEMBER_SHA256,
                user_root: &self.root,
                profile_id: NEW_ID,
                gateway_base_url: "https://edge.example.test/anthropic",
                helper: &self.helper,
                binding_id: BINDING,
                model_id: "orbit-sonnet",
                mapping: None,
                profile_before: None,
                metadata_before: None,
                main_before: None,
                active_profile: None,
            }
        }
    }

    fn bytes(value: Value) -> Vec<u8> {
        serde_json::to_vec(&value).unwrap()
    }

    #[test]
    fn claude_main_preferences_v1_preserves_raw_values_and_rejects_other_changes() {
        let original = br#"{"deploymentMode":"1p","keep":42,"preferences":{"original":true}}"#;
        let applied = br#"{"deploymentMode":"3p","keep":42,"preferences":{"original":true}}"#;
        let preferences = r#"{"huge":184467440737095516170000,"fraction":1.000000000000000000000001,"escaped":"a\u0062","unknown":[true,null,{"x":1}]}"#;
        let current = format!(r#"{{"deploymentMode":"3p","keep":42,"preferences":{preferences}}}"#);
        let result =
            restore_main_preserving_preferences(original, applied, current.as_bytes()).unwrap();
        let result = raw_main(&result).unwrap();
        assert_eq!(result.0["deploymentMode"].get(), r#""1p""#);
        assert_eq!(result.0["preferences"].get(), preferences);
        assert_eq!(result.0["keep"].get(), "42");
        let absent = restore_main_preserving_preferences(
            br#"{"keep":42}"#,
            applied,
            br#"{"deploymentMode":"3p","keep":42}"#,
        )
        .unwrap();
        let absent = raw_main(&absent).unwrap();
        assert!(!absent.0.contains_key("deploymentMode") && !absent.0.contains_key("preferences"));
        let already_3p =
            restore_main_preserving_preferences(applied, applied, current.as_bytes()).unwrap();
        assert_eq!(
            raw_main(&already_3p).unwrap().0["deploymentMode"].get(),
            r#""3p""#
        );
        for invalid in [
            r#"{"deploymentMode":"3p","keep":42,"extra":true}"#,
            r#"{"deploymentMode":"3p"}"#,
            r#"{"deploymentMode":"3p","keep":43}"#,
            r#"{"deploymentMode":"1p","keep":42}"#,
            r#"{"deploymentMode":"3p","keep":42,"preferences":null}"#,
            r#"{"deploymentMode":"3p","keep":42,"preferences":[]}"#,
            r#"{"deploymentMode":"3p","keep":42,"preferences":{},"prefer\u0065nces":{}}"#,
            r#"{"deploymentMode":"3p","deploym\u0065ntMode":"3p","keep":42}"#,
            r#"{"deploymentMode":"3p","keep":42} {}"#,
            "[]",
        ] {
            assert_eq!(
                restore_main_preserving_preferences(original, applied, invalid.as_bytes())
                    .unwrap_err()
                    .code,
                "configuration_conflict"
            );
        }
        assert!(
            restore_main_preserving_preferences(
                br#"{"deploymentMode":"unknown"}"#,
                br#"{"deploymentMode":"3p"}"#,
                br#"{"deploymentMode":"3p"}"#,
            )
            .is_err()
        );
        assert!(
            restore_main_preserving_preferences(
                br#"{"keep":184467440737095516170000}"#,
                br#"{"deploymentMode":"3p","keep":184467440737095516170000}"#,
                br#"{"deploymentMode":"3p","keep":184467440737095516170001}"#,
            )
            .is_err(),
            "different large non-preference numbers must never collapse through f64"
        );
        assert!(
            restore_main_preserving_preferences(original, applied, &vec![b' '; MAX_FILE_BYTES + 1])
                .is_err()
        );
    }

    #[test]
    fn claude_mapping_compiler_is_explicit_and_preserves_https_authority() {
        let fixture = Fixture::new();
        let manager = crate::claude_mapping::MappingManager::default();
        let (mapping, _ticket) = manager
            .reserve(BINDING, NEW_ID, "gpt-6-luna", None, ["GPT 6 Luna"; 3])
            .unwrap();
        let mut plan = fixture.plan();
        plan.model_id = "gpt-6-luna";
        plan.mapping = Some(&mapping);
        let compiled = compile(&plan).unwrap();
        let profile = after(&compiled.files()[0]);
        assert_eq!(profile["inferenceGatewayBaseUrl"], mapping.base_url());
        assert_eq!(
            profile["inferenceCredentialHelperArgs"],
            json!(["claude-map", BINDING])
        );
        assert_eq!(profile["inferenceModels"], json!(mapping.aliases));
        assert_eq!(profile["inferenceCredentialHelperTimeoutSec"], 60);
        assert!(profile.get("inferenceGatewayApiKey").is_none());
        assert_eq!(compiled.mapping(), Some(&mapping));
        assert!(
            profile["inferenceModels"]
                .as_array()
                .unwrap()
                .iter()
                .all(|model| {
                    model["name"].as_str().unwrap().starts_with("claude-")
                        && model["labelOverride"] == "GPT 6 Luna"
                        && model.get("supports1m").is_none()
                })
        );
        plan.gateway_base_url = "http://127.0.0.1:1234";
        assert_eq!(compile(&plan).err().unwrap().code, "invalid_configuration");
        plan.gateway_base_url = "https://edge.example.test/anthropic";
        plan.mapping = None;
        let direct = compile(&plan).unwrap();
        assert_eq!(
            after(&direct.files()[0])["inferenceGatewayBaseUrl"],
            plan.gateway_base_url
        );
        assert_eq!(
            after(&direct.files()[0])["inferenceCredentialHelperArgs"],
            json!(["claude", BINDING])
        );
    }

    fn after(change: &FileChange) -> Value {
        serde_json::from_slice(change.after.as_ref().unwrap()).unwrap()
    }

    fn existing_meta() -> Value {
        json!({
            "appliedId": OLD_ID,
            "entries": [
                {"id": OLD_ID, "name": "Original", "extra": {"retain": true}},
                {"id": INACTIVE_ID, "name": "Inactive bootstrap"}
            ],
            "unknownMetadata": [1, {"retain": true}]
        })
    }

    fn rejected(plan: &LibraryPlan<'_>, code: &str) {
        assert!(compile(plan).is_err_and(|error| error.code == code));
    }

    #[test]
    fn fresh_profile_is_flat_and_activates_exactly_three_files_in_order() {
        let fixture = Fixture::new();
        let compiled = compile(&fixture.plan()).unwrap();
        let files = compiled.files();
        assert_eq!(files.len(), 3);
        assert!(files.iter().all(|file| file.before.is_none()));
        assert_eq!(
            Path::new(&files[0].path),
            fixture
                .root
                .join("configLibrary")
                .join(format!("{NEW_ID}.json"))
        );
        assert_eq!(
            Path::new(&files[1].path),
            fixture.root.join("configLibrary/_meta.json")
        );
        assert_eq!(
            Path::new(&files[2].path),
            fixture.root.join("claude_desktop_config.json")
        );
        let profile = after(&files[0]);
        assert_eq!(
            profile.as_object().unwrap().len(),
            if cfg!(windows) { 12 } else { 11 }
        );
        assert_eq!(profile["inferenceProvider"], "gateway");
        assert_eq!(profile["inferenceCredentialKind"], "helper-script");
        assert_eq!(profile["inferenceCredentialHelper"], json!(fixture.helper));
        #[cfg(windows)]
        assert_eq!(
            profile["inferenceCredentialHelperWindows"],
            json!(fixture.helper)
        );
        #[cfg(not(windows))]
        assert!(profile.get("inferenceCredentialHelperWindows").is_none());
        assert_eq!(
            profile["inferenceCredentialHelperArgs"],
            json!(["claude", BINDING])
        );
        assert_eq!(profile["inferenceCredentialHelperTtlSec"], 3600);
        assert_eq!(profile["inferenceCredentialHelperTimeoutSec"], 60);
        assert_eq!(
            profile["inferenceCredentialHelperSilentRefreshEnabled"],
            true
        );
        assert_eq!(profile["inferenceGatewayAuthScheme"], "bearer");
        assert_eq!(
            profile["inferenceGatewayBaseUrl"],
            fixture.plan().gateway_base_url
        );
        assert_eq!(
            profile["inferenceModels"],
            json!([{"name": "orbit-sonnet"}])
        );
        assert_eq!(profile["modelDiscoveryEnabled"], false);
        assert_eq!(after(&files[1])["appliedId"], NEW_ID);
        assert_eq!(
            after(&files[1])["entries"],
            json!([{"id": NEW_ID, "name": PROFILE_NAME}])
        );
        assert_eq!(after(&files[2]), json!({"deploymentMode": "3p"}));
        assert!(compiled.active_profile_id().is_none());
        assert!(compiled.active_profile_sha256().is_none());
    }

    #[test]
    fn migrated_profile_preserves_safe_settings_without_copying_old_credentials() {
        let fixture = Fixture::new();
        let mut source = Map::new();
        for key in REPLACED_CONNECTION_KEYS {
            source.insert((*key).into(), json!("old-provider-secret-marker"));
        }
        source.insert("keepAwakeEnabled".into(), json!(true));
        source.insert("disableDeploymentModeChooser".into(), json!(false));
        source.insert("coworkEgressAllowedHosts".into(), json!(["*"]));
        source.insert("inferenceMaxTokensPerWindow".into(), json!(4000));
        source.insert("inferenceTokenWindowHours".into(), json!(8));
        let source = bytes(Value::Object(source));
        let unchanged = source.clone();
        let meta_value = existing_meta();
        let meta = bytes(meta_value.clone());
        let main_value = json!({
            "deploymentMode": "1p",
            "unrelatedSignInState": {"retain": true},
            "unknownMain": [false, "retained"]
        });
        let main = bytes(main_value.clone());
        let mut plan = fixture.plan();
        plan.metadata_before = Some(&meta);
        plan.main_before = Some(&main);
        plan.active_profile = Some(ActiveProfile {
            id: OLD_ID,
            bytes: &source,
        });
        let compiled = compile(&plan).unwrap();
        let files = compiled.files();
        let profile = after(&files[0]);
        assert_eq!(profile["keepAwakeEnabled"], true);
        assert_eq!(profile["disableDeploymentModeChooser"], false);
        assert_eq!(profile["coworkEgressAllowedHosts"], json!(["*"]));
        assert_eq!(profile["inferenceMaxTokensPerWindow"], 4000);
        assert_eq!(profile["inferenceTokenWindowHours"], 8);
        assert!(
            !files[0]
                .after
                .as_ref()
                .unwrap()
                .windows(26)
                .any(|part| part == b"old-provider-secret-marker")
        );
        assert!(REPLACED_CONNECTION_KEYS.iter().all(|key| {
            profile.get(*key).is_none()
                || [
                    "inferenceProvider",
                    "inferenceGatewayBaseUrl",
                    "inferenceCredentialKind",
                    "inferenceCredentialHelper",
                    "inferenceCredentialHelperWindows",
                    "inferenceCredentialHelperArgs",
                    "inferenceCredentialHelperTtlSec",
                    "inferenceCredentialHelperTimeoutSec",
                    "inferenceCredentialHelperSilentRefreshEnabled",
                    "inferenceGatewayAuthScheme",
                    "inferenceModels",
                    "modelDiscoveryEnabled",
                ]
                .contains(key)
        }));
        assert!(source == unchanged);
        assert!(
            files
                .iter()
                .all(|file| !file.path.ends_with(&format!("{OLD_ID}.json")))
        );
        let merged_meta = after(&files[1]);
        assert_eq!(merged_meta["entries"][0], meta_value["entries"][0]);
        assert_eq!(merged_meta["entries"][1], meta_value["entries"][1]);
        assert_eq!(
            merged_meta["unknownMetadata"],
            meta_value["unknownMetadata"]
        );
        assert_eq!(merged_meta["appliedId"], NEW_ID);
        let mut expected_main = main_value;
        expected_main["deploymentMode"] = json!("3p");
        assert_eq!(after(&files[2]), expected_main);
        assert!(files[1].before.as_deref() == Some(meta.as_slice()));
        assert!(files[2].before.as_deref() == Some(main.as_slice()));
        assert_eq!(compiled.active_profile_id(), Some(OLD_ID));
        assert_eq!(
            compiled.active_profile_sha256(),
            Some(sha256(&source).as_str())
        );
    }

    #[test]
    fn hybrid_bootstrap_unknown_and_incompatible_model_settings_are_not_migrated() {
        let fixture = Fixture::new();
        let meta = bytes(existing_meta());
        for source in [
            json!({"bootstrapUrl": "https://managed.example.test/config"}),
            json!({"bootstrapEnabled": false}),
            json!({"selfHostedToken": "synthetic-old-secret"}),
            json!({"otlpHeaders": {"Authorization": "synthetic-old-secret"}}),
            json!({"unknownSetting": true}),
            json!({"defaultModelEffort": "high"}),
            json!({"alwaysStartWithDefaultModel": true}),
            json!({"modelCatalogEnabled": true}),
            json!({"modelCatalogUrl": "https://other.example.test/catalog"}),
            json!({"inference": {"provider": "gateway"}}),
            json!({"keepAwakeEnabled": "not-a-boolean"}),
            json!({"coworkEgressAllowedHosts": ["*"], "unknownEgressSetting": true}),
        ] {
            let source = bytes(source);
            let mut plan = fixture.plan();
            plan.metadata_before = Some(&meta);
            plan.active_profile = Some(ActiveProfile {
                id: OLD_ID,
                bytes: &source,
            });
            rejected(&plan, "claude_configuration_source_unsupported");
        }
        for pointer in [
            Value::Null,
            json!(false),
            json!({"bootstrapUrl": "https://example.test"}),
        ] {
            let mut value = existing_meta();
            value["hybridPointer"] = pointer;
            let meta = bytes(value);
            let mut plan = fixture.plan();
            plan.metadata_before = Some(&meta);
            plan.active_profile = Some(ActiveProfile {
                id: OLD_ID,
                bytes: b"{}",
            });
            rejected(&plan, "claude_configuration_source_unsupported");
        }
    }

    #[test]
    fn existing_egress_host_patterns_are_preserved_exactly_within_finite_bounds() {
        let fixture = Fixture::new();
        let meta = bytes(existing_meta());
        let longest_host = [
            "a".repeat(63),
            "b".repeat(63),
            "c".repeat(63),
            "d".repeat(61),
        ]
        .join(".");
        assert_eq!(longest_host.len(), 253);
        for hosts in [
            json!([]),
            json!(["*"]),
            json!([
                "API.Example.test",
                "*.corp.example.test:8443",
                "localhost:1",
                "192.0.2.1:65535",
                "API.Example.test"
            ]),
            json!([format!("*.{longest_host}:65535")]),
            json!(vec!["*"; MAX_EGRESS_HOSTS]),
        ] {
            let source = bytes(json!({
                "coworkEgressAllowedHosts": hosts,
                "disableDeploymentModeChooser": false,
                "keepAwakeEnabled": true
            }));
            let unchanged = source.clone();
            let mut plan = fixture.plan();
            plan.metadata_before = Some(&meta);
            plan.active_profile = Some(ActiveProfile {
                id: OLD_ID,
                bytes: &source,
            });
            let compiled = compile(&plan).unwrap();
            let profile = after(&compiled.files()[0]);
            assert_eq!(profile["coworkEgressAllowedHosts"], hosts);
            assert_eq!(profile["disableDeploymentModeChooser"], false);
            assert_eq!(profile["keepAwakeEnabled"], true);
            assert_eq!(source, unchanged);
            assert_eq!(
                compiled.active_profile_sha256(),
                Some(sha256(&source).as_str())
            );
            assert!(
                compiled
                    .files()
                    .iter()
                    .all(|file| !file.path.ends_with(&format!("{OLD_ID}.json")))
            );
        }
        let profile = after(&compile(&fixture.plan()).unwrap().files()[0]);
        assert!(profile.get("coworkEgressAllowedHosts").is_none());
        assert!(profile.get("disableDeploymentModeChooser").is_none());
    }

    #[test]
    fn malformed_or_unbounded_existing_egress_lists_fail_closed() {
        let fixture = Fixture::new();
        let meta = bytes(existing_meta());
        let mut invalid = vec![
            Value::Null,
            json!("*"),
            json!(true),
            json!(1),
            json!({"host": "*"}),
            json!([null]),
            json!(["*", true]),
            json!(vec!["*"; MAX_EGRESS_HOSTS + 1]),
            json!(["a".repeat(MAX_EGRESS_HOST_BYTES + 1)]),
            json!([format!("{}.example.test", "a".repeat(64))]),
        ];
        for host in [
            "",
            "https://example.test",
            "user@example.test",
            "user:secret@example.test",
            "example.test/path",
            "example.test?token=secret",
            "example.test#secret",
            "example.test\\secret",
            "example.test%40secret",
            "example.test\n",
            "example.test\u{0}",
            "example.test\u{7f}",
            " example.test",
            "example.test ",
            "éxample.test",
            "**",
            "*.*.example.test",
            "api.*.example.test",
            "*.localhost",
            "example",
            "example..test",
            ".example.test",
            "example.test.",
            "-host.example.test",
            "host-.example.test",
            "host_name.example.test",
            "*:443",
            "example.test:0",
            "example.test:01",
            "example.test:65536",
            "example.test:999999",
            "example.test:",
            "example.test:443:80",
            "[2001:db8::1]",
            "[2001:db8::1]:443",
        ] {
            invalid.push(json!(["safe.example.test", host]));
        }
        for hosts in invalid {
            let source = bytes(json!({"coworkEgressAllowedHosts": hosts}));
            let mut plan = fixture.plan();
            plan.metadata_before = Some(&meta);
            plan.active_profile = Some(ActiveProfile {
                id: OLD_ID,
                bytes: &source,
            });
            rejected(&plan, "claude_configuration_source_unsupported");
        }
    }

    #[test]
    fn switcher_indexes_with_an_empty_selection_and_opaque_inactive_entries_roundtrip() {
        let fixture = Fixture::new();
        for metadata in [br#"{}"#.as_slice(),br#"{ "entries": [] }"#,
            br#"{"appliedId":null,"entries":[{"name":"Archived","id":"other-1","weight":1.000000000000000000001}]}"#] {
            let mut plan = fixture.plan();
            plan.metadata_before = Some(metadata);
            let result = compile(&plan).unwrap();
            let applied = result.files()[1].after.as_deref().unwrap();
            assert_eq!(selected_profile(applied).unwrap().as_deref(), Some(NEW_ID));
            assert_eq!(restore_index(Some(metadata),applied,Some(applied),NEW_ID).unwrap().as_deref(),Some(metadata));
        }
        let switcher_id = "00000000-0000-4000-8000-000000157210";
        let metadata = bytes(
            json!({"appliedId":switcher_id,"entries":[{"id":"other-1","name":"Archived"},{"id":switcher_id,"name":"CC Switch"}]}),
        );
        let source = br#"{"inferenceProvider":"gateway","inferenceGatewayApiKey":"__sealed__:v1:synthetic","disableDeploymentModeChooser":true,"coworkEgressAllowedHosts":["corp.example"]}"#;
        let mut plan = fixture.plan();
        plan.metadata_before = Some(&metadata);
        plan.active_profile = Some(ActiveProfile {
            id: switcher_id,
            bytes: source,
        });
        let result = compile(&plan).unwrap();
        let profile = after(&result.files()[0]);
        assert_eq!(profile["coworkEgressAllowedHosts"], json!(["corp.example"]));
        assert_eq!(profile["disableDeploymentModeChooser"], true);
        assert!(profile.get("inferenceGatewayApiKey").is_none());
        assert_eq!(after(&result.files()[1])["entries"][0]["id"], "other-1");
    }

    #[test]
    fn existing_metadata_requires_exact_readable_active_source_and_fresh_id() {
        let fixture = Fixture::new();
        let meta = bytes(existing_meta());
        let mut plan = fixture.plan();
        plan.metadata_before = Some(&meta);
        rejected(&plan, "claude_configuration_source_unsupported");
        plan.active_profile = Some(ActiveProfile {
            id: INACTIVE_ID,
            bytes: b"{}",
        });
        rejected(&plan, "claude_configuration_source_unsupported");
        plan.active_profile = Some(ActiveProfile {
            id: OLD_ID,
            bytes: b"{",
        });
        rejected(&plan, "claude_configuration_source_unsupported");
        plan.active_profile = Some(ActiveProfile {
            id: OLD_ID,
            bytes: b"{}",
        });
        plan.profile_id = OLD_ID;
        rejected(&plan, "configuration_conflict");
        plan.profile_id = NEW_ID;
        plan.profile_before = Some(b"{}");
        rejected(&plan, "configuration_conflict");
        plan.profile_before = None;
        for value in [
            json!({"appliedId": OLD_ID, "entries": []}),
            json!({"appliedId": OLD_ID, "entries": [{"id": "bad-id", "name": "bad"}]}),
            json!({"appliedId": OLD_ID, "entries": [{"id": OLD_ID, "name": "A"}, {"id": OLD_ID, "name": "B"}]}),
            json!({"appliedId": NEW_ID, "entries": [{"id": OLD_ID, "name": "A"}]}),
        ] {
            let value = bytes(value);
            let mut candidate = fixture.plan();
            candidate.metadata_before = Some(&value);
            candidate.active_profile = Some(ActiveProfile {
                id: OLD_ID,
                bytes: b"{}",
            });
            rejected(&candidate, "claude_configuration_source_unsupported");
        }
        plan.metadata_before = None;
        rejected(&plan, "claude_configuration_source_unsupported");
    }

    #[test]
    fn native_migration_inspects_only_pinned_string_leaves_without_retaining_values() {
        for &key in NATIVE_SEALED_STRING_KEYS {
            assert!(SCHEMA_FLAT_KEYS.contains(&key));
            for (value, required) in [
                ("synthetic-old-secret", true),
                ("", false),
                (" ", true),
                ("__sealed__:v1:synthetic-ciphertext", false),
                ("\u{feff}\t__sealed__:v1:synthetic-ciphertext", false),
                ("\u{0085}__sealed__:v1:synthetic-ciphertext", true),
            ] {
                let bytes = serde_json::to_vec(&json!({key: value})).unwrap();
                assert_eq!(native_migration_required(&bytes, false).unwrap(), required);
            }
        }
        assert!(
            native_migration_required(
                br#"{"inferenceGatewayApi\u004bey":"synthetic-old-secret"}"#,
                false
            )
            .unwrap()
        );
        assert!(
            !native_migration_required(
                br#"{"inferenceCredentialHelper":"/synthetic/helper","other":{"token":"unobserved"}}"#,
                false
            )
            .unwrap()
        );
        for bytes in [
            br#"{"inferenceGatewayApiKey":null}"#.as_slice(),
            br#"{"inferenceGatewayApiKey":42}"#,
            br#"{"inferenceGatewayApiKey":{}}"#,
            br#"{} trailing"#,
            br#"[]"#,
            br#"{"#,
        ] {
            assert_eq!(
                native_migration_required(bytes, false).unwrap_err(),
                fail("claude_configuration_target_unverified")
            );
        }
    }

    #[test]
    fn native_migration_main_checks_enterprise_presence_including_null() {
        for bytes in [
            br#"{"enterpriseConfig":null}"#.as_slice(),
            br#"{"enterpriseConfig":{}}"#,
            br#"{"enterpriseConfig":{"inferenceGatewayApiKey":"unobserved"}}"#,
        ] {
            assert!(native_migration_required(bytes, true).unwrap());
        }
        assert!(
            !native_migration_required(
                br#"{"deploymentMode":"3p","inferenceGatewayApiKey":"not-a-main-migration-leaf"}"#,
                true
            )
            .unwrap()
        );
        assert!(!native_migration_required(br#"{"enterpriseConfig":null}"#, false).unwrap());
    }

    #[test]
    fn fixed_source_inventory_inputs_and_serialized_outputs_are_bounded() {
        assert_eq!(SUPPORTED_APP_VERSION, "4.4212.0");
        assert_eq!(SCHEMA_FLAT_KEYS.len(), 207);
        assert_eq!(
            sha256(&serde_json::to_vec(SCHEMA_FLAT_KEYS).unwrap()),
            SCHEMA_KEYS_SHA256
        );
        assert!(SCHEMA_FLAT_KEYS.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(
            REPLACED_CONNECTION_KEYS
                .iter()
                .all(|key| SCHEMA_FLAT_KEYS.contains(key))
        );
        let fixture = Fixture::new();
        let mut plan = fixture.plan();
        plan.app_version = "999.1.0";
        assert!(compile(&plan).is_ok());
        plan.app_version = SUPPORTED_APP_VERSION;
        plan.source_member_sha256 = "unknown";
        rejected(&plan, "claude_configuration_target_unverified");
        plan.source_member_sha256 = SOURCE_MEMBER_SHA256;
        for gateway in [
            "http://edge.example.test",
            "https://127.0.0.1",
            "https://edge.example.test/?token=value",
        ] {
            plan.gateway_base_url = gateway;
            rejected(&plan, "invalid_configuration");
        }
        plan.gateway_base_url = "https://edge.example.test/anthropic";
        let traversal = fixture.root.join("..").join("Claude-3p");
        plan.user_root = &traversal;
        rejected(&plan, "invalid_configuration");
        plan.user_root = &fixture.root;
        plan.binding_id = "not-a-binding-id";
        rejected(&plan, "invalid_configuration");
        plan.binding_id = BINDING;
        for name in ["helper\"quoted", "helper%expanded"] {
            let helper = fixture.root.join(name);
            let mut candidate = fixture.plan();
            candidate.helper = &helper;
            rejected(&candidate, "invalid_configuration");
        }
        plan.main_before = Some(b"{\"deploymentMode\":\"unknown\"}");
        rejected(&plan, "claude_configuration_source_unsupported");
        let too_large = vec![b' '; MAX_FILE_BYTES + 1];
        plan.main_before = Some(&too_large);
        rejected(&plan, "invalid_configuration");
        // The input fits, but adding the owned deployment mode crosses the output cap.
        let value = bytes(json!({"unknownMain": "x".repeat(MAX_FILE_BYTES - 24)}));
        assert!(value.len() < MAX_FILE_BYTES);
        plan.main_before = Some(&value);
        rejected(&plan, "invalid_configuration");
    }
}
