//! Public, read-only usage projections. No pricing or ledger decisions live here.
use crate::error::{Result, fail};
use serde::{Deserialize, Serialize};

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UsageQuery {
    pub connection_id: String,
    pub expected_revision: u64,
    pub date_from: String,
    pub date_to: String,
    pub timezone: String,
    pub outcome: Option<String>,
    pub cursor: Option<String>,
}
impl UsageQuery {
    pub(crate) fn target(&self, summary: bool) -> Result<String> {
        let from = chrono::NaiveDate::parse_from_str(&self.date_from, "%Y-%m-%d")
            .map_err(|_| fail("invalid_usage_query"))?;
        let to = chrono::NaiveDate::parse_from_str(&self.date_to, "%Y-%m-%d")
            .map_err(|_| fail("invalid_usage_query"))?;
        if !(0..90).contains(&(to - from).num_days())
            || self.timezone.is_empty()
            || self.timezone.len() > 128
            || self.cursor.as_ref().is_some_and(|c| c.len() > 2048)
            || self.outcome.as_deref().is_some_and(|s| {
                ![
                    "",
                    "succeeded",
                    "failed",
                    "cancelled",
                    "rejected",
                    "interrupted",
                ]
                .contains(&s)
            })
        {
            return Err(fail("invalid_usage_query"));
        }
        let mut query = url::form_urlencoded::Serializer::new(String::new());
        query
            .append_pair("date_from", &self.date_from)
            .append_pair("date_to", &self.date_to)
            .append_pair("timezone", &self.timezone);
        if !summary {
            if let Some(outcome) = &self.outcome {
                query.append_pair("outcome", outcome);
            }
            if let Some(cursor) = &self.cursor {
                query.append_pair("cursor", cursor);
            }
        }
        Ok(format!(
            "/api/v1/orbit/usage/{}?{}",
            if summary { "summary" } else { "records" },
            query.finish()
        ))
    }
}
#[derive(Clone, Deserialize, Serialize)]
pub struct UsageQuantity {
    pub value: Option<u64>,
    pub status: String,
}
#[derive(Clone, Deserialize, Serialize)]
pub struct UsageTokens {
    pub state: String,
    pub total: UsageQuantity,
    pub input: UsageQuantity,
    pub cached_input: UsageQuantity,
    pub cache_write: UsageQuantity,
    pub output: UsageQuantity,
    pub reasoning: UsageQuantity,
}
#[derive(Clone, Deserialize, Serialize)]
pub struct UsageMetrics {
    pub tokens: UsageTokens,
    pub image_count: Option<UsageQuantity>,
    pub reconciliation_pending_count: u64,
}
#[derive(Clone, Deserialize, Serialize)]
pub struct UsageMoney {
    pub amount: Option<String>,
    pub currency: Option<String>,
    #[serde(default)]
    pub known_amount: Option<String>,
    #[serde(default)]
    pub pending_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
}
#[derive(Clone, Deserialize, Serialize)]
pub struct UsageRange {
    pub date_from: String,
    pub date_to: String,
    pub timezone: String,
    pub defaulted: bool,
    pub max_days: u64,
}
#[derive(Clone, Deserialize, Serialize)]
pub struct UsageCounts {
    pub requests: u64,
    pub succeeded: u64,
    pub failed: u64,
    pub cancelled: u64,
    pub rejected: u64,
    pub interrupted: u64,
}
#[derive(Clone, Deserialize, Serialize)]
pub struct UsageTotals {
    #[serde(flatten)]
    pub counts: UsageCounts,
    pub usage: UsageMetrics,
    pub charges: Vec<UsageMoney>,
}
#[derive(Clone, Deserialize, Serialize)]
pub struct UsageDay {
    pub date: String,
    #[serde(flatten)]
    pub counts: UsageCounts,
    pub charges: Vec<UsageMoney>,
}
#[derive(Clone, Deserialize, Serialize)]
pub struct UsageSummary {
    pub range: UsageRange,
    pub summary: UsageTotals,
    pub days: Vec<UsageDay>,
    pub has_legacy_semantics: bool,
    pub updated_at: Option<String>,
    pub generated_at: String,
}
#[derive(Clone, Deserialize, Serialize)]
pub struct UsageModel {
    pub display_name: String,
}
#[derive(Clone, Deserialize, Serialize)]
pub struct UsageKey {
    pub name: String,
    pub prefix: String,
}
#[derive(Clone, Deserialize, Serialize)]
pub struct UsageRecord {
    pub id: String,
    pub request_id: String,
    pub result_state: String,
    pub outcome: Option<String>,
    pub result_label: String,
    pub result_note: Option<String>,
    pub recorded_at: String,
    pub provider_model: UsageModel,
    pub api_key: Option<UsageKey>,
    pub usage: UsageMetrics,
    pub total_duration_ms: Option<u64>,
    pub first_byte_duration_ms: Option<u64>,
    pub charge_state: String,
    pub charges: Vec<UsageMoney>,
}
#[derive(Clone, Deserialize, Serialize)]
pub struct UsagePage {
    pub data: Vec<UsageRecord>,
    pub prev_cursor: Option<String>,
    pub next_cursor: Option<String>,
    pub cursor_reset: bool,
    pub range: UsageRange,
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_query_encodes_signed_cursors_without_changing_their_value() {
        let mut q = UsageQuery {
            connection_id: "synthetic".into(),
            expected_revision: 1,
            date_from: "2026-11-01".into(),
            date_to: "2026-11-13".into(),
            timezone: "Asia/Shanghai".into(),
            outcome: Some("failed".into()),
            cursor: Some("a+/=?&actor=other".into()),
        };
        let target = q.target(false).unwrap();
        assert!(target.contains("cursor=a%2B%2F%3D%3F%26actor%3Dother"));
        assert!(!q.target(true).unwrap().contains("cursor="));
        q.date_to = "2027-11-01".into();
        assert!(q.target(false).is_err());
    }
}
