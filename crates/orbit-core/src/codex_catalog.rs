//! Pure compilation of caller-asserted aliases from one pinned Codex catalog.
//!
//! A successful compilation establishes neither authorization nor upstream
//! capability. Version, source hash, aliases and auxiliary dependencies are caller
//! assertions; this module does not discover the running Desktop executable,
//! establish global auxiliary-model completeness or write client configuration.

use crate::{
    error::{Result, fail},
    identity::sha256,
};
use serde_json::{Map, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{self, Write},
};

pub const SUPPORTED_CLI_VERSION: &str = "0.163.0-alpha.10.2";
pub const SOURCE_COMMIT: &str = "4ebc485ca321243727539187169425fcf972f2c4";
pub const SOURCE_SHA256: &str = "943ca7fe1d19ed019054158303f0dbbd80b3bef43b3aa425a71aa7cb3fb52c2b";
pub const MAX_AUTHORIZED_ALIASES: usize = 256;
pub const MAX_MODELS: usize = 64;
pub const MAX_ALIAS_BYTES: usize = 128;
pub const MAX_DEPENDENCIES: usize = 256;
pub const MAX_REASONING_EFFORTS: usize = 16;
pub const MAX_OUTPUT_BYTES: usize = 1024 * 1024;
const MAX_SOURCE_BYTES: usize = 1024 * 1024;
const SOURCE: &[u8] = include_bytes!("../resources/codex/0.163.0-alpha.10.2/models.json");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelPurpose {
    Main,
    Auxiliary,
}

/// A restriction intersected with the template's advertised reasoning choices.
///
/// Choices retain the official order and complete metadata. If the intersection
/// removes the template default, a replacement must be explicitly supplied.
#[derive(Debug, Clone, Copy)]
pub struct ReasoningConstraint<'a> {
    pub allowed_efforts: &'a [&'a str],
    pub default_effort: Option<&'a str>,
}

#[derive(Debug, Clone, Copy)]
pub struct ModelMapping<'a> {
    pub alias: &'a str,
    pub template_slug: &'a str,
    pub purpose: ModelPurpose,
    /// Explicit dependencies, each backed by an authorized Auxiliary mapping.
    pub auxiliary_aliases: &'a [&'a str],
    pub reasoning: Option<ReasoningConstraint<'a>>,
}

#[derive(Debug)]
pub struct CatalogPlan<'a> {
    pub runtime_cli_version: &'a str,
    pub source_sha256: &'a str,
    /// An asserted allowed superset; unused authorized aliases are permitted.
    pub authorized_aliases: &'a [&'a str],
    /// Exact output entries, including every explicitly required auxiliary.
    pub mappings: &'a [ModelMapping<'a>],
    /// Returned for future configuration use; catalog priorities stay unchanged.
    pub default_alias: &'a str,
}

#[derive(Debug, Clone)]
pub struct CompiledCatalog {
    bytes: Vec<u8>,
    sha256: String,
    default_alias: String,
    main_aliases: Vec<String>,
    auxiliary_aliases: Vec<String>,
}

impl CompiledCatalog {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn sha256(&self) -> &str {
        &self.sha256
    }

    pub fn default_alias(&self) -> &str {
        &self.default_alias
    }

    pub fn main_aliases(&self) -> &[String] {
        &self.main_aliases
    }

    pub fn auxiliary_aliases(&self) -> &[String] {
        &self.auxiliary_aliases
    }
}

/// Compile a catalog without I/O. Unknown versions and source hashes fail closed.
pub fn compile(plan: &CatalogPlan<'_>) -> Result<CompiledCatalog> {
    if plan.runtime_cli_version != SUPPORTED_CLI_VERSION {
        return Err(fail("codex_catalog_version_unverified"));
    }
    if plan.source_sha256 != SOURCE_SHA256
        || SOURCE.len() > MAX_SOURCE_BYTES
        || sha256(SOURCE) != SOURCE_SHA256
    {
        return Err(fail("codex_catalog_source_unverified"));
    }
    let document =
        serde_json::from_slice(SOURCE).map_err(|_| fail("codex_catalog_source_unverified"))?;
    compile_document(plan, document)
}

fn valid_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_ALIAS_BYTES
        && value.bytes().all(|b| b.is_ascii_graphic())
}

fn plan_mappings<'a>(plan: &'a CatalogPlan<'a>) -> Result<BTreeMap<&'a str, &'a ModelMapping<'a>>> {
    if plan.authorized_aliases.len() > MAX_AUTHORIZED_ALIASES || plan.mappings.len() > MAX_MODELS {
        return Err(fail("codex_catalog_limit_exceeded"));
    }
    if plan.mappings.is_empty() || !valid_name(plan.default_alias) {
        return Err(fail("codex_catalog_invalid_plan"));
    }
    let mut authorized = BTreeSet::new();
    for alias in plan.authorized_aliases {
        if !valid_name(alias) || !authorized.insert(*alias) {
            return Err(fail("codex_catalog_invalid_plan"));
        }
    }
    let mut mappings = BTreeMap::new();
    let mut dependencies = 0usize;
    for mapping in plan.mappings {
        if !valid_name(mapping.alias)
            || !valid_name(mapping.template_slug)
            || mappings.insert(mapping.alias, mapping).is_some()
        {
            return Err(fail("codex_catalog_invalid_plan"));
        }
        if !authorized.contains(mapping.alias) {
            return Err(fail("codex_catalog_alias_unauthorized"));
        }
        dependencies = dependencies
            .checked_add(mapping.auxiliary_aliases.len())
            .filter(|total| *total <= MAX_DEPENDENCIES)
            .ok_or_else(|| fail("codex_catalog_limit_exceeded"))?;
        let mut unique = BTreeSet::new();
        for alias in mapping.auxiliary_aliases {
            if !valid_name(alias) || !unique.insert(*alias) {
                return Err(fail("codex_catalog_dependency_invalid"));
            }
        }
        if let Some(reasoning) = mapping.reasoning {
            if reasoning.allowed_efforts.len() > MAX_REASONING_EFFORTS {
                return Err(fail("codex_catalog_limit_exceeded"));
            }
            let mut efforts = BTreeSet::new();
            if reasoning.allowed_efforts.is_empty()
                || reasoning
                    .allowed_efforts
                    .iter()
                    .any(|effort| !valid_name(effort) || !efforts.insert(*effort))
                || reasoning
                    .default_effort
                    .is_some_and(|effort| !valid_name(effort))
            {
                return Err(fail("codex_catalog_reasoning_invalid"));
            }
        }
    }
    if mappings
        .get(plan.default_alias)
        .is_none_or(|mapping| mapping.purpose != ModelPurpose::Main)
    {
        return Err(fail("codex_catalog_default_invalid"));
    }
    Ok(mappings)
}

fn visit<'a>(
    alias: &'a str,
    mappings: &BTreeMap<&'a str, &'a ModelMapping<'a>>,
    visiting: &mut BTreeSet<&'a str>,
    visited: &mut BTreeSet<&'a str>,
) -> Result<()> {
    if visited.contains(alias) {
        return Ok(());
    }
    if !visiting.insert(alias) {
        return Err(fail("codex_catalog_dependency_invalid"));
    }
    let mapping = mappings
        .get(alias)
        .ok_or_else(|| fail("codex_catalog_dependency_invalid"))?;
    for dependency in mapping.auxiliary_aliases {
        if mappings
            .get(dependency)
            .is_none_or(|target| target.purpose != ModelPurpose::Auxiliary)
        {
            return Err(fail("codex_catalog_dependency_invalid"));
        }
        visit(dependency, mappings, visiting, visited)?;
    }
    visiting.remove(alias);
    visited.insert(alias);
    Ok(())
}

fn check_template_dependencies(
    template: &Map<String, Value>,
    mapping: &ModelMapping<'_>,
    mappings: &BTreeMap<&str, &ModelMapping<'_>>,
    templates: &BTreeMap<&str, &Map<String, Value>>,
) -> Result<()> {
    // upgrade.model is intentionally removed. Other known model references are
    // preserved and must name an exact, explicitly declared hidden dependency.
    if let Some(reference) = template
        .get("auto_review_model_override")
        .filter(|value| !value.is_null())
    {
        let reference = reference
            .as_str()
            .filter(|reference| valid_name(reference))
            .ok_or_else(|| fail("codex_catalog_dependency_invalid"))?;
        if !templates.contains_key(reference)
            || !mapping.auxiliary_aliases.contains(&reference)
            || mappings.get(reference).is_none_or(|target| {
                target.purpose != ModelPurpose::Auxiliary || target.template_slug != reference
            })
        {
            return Err(fail("codex_catalog_dependency_invalid"));
        }
    }
    Ok(())
}

fn narrow_reasoning(
    template: &mut Map<String, Value>,
    constraint: Option<ReasoningConstraint<'_>>,
) -> Result<()> {
    let Some(constraint) = constraint else {
        return Ok(());
    };
    let original = template
        .get("supported_reasoning_levels")
        .and_then(Value::as_array)
        .ok_or_else(|| fail("codex_catalog_reasoning_invalid"))?;
    let allowed: BTreeSet<_> = constraint.allowed_efforts.iter().copied().collect();
    let mut retained = Vec::new();
    let mut retained_efforts = BTreeSet::new();
    for option in original {
        let effort = option
            .get("effort")
            .and_then(Value::as_str)
            .ok_or_else(|| fail("codex_catalog_reasoning_invalid"))?;
        if allowed.contains(effort) {
            if !retained_efforts.insert(effort.to_owned()) {
                return Err(fail("codex_catalog_reasoning_invalid"));
            }
            retained.push(option.clone());
        }
    }
    let default = constraint.default_effort.or_else(|| {
        template
            .get("default_reasoning_level")
            .and_then(Value::as_str)
    });
    if retained.is_empty()
        || default.is_none_or(|effort| !retained_efforts.contains(effort))
        || template
            .get("multi_agent_reasoning_effort")
            .filter(|value| !value.is_null())
            .is_some_and(|value| {
                value
                    .as_str()
                    .is_none_or(|effort| !retained_efforts.contains(effort))
            })
    {
        return Err(fail("codex_catalog_reasoning_invalid"));
    }
    let default = default.unwrap().to_owned();
    template.insert("supported_reasoning_levels".into(), retained.into());
    template.insert("default_reasoning_level".into(), default.into());
    Ok(())
}

fn compile_document(plan: &CatalogPlan<'_>, mut document: Value) -> Result<CompiledCatalog> {
    let mappings = plan_mappings(plan)?;
    let source_models = document
        .get("models")
        .and_then(Value::as_array)
        .filter(|models| !models.is_empty() && models.len() <= MAX_MODELS)
        .ok_or_else(|| fail("codex_catalog_source_unverified"))?;
    let mut templates = BTreeMap::new();
    for model in source_models {
        let template = model
            .as_object()
            .ok_or_else(|| fail("codex_catalog_source_unverified"))?;
        let slug = template
            .get("slug")
            .and_then(Value::as_str)
            .filter(|slug| valid_name(slug))
            .ok_or_else(|| fail("codex_catalog_source_unverified"))?;
        if templates.insert(slug, template).is_some() {
            return Err(fail("codex_catalog_source_unverified"));
        }
    }
    let mut visited = BTreeSet::new();
    for mapping in plan
        .mappings
        .iter()
        .filter(|mapping| mapping.purpose == ModelPurpose::Main)
    {
        visit(mapping.alias, &mappings, &mut BTreeSet::new(), &mut visited)?;
    }
    if visited.len() != plan.mappings.len() {
        return Err(fail("codex_catalog_dependency_invalid"));
    }
    let mut compiled = Vec::new();
    let mut main_aliases = Vec::new();
    let mut auxiliary_aliases = Vec::new();
    for mapping in plan.mappings {
        let template = templates
            .get(mapping.template_slug)
            .ok_or_else(|| fail("codex_catalog_template_unavailable"))?;
        check_template_dependencies(template, mapping, &mappings, &templates)?;
        let mut output = (*template).clone();
        let visibility = output
            .get("visibility")
            .and_then(Value::as_str)
            .filter(|visibility| matches!(*visibility, "list" | "hide"))
            .ok_or_else(|| fail("codex_catalog_source_unverified"))?;
        if mapping.alias == plan.default_alias && visibility != "list" {
            return Err(fail("codex_catalog_default_invalid"));
        }
        output.insert("slug".into(), mapping.alias.into());
        output.insert("upgrade".into(), Value::Null);
        narrow_reasoning(&mut output, mapping.reasoning)?;
        match mapping.purpose {
            ModelPurpose::Main => main_aliases.push(mapping.alias.to_owned()),
            ModelPurpose::Auxiliary => {
                output.insert("visibility".into(), "hide".into());
                auxiliary_aliases.push(mapping.alias.to_owned());
            }
        }
        compiled.push(Value::Object(output));
    }
    document
        .as_object_mut()
        .ok_or_else(|| fail("codex_catalog_source_unverified"))?
        .insert("models".into(), compiled.into());
    let bytes = serialize_bounded(&document)?;
    Ok(CompiledCatalog {
        sha256: sha256(&bytes),
        bytes,
        default_alias: plan.default_alias.to_owned(),
        main_aliases,
        auxiliary_aliases,
    })
}

fn serialize_bounded(document: &Value) -> Result<Vec<u8>> {
    struct BoundedWriter(Vec<u8>);
    impl Write for BoundedWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if bytes.len() > MAX_OUTPUT_BYTES.saturating_sub(self.0.len()) {
                return Err(io::Error::other("catalog output limit exceeded"));
            }
            self.0.extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut writer = BoundedWriter(Vec::new());
    serde_json::to_writer(&mut writer, document)
        .map_err(|_| fail("codex_catalog_limit_exceeded"))?;
    Ok(writer.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn source() -> Value {
        serde_json::from_slice(SOURCE).unwrap()
    }

    fn main<'a>(alias: &'a str, template_slug: &'a str) -> ModelMapping<'a> {
        ModelMapping {
            alias,
            template_slug,
            purpose: ModelPurpose::Main,
            auxiliary_aliases: &[],
            reasoning: None,
        }
    }

    fn plan<'a>(
        authorized_aliases: &'a [&'a str],
        mappings: &'a [ModelMapping<'a>],
    ) -> CatalogPlan<'a> {
        CatalogPlan {
            runtime_cli_version: SUPPORTED_CLI_VERSION,
            source_sha256: SOURCE_SHA256,
            authorized_aliases,
            mappings,
            default_alias: "orbit-main",
        }
    }

    fn assert_code(plan: &CatalogPlan<'_>, code: &str) {
        assert_eq!(compile(plan).unwrap_err().code, code);
    }

    #[test]
    fn all_pinned_templates_keep_complete_raw_metadata() {
        let original = source();
        let models = original["models"].as_array().unwrap();
        let aliases: Vec<_> = (0..models.len()).map(|i| format!("orbit-{i}")).collect();
        let authorized: Vec<_> = aliases.iter().map(String::as_str).collect();
        let mappings: Vec<_> = models
            .iter()
            .zip(&aliases)
            .map(|(model, alias)| main(alias, model["slug"].as_str().unwrap()))
            .collect();
        let mut plan = plan(&authorized, &mappings);
        plan.default_alias = &aliases[0];
        let compiled = compile(&plan).unwrap();
        let output: Value = serde_json::from_slice(compiled.bytes()).unwrap();
        let mut expected = original;
        for (model, alias) in expected["models"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .zip(&aliases)
        {
            model["slug"] = alias.clone().into();
            model["upgrade"] = Value::Null;
        }
        assert_eq!(output, expected);
        assert_eq!(compiled.sha256(), sha256(compiled.bytes()));
        assert_eq!(compiled.default_alias(), aliases[0]);
        assert_eq!(compiled.main_aliases(), aliases);
        assert!(compiled.auxiliary_aliases().is_empty());
        assert!(compiled.bytes().len() < MAX_OUTPUT_BYTES);
    }

    #[test]
    fn unknown_fields_and_disagreeing_instruction_formats_survive() {
        let mut document = source();
        document["future_root"] = json!({"nested": [null, {"model": "gpt-6-sol"}]});
        document["models"][0]["future_model"] = json!({"nested": [1, true, {"x": "y"}]});
        document["models"][0]["base_instructions"] = "deliberate legacy disagreement".into();
        document["models"][0]["model_messages"]["future"] = json!([false, {"kept": 1}]);
        let mappings = [main("orbit-main", "gpt-6-astra")];
        let compiled =
            compile_document(&plan(&["orbit-main"], &mappings), document.clone()).unwrap();
        let output: Value = serde_json::from_slice(compiled.bytes()).unwrap();
        document["models"].as_array_mut().unwrap().truncate(1);
        document["models"][0]["slug"] = "orbit-main".into();
        document["models"][0]["upgrade"] = Value::Null;
        assert_eq!(output, document);
    }

    #[test]
    fn unknown_version_and_mismatched_source_fail_closed() {
        let mappings = [main("orbit-main", "gpt-6.1-sol")];
        let mut plan = plan(&["orbit-main"], &mappings);
        plan.runtime_cli_version = "0.163.0-alpha.10.3";
        assert_code(&plan, "codex_catalog_version_unverified");
        plan.runtime_cli_version = SUPPORTED_CLI_VERSION;
        plan.source_sha256 = "943ca7fe";
        assert_code(&plan, "codex_catalog_source_unverified");
    }

    #[test]
    fn allowed_superset_does_not_grant_or_force_an_output_mapping() {
        let mappings = [main("orbit-main", "gpt-6.1-sol")];
        let compiled = compile(&plan(&["orbit-main", "unused"], &mappings)).unwrap();
        assert_eq!(compiled.main_aliases(), ["orbit-main"]);
        assert_code(
            &plan(&["unused"], &mappings),
            "codex_catalog_alias_unauthorized",
        );
        let missing = [main("orbit-main", "unrecognized-template")];
        assert_code(
            &plan(&["orbit-main"], &missing),
            "codex_catalog_template_unavailable",
        );
    }

    #[test]
    fn empty_duplicate_and_unbounded_inputs_are_rejected() {
        let mappings = [main("orbit-main", "gpt-6-sol")];
        for aliases in [
            &["orbit-main", "orbit-main"][..],
            &[""][..],
            &["has space"][..],
        ] {
            assert_code(&plan(aliases, &mappings), "codex_catalog_invalid_plan");
        }
        assert_code(&plan(&[], &[]), "codex_catalog_invalid_plan");
        assert_code(
            &plan(&["orbit-main"], &[mappings[0], mappings[0]]),
            "codex_catalog_invalid_plan",
        );
        let too_long = "x".repeat(MAX_ALIAS_BYTES + 1);
        assert_code(&plan(&[&too_long], &mappings), "codex_catalog_invalid_plan");
        let too_many_aliases = vec!["orbit-main"; MAX_AUTHORIZED_ALIASES + 1];
        assert_code(
            &plan(&too_many_aliases, &mappings),
            "codex_catalog_limit_exceeded",
        );
        let too_many_models = vec![mappings[0]; MAX_MODELS + 1];
        assert_code(
            &plan(&["orbit-main"], &too_many_models),
            "codex_catalog_limit_exceeded",
        );
        let dependencies = vec!["aux"; MAX_DEPENDENCIES + 1];
        let mut mapping = mappings[0];
        mapping.auxiliary_aliases = &dependencies;
        assert_code(
            &plan(&["orbit-main"], &[mapping]),
            "codex_catalog_limit_exceeded",
        );
    }

    #[test]
    fn explicit_auxiliary_closure_is_authorized_and_hidden() {
        let mut root = main("orbit-main", "gpt-6-sol");
        root.auxiliary_aliases = &["aux"];
        let mut auxiliary = main("aux", "gpt-6-luna");
        auxiliary.purpose = ModelPurpose::Auxiliary;
        let mappings = [root, auxiliary];
        let compiled = compile(&plan(&["orbit-main", "aux"], &mappings)).unwrap();
        let output: Value = serde_json::from_slice(compiled.bytes()).unwrap();
        let mut expected = source()["models"][3].clone();
        expected["slug"] = "aux".into();
        expected["upgrade"] = Value::Null;
        expected["visibility"] = "hide".into();
        assert_eq!(output["models"][1], expected);
        assert_eq!(compiled.auxiliary_aliases(), ["aux"]);
        assert_code(
            &plan(&["orbit-main"], &mappings),
            "codex_catalog_alias_unauthorized",
        );
    }

    #[test]
    fn missing_misclassified_unused_cyclic_and_duplicate_auxiliaries_fail() {
        let authorized = ["orbit-main", "aux"];
        let mut root = main("orbit-main", "gpt-6-sol");
        root.auxiliary_aliases = &["aux"];
        assert_code(
            &plan(&authorized, &[root]),
            "codex_catalog_dependency_invalid",
        );
        let mut auxiliary = main("aux", "gpt-6-luna");
        assert_code(
            &plan(&authorized, &[root, auxiliary]),
            "codex_catalog_dependency_invalid",
        );
        auxiliary.purpose = ModelPurpose::Auxiliary;
        root.auxiliary_aliases = &[];
        assert_code(
            &plan(&authorized, &[root, auxiliary]),
            "codex_catalog_dependency_invalid",
        );
        root.auxiliary_aliases = &["aux"];
        auxiliary.auxiliary_aliases = &["aux"];
        assert_code(
            &plan(&authorized, &[root, auxiliary]),
            "codex_catalog_dependency_invalid",
        );
        root.auxiliary_aliases = &["aux", "aux"];
        auxiliary.auxiliary_aliases = &[];
        assert_code(
            &plan(&authorized, &[root, auxiliary]),
            "codex_catalog_dependency_invalid",
        );
    }

    #[test]
    fn preserved_auto_review_reference_requires_an_exact_declared_target() {
        let mut document = source();
        document["models"][0]["auto_review_model_override"] = "codex-auto-review".into();
        let authorized = ["orbit-main", "codex-auto-review"];
        let mut root = main("orbit-main", "gpt-6-astra");
        let mut auxiliary = main("codex-auto-review", "codex-auto-review");
        auxiliary.purpose = ModelPurpose::Auxiliary;
        // Reachability alone cannot satisfy the template's undeclared edge.
        let mut other = main("other", "gpt-6-luna");
        other.auxiliary_aliases = &["codex-auto-review"];
        let missing_edge = [root, other, auxiliary];
        assert_eq!(
            compile_document(
                &plan(&["orbit-main", "other", "codex-auto-review"], &missing_edge),
                document.clone(),
            )
            .unwrap_err()
            .code,
            "codex_catalog_dependency_invalid"
        );
        root.auxiliary_aliases = &["codex-auto-review"];
        auxiliary.template_slug = "gpt-6-luna";
        assert_eq!(
            compile_document(&plan(&authorized, &[root, auxiliary]), document.clone())
                .unwrap_err()
                .code,
            "codex_catalog_dependency_invalid"
        );
        auxiliary.template_slug = "codex-auto-review";
        let compiled = compile_document(&plan(&authorized, &[root, auxiliary]), document).unwrap();
        let output: Value = serde_json::from_slice(compiled.bytes()).unwrap();
        assert_eq!(
            output["models"][0]["auto_review_model_override"],
            "codex-auto-review"
        );
        assert_eq!(output["models"][1]["visibility"], "hide");
    }

    #[test]
    fn hidden_or_auxiliary_defaults_are_rejected_without_promoting_hidden_templates() {
        let hidden = [main("orbit-main", "gpt-daybreak-blue-latest")];
        assert_code(
            &plan(&["orbit-main"], &hidden),
            "codex_catalog_default_invalid",
        );
        let mut root = main("orbit-main", "gpt-6-sol");
        root.auxiliary_aliases = &["aux"];
        let mut auxiliary = main("aux", "gpt-6-luna");
        auxiliary.purpose = ModelPurpose::Auxiliary;
        let mappings = [root, auxiliary];
        let mut invalid = plan(&["orbit-main", "aux"], &mappings);
        invalid.default_alias = "aux";
        assert_code(&invalid, "codex_catalog_default_invalid");
        let mappings = [root, auxiliary, main("hidden", "gpt-daybreak-blue-latest")];
        let compiled = compile(&plan(&["orbit-main", "aux", "hidden"], &mappings)).unwrap();
        let output: Value = serde_json::from_slice(compiled.bytes()).unwrap();
        assert_eq!(output["models"][2]["visibility"], "hide");
    }

    #[test]
    fn reasoning_intersection_keeps_original_options_and_multi_agent_override() {
        let mut document = source();
        document["models"][0]["supported_reasoning_levels"][0]["future"] = json!({"kept": true});
        let mut root = main("orbit-main", "gpt-6-astra");
        root.reasoning = Some(ReasoningConstraint {
            allowed_efforts: &["xhigh", "unadvertised", "low"],
            default_effort: Some("xhigh"),
        });
        let compiled = compile_document(&plan(&["orbit-main"], &[root]), document.clone()).unwrap();
        let output: Value = serde_json::from_slice(compiled.bytes()).unwrap();
        let mut expected = document["models"][0].clone();
        expected["slug"] = "orbit-main".into();
        expected["upgrade"] = Value::Null;
        expected["default_reasoning_level"] = "xhigh".into();
        expected["supported_reasoning_levels"] = json!([
            expected["supported_reasoning_levels"][0],
            expected["supported_reasoning_levels"][3]
        ]);
        assert_eq!(output["models"][0], expected);
    }

    #[test]
    fn zero_intersection_lost_defaults_and_lost_multi_agent_effort_fail() {
        for (allowed, default) in [
            (&[][..], None),
            (&["unadvertised"][..], None),
            (&["xhigh"][..], None),
            (&["low"][..], None),
            (&["low", "xhigh"][..], Some("high")),
            (&["low", "low", "xhigh"][..], None),
        ] {
            let mut root = main("orbit-main", "gpt-6-astra");
            root.reasoning = Some(ReasoningConstraint {
                allowed_efforts: allowed,
                default_effort: default,
            });
            assert_code(
                &plan(&["orbit-main"], &[root]),
                "codex_catalog_reasoning_invalid",
            );
        }
        let efforts = vec!["low"; MAX_REASONING_EFFORTS + 1];
        let mut root = main("orbit-main", "gpt-6-astra");
        root.reasoning = Some(ReasoningConstraint {
            allowed_efforts: &efforts,
            default_effort: None,
        });
        assert_code(
            &plan(&["orbit-main"], &[root]),
            "codex_catalog_limit_exceeded",
        );
        root.reasoning = Some(ReasoningConstraint {
            allowed_efforts: &["xhigh"],
            default_effort: Some("xhigh"),
        });
        assert!(compile(&plan(&["orbit-main"], &[root])).is_ok());
    }

    #[test]
    fn output_limit_is_enforced_during_serialization() {
        let mut document = source();
        document["models"][0]["future"] = "x".repeat(MAX_OUTPUT_BYTES).into();
        let mappings = [main("orbit-main", "gpt-6-astra")];
        assert_eq!(
            compile_document(&plan(&["orbit-main"], &mappings), document)
                .unwrap_err()
                .code,
            "codex_catalog_limit_exceeded"
        );
        assert_eq!(
            serialize_bounded(&json!("x".repeat(MAX_OUTPUT_BYTES - 2)))
                .unwrap()
                .len(),
            MAX_OUTPUT_BYTES
        );
    }
}
