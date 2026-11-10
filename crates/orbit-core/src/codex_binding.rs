//! Normalize Panel declarations and plan a bounded catalog without I/O.
//!
//! The caller supplies the current actor/node-filtered model projection. This
//! module cannot establish a grant, upstream capability, runtime executable
//! selection, organization policy, or actual client loading.

use crate::{
    codex_catalog::{
        self, CatalogPlan, CompiledCatalog, ModelMapping, ModelPurpose, ReasoningConstraint,
    },
    dto::{CodexCatalogBindingView, ModelView, ReasoningPolicyView},
    error::{Result, fail},
    identity::{canonical_origin, sha256},
};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::BTreeSet;

pub const PROFILE_ID: &str = "codex-cli-0.163.0-alpha.10.2";
const MAIN_TEMPLATES: [&str; 8] = [
    "gpt-6-astra",
    "gpt-6.1-sol",
    "gpt-6-sol",
    "gpt-6-luna",
    "gpt-5.6-sol",
    "gpt-5.6-terra",
    "gpt-5.6-luna",
    "gpt-5.5",
];

fn alias_valid(alias: &str) -> bool {
    !alias.is_empty()
        && alias.len() <= codex_catalog::MAX_ALIAS_BYTES
        && alias
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-'))
}

fn binding_template(binding: &CodexCatalogBindingView) -> Option<&str> {
    match binding {
        CodexCatalogBindingView::Declared {
            verified: false,
            profile_id,
            cli_version,
            source_sha256,
            template_slug,
        } if profile_id == PROFILE_ID
            && cli_version == codex_catalog::SUPPORTED_CLI_VERSION
            && source_sha256 == codex_catalog::SOURCE_SHA256
            && MAIN_TEMPLATES.contains(&template_slug.as_str()) =>
        {
            Some(template_slug)
        }
        _ => None,
    }
}

/// Invalid optional declarations remain unavailable for catalog use, without
/// suppressing the model's other uses or returning the invalid original value.
pub fn normalize_binding(
    value: Option<&Value>,
    alias: &str,
    category: &str,
    capabilities: &[String],
) -> Option<CodexCatalogBindingView> {
    let value = value.filter(|value| !value.is_null())?;
    let bounded = value.as_object().is_some_and(|object| object.len() == 6)
        && value["state"] == "declared"
        && value["verified"] == false
        && value["profile_id"] == PROFILE_ID
        && value["cli_version"] == codex_catalog::SUPPORTED_CLI_VERSION
        && value["source_sha256"] == codex_catalog::SOURCE_SHA256
        && value["template_slug"]
            .as_str()
            .is_some_and(|template| MAIN_TEMPLATES.contains(&template));
    if bounded
        && category == "text"
        && alias_valid(alias)
        && capabilities
            .iter()
            .any(|capability| capability == "openai.responses")
    {
        serde_json::from_value(value.clone()).ok()
    } else {
        Some(CodexCatalogBindingView::Unavailable { verified: false })
    }
}

fn effort_valid(effort: &str) -> bool {
    !effort.is_empty()
        && effort.len() <= codex_catalog::MAX_ALIAS_BYTES
        && effort.bytes().all(|c| c.is_ascii_graphic())
}

fn policy_valid(policy: &ReasoningPolicyView) -> bool {
    let valid_list = |values: &[String]| {
        values.len() <= codex_catalog::MAX_REASONING_EFFORTS
            && values.iter().all(|value| effort_valid(value))
            && values.iter().collect::<BTreeSet<_>>().len() == values.len()
    };
    valid_list(&policy.supported_efforts)
        && policy.allowed_efforts.as_ref().is_none_or(|allowed| {
            valid_list(allowed)
                && allowed
                    .iter()
                    .all(|effort| policy.supported_efforts.contains(effort))
        })
        && matches!(
            (policy.state.as_str(), &policy.allowed_efforts),
            ("unrestricted", None) | ("restricted", Some(_))
        )
        && match policy.capability_status.as_str() {
            "supported" => !policy.supported_efforts.is_empty(),
            "unknown" | "not_applicable" => {
                policy.supported_efforts.is_empty() && policy.default_effort.is_none()
            }
            _ => false,
        }
        && policy
            .default_effort
            .as_ref()
            .is_none_or(|effort| effort_valid(effort) && policy.supported_efforts.contains(effort))
}

pub fn normalize_reasoning(value: Option<&Value>) -> Option<ReasoningPolicyView> {
    let value = value.filter(|value| value.as_object().is_some_and(|object| object.len() == 6))?;
    let bounded_list = |value: &Value| {
        value.as_array().is_some_and(|array| {
            array.len() <= codex_catalog::MAX_REASONING_EFFORTS
                && array
                    .iter()
                    .all(|value| value.as_str().is_some_and(effort_valid))
        })
    };
    if !matches!(value["state"].as_str(), Some("unrestricted" | "restricted"))
        || !matches!(
            value["capability_status"].as_str(),
            Some("supported" | "unknown" | "not_applicable")
        )
        || !bounded_list(&value["supported_efforts"])
        || !(value["allowed_efforts"].is_null() || bounded_list(&value["allowed_efforts"]))
        || !(value["default_effort"].is_null()
            || value["default_effort"].as_str().is_some_and(effort_valid))
        || !(value["revision"].is_null() || value["revision"].as_u64().is_some())
    {
        return None;
    }
    let policy: ReasoningPolicyView = serde_json::from_value(value.clone()).ok()?;
    policy_valid(&policy).then_some(policy)
}

/// All identity and revision fields are caller assertions, never UI inputs.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct ScopeContext<'a> {
    pub panel_origin: &'a str,
    pub account_id: &'a str,
    pub installation_id: &'a str,
    pub connection_id: &'a str,
    pub connection_revision: u64,
    pub authorization_epoch: u64,
    pub credential_revision: &'a str,
    pub node_id: &'a str,
    pub provider_id: &'a str,
    pub catalog_revision: &'a str,
}

pub struct CatalogScope<'a> {
    pub context: ScopeContext<'a>,
    pub models: &'a [ModelView],
}

#[derive(Debug)]
pub struct PlannedCatalog {
    catalog: CompiledCatalog,
    scope_revision: String,
    default_reasoning_effort: String,
}
impl PlannedCatalog {
    pub fn catalog(&self) -> &CompiledCatalog {
        &self.catalog
    }
    pub fn scope_revision(&self) -> &str {
        &self.scope_revision
    }
    pub fn default_reasoning_effort(&self) -> &str {
        &self.default_reasoning_effort
    }
}

fn effective_efforts(policy: &ReasoningPolicyView) -> Option<Vec<&str>> {
    if !policy_valid(policy)
        || policy.capability_status != "supported"
        || policy.default_effort.is_none()
    {
        return None;
    }
    Some(
        policy
            .supported_efforts
            .iter()
            .filter(|effort| {
                policy
                    .allowed_efforts
                    .as_ref()
                    .is_none_or(|allowed| allowed.contains(effort))
            })
            .map(String::as_str)
            .collect(),
    )
}

fn mapping<'a>(
    alias: &'a str,
    template: &'a str,
    efforts: &'a [&'a str],
    default: Option<&'a str>,
) -> ModelMapping<'a> {
    ModelMapping {
        alias,
        template_slug: template,
        purpose: ModelPurpose::Main,
        auxiliary_aliases: &[],
        reasoning: Some(ReasoningConstraint {
            allowed_efforts: efforts,
            default_effort: default,
        }),
    }
}

/// Preserve Panel order and include every compatible explicit main binding in
/// the supplied scope. Unknown/incompatible entries are omitted; the selected
/// default must remain eligible. Limits reject the whole plan, never truncate.
pub fn compile_scope(scope: &CatalogScope<'_>, selected: &str) -> Result<PlannedCatalog> {
    let context = &scope.context;
    if scope.models.len() > codex_catalog::MAX_AUTHORIZED_ALIASES {
        return Err(fail("codex_catalog_limit_exceeded"));
    }
    let opaque = |value: &str| {
        !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
    };
    if ![
        context.account_id,
        context.installation_id,
        context.connection_id,
        context.credential_revision,
        context.node_id,
        context.provider_id,
        context.catalog_revision,
    ]
    .into_iter()
    .all(opaque)
        || context.panel_origin.len() > 2048
        || !canonical_origin(context.panel_origin)
            .is_ok_and(|origin| origin == context.panel_origin)
    {
        return Err(fail("codex_catalog_scope_invalid"));
    }
    let scoped: Vec<_> = scope
        .models
        .iter()
        .filter(|model| {
            model.provider_id == context.provider_id
                && model.category == "text"
                && model
                    .capabilities
                    .iter()
                    .any(|capability| capability == "openai.responses")
        })
        .collect();
    let mut unique = BTreeSet::new();
    let mut authorized = Vec::new();
    let mut prepared = Vec::new();
    let mut selected_error = "codex_binding_unavailable";
    let mut selected_default = None;
    let mut default_alias = selected;
    for model in &scoped {
        if !unique.insert(model.id.as_str()) {
            return Err(fail("codex_catalog_scope_invalid"));
        }
        if !alias_valid(&model.id) {
            continue;
        }
        authorized.push(model.id.as_str());
        let Some(template) = model
            .codex_catalog_binding
            .as_ref()
            .and_then(binding_template)
        else {
            continue;
        };
        let Some((policy, efforts)) = model
            .reasoning
            .as_ref()
            .and_then(|policy| effective_efforts(policy).map(|efforts| (policy, efforts)))
        else {
            if model.id == selected {
                selected_error = "codex_reasoning_unavailable";
            }
            continue;
        };
        // Use the compiler's exact default/multi-agent rules rather than
        // approximating compatibility or silently choosing another effort.
        let entry = [mapping(
            &model.id,
            template,
            &efforts,
            policy.default_effort.as_deref(),
        )];
        let one = codex_catalog::compile(&CatalogPlan {
            runtime_cli_version: codex_catalog::SUPPORTED_CLI_VERSION,
            source_sha256: codex_catalog::SOURCE_SHA256,
            authorized_aliases: &[&model.id],
            mappings: &entry,
            default_alias: &model.id,
        });
        match one {
            Ok(one) => {
                if model.id == default_alias || default_alias.is_empty() {
                    default_alias = &model.id;
                    let output: Value = serde_json::from_slice(one.bytes())
                        .map_err(|_| fail("codex_catalog_source_unverified"))?;
                    selected_default = Some(
                        output["models"][0]["default_reasoning_level"]
                            .as_str()
                            .ok_or_else(|| fail("codex_catalog_reasoning_invalid"))?
                            .to_owned(),
                    );
                }
            }
            Err(error) if error.code == "codex_catalog_reasoning_invalid" => {
                if model.id == selected {
                    return Err(error);
                }
                continue;
            }
            Err(error) => return Err(error),
        }
        prepared.push((
            model.id.as_str(),
            template,
            efforts,
            policy.default_effort.as_deref(),
        ));
        if prepared.len() > codex_catalog::MAX_MODELS {
            return Err(fail("codex_catalog_limit_exceeded"));
        }
    }
    let default_reasoning_effort = selected_default.ok_or_else(|| fail(selected_error))?;
    let mappings: Vec<_> = prepared
        .iter()
        .map(|(alias, template, efforts, default)| mapping(alias, template, efforts, *default))
        .collect();
    let catalog = codex_catalog::compile(&CatalogPlan {
        runtime_cli_version: codex_catalog::SUPPORTED_CLI_VERSION,
        source_sha256: codex_catalog::SOURCE_SHA256,
        authorized_aliases: &authorized,
        mappings: &mappings,
        default_alias,
    })?;
    let inputs: Vec<_> = scoped.iter().map(|model| json!({
        "alias": model.id, "provider_id": model.provider_id,
        "category": model.category, "capabilities": model.capabilities, "revision": model.revision,
        "binding": model.codex_catalog_binding, "reasoning": model.reasoning,
    })).collect();
    let scope_revision = sha256(
        &serde_json::to_vec(&(
            PROFILE_ID,
            codex_catalog::SOURCE_SHA256,
            context,
            default_alias,
            catalog.sha256(),
            inputs,
        ))
        .map_err(|_| fail("serialization_failed"))?,
    );
    Ok(PlannedCatalog {
        catalog,
        scope_revision,
        default_reasoning_effort,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn declaration(template: &str) -> Value {
        json!({"state":"declared","verified":false,"profile_id":PROFILE_ID,
            "cli_version":codex_catalog::SUPPORTED_CLI_VERSION,
            "source_sha256":codex_catalog::SOURCE_SHA256,"template_slug":template})
    }
    fn policy() -> ReasoningPolicyView {
        ReasoningPolicyView {
            state: "unrestricted".into(),
            capability_status: "supported".into(),
            supported_efforts: ["low", "medium", "high", "xhigh"].map(str::to_owned).into(),
            allowed_efforts: None,
            default_effort: Some("medium".into()),
            revision: Some(7),
        }
    }
    fn model(alias: &str, template: &str) -> ModelView {
        ModelView {
            id: alias.into(),
            label: alias.into(),
            provider_id: "openai".into(),
            revision: format!("revision-{alias}"),
            category: "text".into(),
            capabilities: vec!["openai.responses".into()],
            execution_profile: None,
            image_billing_mode: None,
            reasoning_efforts: policy().supported_efforts,
            reasoning: Some(policy()),
            codex_catalog_binding: serde_json::from_value(declaration(template)).unwrap(),
            support_status: "unverified".into(),
            image_parameters: None,
            sales_price: None,
            official_price: None,
        }
    }
    fn scope(models: &[ModelView]) -> CatalogScope<'_> {
        CatalogScope {
            context: ScopeContext {
                panel_origin: "https://panel.example",
                account_id: "actor-a",
                installation_id: "install-a",
                connection_id: "connection-a",
                connection_revision: 2,
                authorization_epoch: 3,
                credential_revision: "credential-a",
                node_id: "node-a",
                provider_id: "openai",
                catalog_revision: "catalog-a",
            },
            models,
        }
    }
    fn code(models: &[ModelView], selected: &str) -> String {
        compile_scope(&scope(models), selected).unwrap_err().code
    }

    #[test]
    fn optional_wire_binding_is_exact_and_never_echoes_invalid_values() {
        let caps = vec!["openai.responses".into()];
        assert!(normalize_binding(None, "alias", "text", &caps).is_none());
        assert!(normalize_binding(Some(&Value::Null), "alias", "text", &caps).is_none());
        let valid = declaration("gpt-5.5");
        assert_eq!(
            serde_json::to_value(normalize_binding(Some(&valid), "alias", "text", &caps)).unwrap(),
            valid
        );
        for (field, invalid) in [
            ("profile_id", json!("unknown")),
            ("cli_version", json!("later")),
            ("source_sha256", json!("different")),
            ("template_slug", json!("codex-auto-review")),
            ("verified", json!(true)),
            ("future", json!("do-not-echo")),
        ] {
            let mut bad = valid.clone();
            bad[field] = invalid;
            assert_eq!(
                serde_json::to_value(normalize_binding(Some(&bad), "alias", "text", &caps))
                    .unwrap(),
                json!({"state":"unavailable","verified":false})
            );
        }
        for (alias, category, capabilities) in [
            ("bad alias", "text", caps.clone()),
            ("alias", "image", caps.clone()),
            ("alias", "text", vec![]),
        ] {
            assert_eq!(
                serde_json::to_value(normalize_binding(
                    Some(&valid),
                    alias,
                    category,
                    &capabilities
                ))
                .unwrap(),
                json!({"state":"unavailable","verified":false})
            );
        }
    }

    #[test]
    fn reasoning_keeps_real_nullable_integer_revision_and_rejects_malformed_policy() {
        let mut wire = serde_json::to_value(policy()).unwrap();
        for revision in [Value::Null, json!(7)] {
            wire["revision"] = revision.clone();
            assert_eq!(
                serde_json::to_value(normalize_reasoning(Some(&wire))).unwrap(),
                wire
            );
        }
        for (field, invalid) in [
            ("revision", json!("legacy-hash")),
            ("revision", json!(-1)),
            ("state", json!("unconfigured")),
            ("capability_status", json!("future")),
            ("allowed_efforts", json!([])),
            ("supported_efforts", json!(["low", "low"])),
            ("future", json!(true)),
        ] {
            let mut bad = wire.clone();
            bad[field] = invalid;
            assert!(normalize_reasoning(Some(&bad)).is_none());
        }
        wire["capability_status"] = "unknown".into();
        wire["supported_efforts"] = json!([]);
        wire["default_effort"] = Value::Null;
        assert_eq!(
            normalize_reasoning(Some(&wire)).unwrap().capability_status,
            "unknown"
        );
    }

    #[test]
    fn all_eligible_aliases_keep_panel_order_and_the_selected_explicit_default() {
        let mut first = model("first", "gpt-6-astra");
        let policy = first.reasoning.as_mut().unwrap();
        policy.state = "restricted".into();
        policy.allowed_efforts = Some(vec!["xhigh".into(), "low".into()]);
        policy.default_effort = Some("xhigh".into());
        let mut unbound = model("unbound", "gpt-5.5");
        unbound.codex_catalog_binding = None;
        let mut other = model("other", "gpt-5.5");
        other.provider_id = "other".into();
        let mut image = model("image", "gpt-5.5");
        image.category = "image".into();
        let models = [first, unbound, other, image, model("last", "gpt-5.5")];
        let planned = compile_scope(&scope(&models), "last").unwrap();
        let output: Value = serde_json::from_slice(planned.catalog().bytes()).unwrap();
        assert_eq!(planned.catalog().main_aliases(), ["first", "last"]);
        assert!(planned.catalog().auxiliary_aliases().is_empty());
        assert_eq!(planned.catalog().default_alias(), "last");
        assert_eq!(planned.default_reasoning_effort(), "medium");
        assert_eq!(output["models"][0]["default_reasoning_level"], "xhigh");
        assert_eq!(
            output["models"][0]["supported_reasoning_levels"]
                .as_array()
                .unwrap()
                .iter()
                .map(|value| value["effort"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["low", "xhigh"]
        );
        assert_eq!(code(&models, "unbound"), "codex_binding_unavailable");
        assert_eq!(code(&models, "other"), "codex_binding_unavailable");
    }

    #[test]
    fn unknown_and_unconfigured_reasoning_never_become_template_support() {
        for missing in [true, false] {
            let mut invalid = model("unknown", "gpt-5.5");
            if missing {
                invalid.reasoning = None;
            } else {
                let policy = invalid.reasoning.as_mut().unwrap();
                policy.capability_status = "unknown".into();
                policy.supported_efforts.clear();
                policy.default_effort = None;
            }
            let models = [model("good", "gpt-5.5"), invalid];
            assert_eq!(
                compile_scope(&scope(&models), "good")
                    .unwrap()
                    .catalog()
                    .main_aliases(),
                ["good"]
            );
            assert_eq!(code(&models, "unknown"), "codex_reasoning_unavailable");
        }
    }

    #[test]
    fn unknown_default_never_inherits_an_otherwise_allowed_template_default() {
        let mut unknown = model("unknown", "gpt-5.5");
        unknown.reasoning.as_mut().unwrap().default_effort = None;
        // medium is still supported and unrestricted, but Panel has not
        // declared a default. The compiler's general fallback is not evidence.
        let models = [model("good", "gpt-5.5"), unknown];
        assert_eq!(code(&models, "unknown"), "codex_reasoning_unavailable");
        assert_eq!(
            compile_scope(&scope(&models), "good")
                .unwrap()
                .catalog()
                .main_aliases(),
            ["good"]
        );
    }

    #[test]
    fn incompatible_defaults_and_multi_agent_efforts_are_excluded_without_fallback() {
        let mut invalid = model("invalid", "gpt-5.5");
        let policy = invalid.reasoning.as_mut().unwrap();
        policy.state = "restricted".into();
        policy.allowed_efforts = Some(vec!["high".into()]);
        assert_eq!(
            code(&[invalid.clone()], "invalid"),
            "codex_catalog_reasoning_invalid"
        );
        invalid.reasoning.as_mut().unwrap().default_effort = None;
        assert_eq!(
            code(&[invalid.clone()], "invalid"),
            "codex_reasoning_unavailable"
        );
        invalid.reasoning.as_mut().unwrap().default_effort = Some("high".into());
        assert_eq!(
            compile_scope(&scope(&[invalid]), "invalid")
                .unwrap()
                .default_reasoning_effort(),
            "high"
        );
        let mut lost_multi_agent = model("lost", "gpt-6-astra");
        let policy = lost_multi_agent.reasoning.as_mut().unwrap();
        policy.state = "restricted".into();
        policy.allowed_efforts = Some(vec!["low".into()]);
        policy.default_effort = Some("low".into());
        let models = [model("good", "gpt-5.5"), lost_multi_agent];
        assert_eq!(
            compile_scope(&scope(&models), "good")
                .unwrap()
                .catalog()
                .main_aliases(),
            ["good"]
        );
        assert_eq!(code(&models, "lost"), "codex_catalog_reasoning_invalid");
    }

    #[test]
    fn cached_declarations_are_revalidated_and_old_model_dtos_stay_readable() {
        let original = model("alias", "gpt-5.5");
        let mut wire = serde_json::to_value(&original).unwrap();
        wire.as_object_mut()
            .unwrap()
            .remove("codex_catalog_binding");
        wire.as_object_mut().unwrap().remove("reasoning");
        let old: ModelView = serde_json::from_value(wire).unwrap();
        assert!(old.codex_catalog_binding.is_none() && old.reasoning.is_none());
        assert_eq!(old.reasoning_efforts, original.reasoning_efforts);
        let mut invalid = original;
        invalid.codex_catalog_binding = Some(CodexCatalogBindingView::Declared {
            verified: true,
            profile_id: PROFILE_ID.into(),
            cli_version: codex_catalog::SUPPORTED_CLI_VERSION.into(),
            source_sha256: codex_catalog::SOURCE_SHA256.into(),
            template_slug: "gpt-5.5".into(),
        });
        assert_eq!(code(&[invalid], "alias"), "codex_binding_unavailable");
    }

    #[test]
    fn fingerprint_binds_all_candidates_scope_and_output_not_only_selected_model() {
        let models = [model("selected", "gpt-5.5"), model("menu", "gpt-5.5")];
        let baseline = compile_scope(&scope(&models), "selected").unwrap();
        let mut late = models.clone();
        late[1].revision = "late".into();
        let changed = compile_scope(&scope(&late), "selected").unwrap();
        assert_eq!(baseline.catalog().sha256(), changed.catalog().sha256());
        assert_ne!(baseline.scope_revision(), changed.scope_revision());
        late[1].codex_catalog_binding = None;
        let changed = compile_scope(&scope(&late), "selected").unwrap();
        assert_ne!(baseline.catalog().sha256(), changed.catalog().sha256());
        assert_ne!(baseline.scope_revision(), changed.scope_revision());
        for field in 0..8 {
            let mut current = scope(&models);
            match field {
                0 => current.context.account_id = "actor-b",
                1 => current.context.installation_id = "install-b",
                2 => current.context.connection_revision += 1,
                3 => current.context.authorization_epoch += 1,
                4 => current.context.credential_revision = "credential-b",
                5 => current.context.node_id = "node-b",
                6 => current.context.catalog_revision = "catalog-b",
                _ => current.context.panel_origin = "https://other.example",
            }
            assert_ne!(
                baseline.scope_revision(),
                compile_scope(&current, "selected")
                    .unwrap()
                    .scope_revision()
            );
        }
    }

    #[test]
    fn ambiguous_scope_and_all_resource_limits_fail_the_whole_plan() {
        let entry = model("alias", "gpt-5.5");
        assert_eq!(
            code(&[entry.clone(), entry], "alias"),
            "codex_catalog_scope_invalid"
        );
        let authorized: Vec<_> = (0..=codex_catalog::MAX_AUTHORIZED_ALIASES)
            .map(|i| model(&format!("model-{i}"), "gpt-5.5"))
            .collect();
        assert_eq!(code(&authorized, "model-0"), "codex_catalog_limit_exceeded");
        let mapped: Vec<_> = (0..=codex_catalog::MAX_MODELS)
            .map(|i| model(&format!("model-{i}"), "gpt-5.5"))
            .collect();
        assert_eq!(code(&mapped, "model-0"), "codex_catalog_limit_exceeded");
        let large: Vec<_> = (0..codex_catalog::MAX_MODELS)
            .map(|i| model(&format!("model-{i}"), "gpt-6-astra"))
            .collect();
        assert_eq!(code(&large, "model-0"), "codex_catalog_limit_exceeded");
        let single = [model("alias", "gpt-5.5")];
        let mut invalid = scope(&single);
        invalid.context.account_id = "";
        assert_eq!(
            compile_scope(&invalid, "alias").unwrap_err().code,
            "codex_catalog_scope_invalid"
        );
    }
}
