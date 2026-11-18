//! The default Codex route is shared with other provider switchers. Work on the
//! effective TOML fields and keep the byte-level snapshots in the journal.
use crate::{
    config::MAX_CONFIG_BYTES,
    error::{Result, fail},
    identity::{sha256, validate_uuid},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};
use toml_edit::{DocumentMut, Item, Table, TableLike, Value, value};

const PROVIDER: &str = "orivelle_orbit";
pub(crate) const ROUTING: [&str; 4] = [
    "model",
    "model_provider",
    "model_catalog_json",
    "model_reasoning_effort",
];

pub(crate) fn document(bytes: Option<&[u8]>) -> Result<DocumentMut> {
    let text = std::str::from_utf8(bytes.unwrap_or_default())
        .map_err(|_| fail("codex_configuration_invalid"))?;
    text.trim_start_matches('\u{feff}')
        .parse()
        .map_err(|_| fail("codex_configuration_invalid"))
}

fn render(doc: &DocumentMut, source: Option<&[u8]>) -> Result<Vec<u8>> {
    let source = source.unwrap_or_default();
    let mut text = doc.to_string();
    if source.windows(2).any(|pair| pair == b"\r\n")
        && !source
            .split_inclusive(|&b| b == b'\n')
            .any(|line| line.ends_with(b"\n") && !line.ends_with(b"\r\n"))
    {
        text = text.replace("\r\n", "\n").replace('\n', "\r\n");
    }
    if source.starts_with(&[0xef, 0xbb, 0xbf]) {
        text.insert(0, '\u{feff}');
    }
    if text.len() as u64 > MAX_CONFIG_BYTES {
        return Err(fail("invalid_configuration"));
    }
    Ok(text.into_bytes())
}

fn active_profile(doc: &DocumentMut) -> Result<Option<&str>> {
    let Some(selector) = doc.get("profile") else {
        return Ok(None);
    };
    let name = selector
        .as_str()
        .ok_or_else(|| fail("codex_config_override_unverified"))?;
    doc.get("profiles")
        .and_then(|profiles| profiles.get(name))
        .and_then(Item::as_table_like)
        .ok_or_else(|| fail("codex_config_override_unverified"))?;
    Ok(Some(name))
}

fn profile<'a>(doc: &'a DocumentMut, name: Option<&str>) -> Option<&'a dyn TableLike> {
    doc.get("profiles")?.get(name?)?.as_table_like()
}

fn provider<'a>(doc: &'a DocumentMut, id: &str) -> Option<&'a Item> {
    doc.get("model_providers")?.get(id)
}

fn orbit_slot(id: &str) -> bool {
    id == PROVIDER
        || id
            .strip_prefix("orivelle_orbit_")
            .is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
}

fn slot(doc: &DocumentMut, binding: &str, helper: &Path) -> Result<String> {
    let providers = doc
        .get("model_providers")
        .map(|item| {
            item.as_table_like()
                .ok_or_else(|| fail("codex_configuration_invalid"))
        })
        .transpose()?;
    if let Some(id) = doc
        .get("model_provider")
        .and_then(Item::as_str)
        .filter(|id| orbit_slot(id))
        && let Some(auth) = provider(doc, id).and_then(|p| p.get("auth"))
        && auth.get("command").and_then(Item::as_str) == helper.to_str()
        && let Some(args) = auth.get("args").and_then(Item::as_array)
        && args.len() == 2
        && args.get(0).and_then(Value::as_str) == Some("codex")
        && args.get(1).and_then(Value::as_str) == Some(binding)
    {
        return Ok(id.into());
    }
    let mut reserved: BTreeSet<&str> = providers
        .into_iter()
        .flat_map(|p| p.iter().map(|(id, _)| id))
        .collect();
    reserved.extend(doc.get("model_provider").and_then(Item::as_str));
    if let Some(profiles) = doc.get("profiles").and_then(Item::as_table_like) {
        reserved.extend(
            profiles
                .iter()
                .filter_map(|(_, p)| p.get("model_provider")?.as_str()),
        );
    }
    if !reserved.contains(PROVIDER) {
        return Ok(PROVIDER.into());
    }
    // Every rejected name is an existing table entry, so the loop is bounded by the input.
    Ok((2..)
        .map(|i| format!("{PROVIDER}_{i}"))
        .find(|name| !reserved.contains(name.as_str()))
        .unwrap())
}

fn set_field(table: &mut dyn TableLike, key: &str, item: Option<&Item>) {
    match item {
        Some(item) => {
            if let Some(existing) = table.get_mut(key) {
                if canonical(Some(existing)) == canonical(Some(item)) {
                    return;
                }
                let mut next = item.clone();
                if let Some(old) = existing.as_value() {
                    next.make_value();
                    if let Some(new) = next.as_value_mut() {
                        *new.decor_mut() = old.decor().clone();
                    }
                }
                *existing = next;
            } else {
                table.insert(key, item.clone());
            }
        }
        None => {
            table.remove(key);
        }
    }
}

pub(crate) fn patch(
    original: Option<&[u8]>,
    base_url: &str,
    model: &str,
    helper: &Path,
    binding: &str,
    catalog: Option<(&Path, &str)>,
) -> Result<Vec<u8>> {
    validate_uuid(binding)?;
    let url = url::Url::parse(base_url).map_err(|_| fail("invalid_configuration"))?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || !helper.is_absolute()
        || model.is_empty()
        || model.len() > 255
    {
        return Err(fail("invalid_configuration"));
    }
    let mut doc = document(original)?;
    let active = active_profile(&doc)?.map(str::to_owned);
    for section in [
        Some(doc.as_table() as &dyn TableLike),
        profile(&doc, active.as_deref()),
    ]
    .into_iter()
    .flatten()
    {
        if ROUTING
            .iter()
            .filter_map(|k| section.get(k))
            .any(|i| i.as_str().is_none())
        {
            return Err(fail("codex_configuration_invalid"));
        }
    }
    let id = slot(&doc, binding, helper)?;
    let mut route = vec![("model", value(model)), ("model_provider", value(&id))];
    if let Some((path, effort)) = catalog {
        if !path.is_absolute()
            || effort.is_empty()
            || effort.len() > 128
            || !effort.bytes().all(|b| b.is_ascii_graphic())
        {
            return Err(fail("invalid_configuration"));
        }
        route.push((
            "model_catalog_json",
            value(path.to_str().ok_or_else(|| fail("invalid_path"))?),
        ));
        route.push(("model_reasoning_effort", value(effort)));
    }
    for (key, item) in &route {
        set_field(doc.as_table_mut(), key, Some(item));
    }
    if let Some(name) = active {
        let section = doc["profiles"][&name]
            .as_table_like_mut()
            .ok_or_else(|| fail("codex_config_override_unverified"))?;
        for (key, item) in &route {
            if section.contains_key(key) {
                set_field(section, key, Some(item));
            }
        }
    }
    if doc.get("model_providers").is_none() {
        let mut parent = Table::new();
        parent.set_implicit(true);
        doc["model_providers"] = Item::Table(parent);
    }
    let providers = doc["model_providers"]
        .as_table_like_mut()
        .ok_or_else(|| fail("invalid_configuration"))?;
    if let Some(existing) = providers.get_mut(&id) {
        let existing = existing
            .as_table_like_mut()
            .ok_or_else(|| fail("codex_configuration_invalid"))?;
        set_field(existing, "base_url", Some(&value(base_url)));
    } else {
        let mut auth = Table::new();
        auth["command"] = value(helper.to_str().ok_or_else(|| fail("invalid_path"))?);
        auth["args"] = value(["codex", binding].into_iter().collect::<toml_edit::Array>());
        auth["timeout_ms"] = value(5000);
        auth["refresh_interval_ms"] = value(300000);
        let mut p = Table::new();
        p["name"] = value("Orivelle Orbit");
        p["base_url"] = value(base_url);
        p["wire_api"] = value("responses");
        p["auth"] = Item::Table(auth);
        providers.insert(&id, Item::Table(p));
    }
    render(&doc, original)
}

// Canonicalize just the compared fields. Table order, quoting, whitespace and
// comments are not changes of routing authority.
fn semantic(v: &Value) -> serde_json::Value {
    use serde_json::json;
    match v {
        Value::InlineTable(t) => json!(
            t.iter()
                .map(|(k, v)| (k, semantic(v)))
                .collect::<BTreeMap<_, _>>()
        ),
        Value::Array(a) => json!(a.iter().map(semantic).collect::<Vec<_>>()),
        Value::String(s) => json!(["string", s.value()]),
        Value::Integer(i) => json!(["integer", i.value()]),
        Value::Boolean(b) => json!(["boolean", b.value()]),
        Value::Float(f) => json!(["float", f.value().to_bits()]),
        Value::Datetime(d) => json!(["datetime", d.value().to_string()]),
    }
}
fn fields(section: Option<&dyn TableLike>) -> Vec<Option<serde_json::Value>> {
    ROUTING
        .iter()
        .map(|key| canonical(section.and_then(|s| s.get(key))))
        .collect()
}
fn canonical(item: Option<&Item>) -> Option<serde_json::Value> {
    item.and_then(|i| i.clone().into_value().ok())
        .map(|v| semantic(&v))
}

pub(crate) fn routing_revision(bytes: Option<&[u8]>, slot: &str) -> Result<String> {
    let doc = document(bytes)?;
    let name = active_profile(&doc)?;
    let selected = profile(&doc, name)
        .and_then(|p| p.get("model_provider"))
        .or_else(|| doc.get("model_provider"))
        .and_then(Item::as_str);
    let data = (
        slot,
        name,
        fields(Some(doc.as_table())),
        fields(profile(&doc, name)),
        canonical(provider(&doc, slot)),
        selected.and_then(|id| canonical(provider(&doc, id))),
        if selected.unwrap_or("openai") == "openai" {
            canonical(doc.get("openai_base_url"))
        } else {
            None
        },
    );
    Ok(sha256(
        &serde_json::to_vec(&data).map_err(|_| fail("serialization_failed"))?,
    ))
}

pub(crate) fn applied_slot(bytes: Option<&[u8]>) -> Result<String> {
    document(bytes)?
        .get("model_provider")
        .and_then(Item::as_str)
        .filter(|id| orbit_slot(id))
        .map(str::to_owned)
        .ok_or_else(|| fail("configuration_conflict"))
}

pub(crate) fn same_route(applied: Option<&[u8]>, current: Option<&[u8]>) -> Result<bool> {
    if applied == current {
        return Ok(true);
    }
    let id = applied_slot(applied)?;
    Ok(routing_revision(applied, &id)? == routing_revision(current, &id)?)
}

pub(crate) fn restore(
    original: Option<&[u8]>,
    applied: Option<&[u8]>,
    current: Option<&[u8]>,
) -> Result<Option<Vec<u8>>> {
    if current == applied || current == original {
        return Ok(original.map(<[u8]>::to_vec));
    }
    if !same_route(applied, current).unwrap_or(false) {
        return Err(fail("configuration_conflict"));
    }
    let before = document(original)?;
    let mut doc = document(current)?;
    let id = applied_slot(applied)?;
    let active = active_profile(&doc)?.map(str::to_owned);
    for key in ROUTING {
        set_field(doc.as_table_mut(), key, before.get(key));
    }
    if let Some(name) = active {
        let saved = profile(&before, Some(&name));
        let section = doc["profiles"][&name]
            .as_table_like_mut()
            .ok_or_else(|| fail("configuration_conflict"))?;
        for key in ROUTING {
            set_field(section, key, saved.and_then(|s| s.get(key)));
        }
    }
    let providers = doc["model_providers"]
        .as_table_like_mut()
        .ok_or_else(|| fail("configuration_conflict"))?;
    set_field(providers, &id, provider(&before, &id));
    if providers.is_empty() && before.get("model_providers").is_none() {
        doc.remove("model_providers");
    }
    let bytes = render(&doc, current)?;
    if original.is_none() && bytes.iter().all(u8::is_ascii_whitespace) {
        return Ok(None);
    }
    Ok(Some(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connect(source: Option<&[u8]>) -> Vec<u8> {
        let helper = std::env::current_exe().unwrap();
        let catalog = helper.with_file_name("orbit.models.json");
        patch(
            source,
            "https://edge.example/v1",
            "gateway/text",
            &helper,
            "be9fc769-be65-40e6-bf00-167c845f1380",
            Some((&catalog, "high")),
        )
        .unwrap()
    }

    #[test]
    fn switcher_profiles_and_colliding_inline_providers_survive_the_round_trip() {
        let source = "\u{feff}# selected by another tool\r\nmodel = 'old' # keep comment\r\nprofile = 'work'\r\nprofiles = { work = { model = 'active', model_provider = 'custom', approval_policy = 'on-request' }, play = { model = 'inactive' } }\r\nmodel_providers = { custom = { base_url = 'https://other.example' }, orivelle_orbit = { env_key = 'OTHER_KEY' } }\r\n";
        let applied = connect(Some(source.as_bytes()));
        let doc = document(Some(&applied)).unwrap();
        let id = applied_slot(Some(&applied)).unwrap();
        assert_eq!(id, "orivelle_orbit_2");
        assert_eq!(
            doc["profiles"]["work"]["model_provider"].as_str(),
            Some(id.as_str())
        );
        assert_eq!(
            doc["profiles"]["work"]["model"].as_str(),
            Some("gateway/text")
        );
        assert!(
            doc["profiles"]["work"]
                .get("model_reasoning_effort")
                .is_none()
        );
        assert_eq!(doc["profiles"]["play"]["model"].as_str(), Some("inactive"));
        assert_eq!(
            doc["model_providers"]["orivelle_orbit"]["env_key"].as_str(),
            Some("OTHER_KEY")
        );
        assert_eq!(
            doc["model_providers"][&id]["auth"]["args"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        let rendered = std::str::from_utf8(&applied).unwrap();
        assert!(rendered.starts_with("\u{feff}# selected by another tool\r\n"));
        assert!(rendered.contains("# keep comment\r\n"));
        assert!(!rendered.replace("\r\n", "").contains('\n'));
        assert_eq!(connect(Some(&applied)), applied);
        assert_eq!(
            restore(Some(source.as_bytes()), Some(&applied), Some(&applied))
                .unwrap()
                .as_deref(),
            Some(source.as_bytes())
        );
        let edited = rendered
            .replace("on-request", "never")
            .replace("'inactive'", "'my-new-model'");
        let merged = restore(
            Some(source.as_bytes()),
            Some(&applied),
            Some(edited.as_bytes()),
        )
        .unwrap()
        .unwrap();
        let restored = document(Some(&merged)).unwrap();
        assert_eq!(
            restored["profiles"]["work"]["approval_policy"].as_str(),
            Some("never")
        );
        assert_eq!(
            restored["profiles"]["work"]["model"].as_str(),
            Some("active")
        );
        assert_eq!(
            restored["profiles"]["play"]["model"].as_str(),
            Some("my-new-model")
        );
        assert!(restored["model_providers"].get(&id).is_none());
        for changed in [
            rendered.replace("https://edge.example/v1", "https://different.example"),
            rendered.replace("profile = 'work'", "profile = 'play'"),
        ] {
            assert_eq!(
                restore(
                    Some(source.as_bytes()),
                    Some(&applied),
                    Some(changed.as_bytes())
                )
                .unwrap_err()
                .code,
                "configuration_conflict"
            );
        }
    }

    #[test]
    fn missing_file_and_dotted_profiles_keep_only_user_changes_on_disconnect() {
        let applied = connect(None);
        assert_eq!(restore(None, Some(&applied), Some(&applied)).unwrap(), None);
        let extra = b"\n[features]\npersonal = true\n";
        let edited = [applied.as_slice(), extra].concat();
        let restored = restore(None, Some(&applied), Some(&edited))
            .unwrap()
            .unwrap();
        let doc = document(Some(&restored)).unwrap();
        assert_eq!(doc.len(), 1);
        assert_eq!(doc["features"]["personal"].as_bool(), Some(true));
        let dotted =
            b"profile='local'\nprofiles.local.model='old'\nprofiles.other.model='private'\n";
        let patched = connect(Some(dotted));
        let doc = document(Some(&patched)).unwrap();
        assert_eq!(
            doc["profiles"]["local"]["model"].as_str(),
            Some("gateway/text")
        );
        assert_eq!(doc["profiles"]["other"]["model"].as_str(), Some("private"));
    }

    #[test]
    fn an_older_receipt_restores_a_preexisting_provider_into_an_inline_container() {
        let original =
            b"model_provider='custom'\n[model_providers.orivelle_orbit]\nname='previous'\n";
        let applied = connect(None);
        let mut edited = document(Some(&applied)).unwrap();
        edited["model_providers"].make_value();
        edited["personal"] = value(true);
        let restored = restore(
            Some(original),
            Some(&applied),
            Some(edited.to_string().as_bytes()),
        )
        .unwrap()
        .unwrap();
        let doc = document(Some(&restored)).unwrap();
        assert_eq!(
            doc["model_providers"]["orivelle_orbit"]["name"].as_str(),
            Some("previous")
        );
        assert_eq!(doc["model_provider"].as_str(), Some("custom"));
        assert_eq!(doc["personal"].as_bool(), Some(true));
    }

    #[test]
    fn review_revisions_track_the_selected_route_and_chosen_slot() {
        assert_eq!(
            routing_revision(None, PROVIDER).unwrap(),
            routing_revision(Some(b"[features]\npersonal=true"), PROVIDER).unwrap()
        );
        let original = b"openai_base_url = 'https://first.example'\n";
        assert_ne!(
            routing_revision(Some(original), PROVIDER).unwrap(),
            routing_revision(
                Some(b"openai_base_url = 'https://second.example'"),
                PROVIDER
            )
            .unwrap()
        );
        assert_ne!(
            routing_revision(Some(original), PROVIDER).unwrap(),
            routing_revision(Some(original), "orivelle_orbit_2").unwrap()
        );
    }
}
