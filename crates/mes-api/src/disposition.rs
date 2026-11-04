use std::collections::HashSet;

use chrono::{DateTime, Duration, Utc};
use mes_application::{
    AgentToolCallRead, DispositionEvidenceRef, DispositionTargetRef, DowntimeSummary,
    EvidenceState, ProductionSummary, ProposalType, QualitySummary, RecommendationCode,
};
use serde::Deserialize;
use uuid::Uuid;

const MAX_PROFILE_WINDOW_DAYS: i64 = 31;
const MAX_EVIDENCE_REFS: usize = 8;
const MAX_TARGET_REFS: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GeneratedDisposition {
    pub target_refs: Vec<DispositionTargetRef>,
    pub evidence_refs: Vec<DispositionEvidenceRef>,
    pub evidence_state: EvidenceState,
    pub recommended_actions: Vec<RecommendationCode>,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct WindowArguments {
    line_id: Uuid,
    start_at: DateTime<Utc>,
    end_at: DateTime<Utc>,
}

struct ProfileCall<'a, T> {
    audit: &'a AgentToolCallRead,
    arguments: WindowArguments,
    result: T,
}

trait ScopedSummary {
    fn line_id(&self) -> Uuid;
    fn start_at(&self) -> DateTime<Utc>;
    fn end_at(&self) -> DateTime<Utc>;
}

impl ScopedSummary for ProductionSummary {
    fn line_id(&self) -> Uuid {
        self.line_id
    }

    fn start_at(&self) -> DateTime<Utc> {
        self.start_at
    }

    fn end_at(&self) -> DateTime<Utc> {
        self.end_at
    }
}

impl ScopedSummary for DowntimeSummary {
    fn line_id(&self) -> Uuid {
        self.line_id
    }

    fn start_at(&self) -> DateTime<Utc> {
        self.start_at
    }

    fn end_at(&self) -> DateTime<Utc> {
        self.end_at
    }
}

impl ScopedSummary for QualitySummary {
    fn line_id(&self) -> Uuid {
        self.line_id
    }

    fn start_at(&self) -> DateTime<Utc> {
        self.start_at
    }

    fn end_at(&self) -> DateTime<Utc> {
        self.end_at
    }
}

pub(crate) fn generate_from_executed_evidence(
    proposal_type: ProposalType,
    calls: &[AgentToolCallRead],
) -> Result<GeneratedDisposition, &'static str> {
    let production = parse_calls::<ProductionSummary>(calls, "get_production_summary");
    if production.is_empty() {
        return Err("a bounded authorized production result is required");
    }
    match proposal_type {
        ProposalType::MaintenanceInvestigation => Ok(generate_maintenance(&production, calls)),
        ProposalType::QualityInvestigation => Ok(generate_quality(&production, calls)),
        ProposalType::MaterialDiscrepancyInvestigation => {
            Ok(generate_material_inconclusive(&production[0]))
        }
    }
}

fn generate_maintenance(
    production: &[ProfileCall<'_, ProductionSummary>],
    calls: &[AgentToolCallRead],
) -> GeneratedDisposition {
    let downtime = parse_calls::<DowntimeSummary>(calls, "get_equipment_downtime");
    let matched = downtime.iter().find_map(|downtime_call| {
        matching_production(production, &downtime_call.arguments)
            .map(|production_call| (production_call, downtime_call))
    });
    let Some((production_call, downtime_call)) = matched else {
        return inconclusive(
            &production[0],
            RecommendationCode::InspectMachine,
            "Required matching downtime evidence is unavailable; inspect the machine and collect bounded downtime evidence before drawing a conclusion.",
        );
    };
    let supported = !production_call.result.missing_data
        && !downtime_call.result.missing_data
        && !downtime_call.result.events.is_empty();
    if !supported {
        return inconclusive(
            production_call,
            RecommendationCode::InspectMachine,
            "Production or downtime evidence is missing or empty; inspect the machine before drawing a conclusion.",
        );
    }
    let mut targets = vec![line_target(production_call.arguments.line_id)];
    targets.extend(
        downtime_call
            .result
            .events
            .iter()
            .map(|event| DispositionTargetRef {
                resource_type: "machine".to_owned(),
                resource_id: event.machine_id.to_string(),
            }),
    );
    GeneratedDisposition {
        target_refs: dedupe_targets(targets),
        evidence_refs: evidence_refs([production_call.audit, downtime_call.audit]),
        evidence_state: EvidenceState::Supported,
        recommended_actions: vec![
            RecommendationCode::InspectMachine,
            RecommendationCode::RequestMaintenanceReview,
        ],
        summary: "Executed production and equipment-downtime evidence supports requesting a human maintenance investigation; it does not prove physical root cause.".to_owned(),
    }
}

fn generate_quality(
    production: &[ProfileCall<'_, ProductionSummary>],
    calls: &[AgentToolCallRead],
) -> GeneratedDisposition {
    let quality = parse_calls::<QualitySummary>(calls, "get_quality_summary");
    let matched = quality.iter().find_map(|quality_call| {
        matching_production(production, &quality_call.arguments)
            .map(|production_call| (production_call, quality_call))
    });
    let Some((production_call, quality_call)) = matched else {
        return inconclusive(
            &production[0],
            RecommendationCode::RequestQualityReview,
            "Required matching quality evidence is unavailable; request a quality review before drawing a conclusion.",
        );
    };
    let supported = !production_call.result.missing_data
        && !quality_call.result.missing_data
        && !quality_call.result.inspections.is_empty();
    if !supported {
        return inconclusive(
            production_call,
            RecommendationCode::RequestQualityReview,
            "Production or quality evidence is missing or empty; request a quality review before drawing a conclusion.",
        );
    }
    GeneratedDisposition {
        target_refs: vec![line_target(production_call.arguments.line_id)],
        evidence_refs: evidence_refs([production_call.audit, quality_call.audit]),
        evidence_state: EvidenceState::Supported,
        recommended_actions: vec![
            RecommendationCode::RequestQualityReview,
            RecommendationCode::ReviewProductionRecords,
        ],
        summary: "Executed production and quality-inspection evidence supports requesting a human quality investigation; it does not prove physical root cause.".to_owned(),
    }
}

fn generate_material_inconclusive(
    production: &ProfileCall<'_, ProductionSummary>,
) -> GeneratedDisposition {
    GeneratedDisposition {
        target_refs: vec![line_target(production.arguments.line_id)],
        evidence_refs: evidence_refs([production.audit]),
        evidence_state: EvidenceState::Inconclusive,
        recommended_actions: vec![RecommendationCode::InspectMaterialIdentity],
        summary: "No eligible material, inventory, RFID, weigh-scale, or independent physical-observation evidence is available; inspect material identity without asserting a mismatch or misconduct.".to_owned(),
    }
}

fn inconclusive(
    production: &ProfileCall<'_, ProductionSummary>,
    action: RecommendationCode,
    summary: &str,
) -> GeneratedDisposition {
    GeneratedDisposition {
        target_refs: vec![line_target(production.arguments.line_id)],
        evidence_refs: evidence_refs([production.audit]),
        evidence_state: EvidenceState::Inconclusive,
        recommended_actions: vec![action],
        summary: summary.to_owned(),
    }
}

fn matching_production<'a>(
    production: &'a [ProfileCall<'a, ProductionSummary>],
    arguments: &WindowArguments,
) -> Option<&'a ProfileCall<'a, ProductionSummary>> {
    production.iter().find(|call| call.arguments == *arguments)
}

fn parse_calls<'a, T>(calls: &'a [AgentToolCallRead], tool_name: &str) -> Vec<ProfileCall<'a, T>>
where
    T: for<'de> Deserialize<'de> + ScopedSummary,
{
    calls
        .iter()
        .filter(|call| call.tool_name == tool_name && call.auth_decision == "authorized")
        .filter_map(|call| {
            let arguments =
                serde_json::from_value::<WindowArguments>(call.redacted_arguments.clone()).ok()?;
            let duration = arguments.end_at - arguments.start_at;
            if duration <= Duration::zero() || duration > Duration::days(MAX_PROFILE_WINDOW_DAYS) {
                return None;
            }
            let result = serde_json::from_value::<T>(call.result_summary.clone()?).ok()?;
            if result.line_id() != arguments.line_id
                || result.start_at() != arguments.start_at
                || result.end_at() != arguments.end_at
            {
                return None;
            }
            Some(ProfileCall {
                audit: call,
                arguments,
                result,
            })
        })
        .collect()
}

fn line_target(line_id: Uuid) -> DispositionTargetRef {
    DispositionTargetRef {
        resource_type: "production_line".to_owned(),
        resource_id: line_id.to_string(),
    }
}

fn evidence_refs<'a>(
    calls: impl IntoIterator<Item = &'a AgentToolCallRead>,
) -> Vec<DispositionEvidenceRef> {
    let mut seen = HashSet::new();
    calls
        .into_iter()
        .filter(|call| seen.insert(call.id))
        .take(MAX_EVIDENCE_REFS)
        .map(|call| DispositionEvidenceRef {
            tool_call_id: call.id,
            tool_name: call.tool_name.clone(),
        })
        .collect()
}

fn dedupe_targets(targets: Vec<DispositionTargetRef>) -> Vec<DispositionTargetRef> {
    let mut seen = HashSet::new();
    targets
        .into_iter()
        .filter(|target| seen.insert((target.resource_type.clone(), target.resource_id.clone())))
        .take(MAX_TARGET_REFS)
        .collect()
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Utc};
    use mes_application::{AgentToolCallRead, EvidenceState, ProposalType, RecommendationCode};
    use serde_json::{Value, json};
    use uuid::Uuid;

    use super::generate_from_executed_evidence;

    #[test]
    fn profiles_fail_closed_on_mismatched_or_missing_evidence() {
        let line_id = Uuid::new_v4();
        let production = call(
            "get_production_summary",
            window(line_id, "2026-01-30T00:00:00Z", "2026-01-31T00:00:00Z"),
            json!({
                "line_id": line_id,
                "start_at": "2026-01-30T00:00:00Z",
                "end_at": "2026-01-31T00:00:00Z",
                "good_qty": 10,
                "reject_qty": 1,
                "throughput_good_units_per_hour": 10.0,
                "scrap_rate": 0.1,
                "records": [],
                "missing_data": false
            }),
            "authorized",
        );
        let mismatched_downtime = call(
            "get_equipment_downtime",
            window(line_id, "2026-01-29T00:00:00Z", "2026-01-30T00:00:00Z"),
            json!({
                "line_id": line_id,
                "start_at": "2026-01-29T00:00:00Z",
                "end_at": "2026-01-30T00:00:00Z",
                "line_downtime_minutes": 30,
                "events": [{
                    "event_id": Uuid::new_v4(),
                    "machine_id": Uuid::new_v4(),
                    "started_at": "2026-01-29T01:00:00Z",
                    "ended_at": "2026-01-29T01:30:00Z",
                    "reason_code": "MACHINE_FAILURE"
                }],
                "missing_data": false
            }),
            "authorized",
        );
        let maintenance = generate_from_executed_evidence(
            ProposalType::MaintenanceInvestigation,
            &[production.clone(), mismatched_downtime],
        )
        .expect("production evidence permits an inconclusive proposal");
        assert_eq!(maintenance.evidence_state, EvidenceState::Inconclusive);
        assert_eq!(
            maintenance.recommended_actions,
            vec![RecommendationCode::InspectMachine]
        );
        assert_eq!(maintenance.evidence_refs.len(), 1);

        let denied_quality = call(
            "get_quality_summary",
            window(line_id, "2026-01-30T00:00:00Z", "2026-01-31T00:00:00Z"),
            json!({
                "line_id": line_id,
                "start_at": "2026-01-30T00:00:00Z",
                "end_at": "2026-01-31T00:00:00Z",
                "sample_size": 10,
                "defect_count": 1,
                "qc_sample_defect_rate": 0.1,
                "inspections": [],
                "missing_data": false
            }),
            "denied",
        );
        let quality = generate_from_executed_evidence(
            ProposalType::QualityInvestigation,
            &[production, denied_quality],
        )
        .expect("production evidence permits an inconclusive proposal");
        assert_eq!(quality.evidence_state, EvidenceState::Inconclusive);
        assert_eq!(
            quality.recommended_actions,
            vec![RecommendationCode::RequestQualityReview]
        );
        assert_eq!(quality.evidence_refs.len(), 1);
    }

    fn window(line_id: Uuid, start_at: &str, end_at: &str) -> Value {
        json!({"line_id": line_id, "start_at": start_at, "end_at": end_at})
    }

    fn call(
        tool_name: &str,
        arguments: Value,
        result: Value,
        auth_decision: &str,
    ) -> AgentToolCallRead {
        AgentToolCallRead {
            id: Uuid::new_v4(),
            run_id: Uuid::new_v4(),
            tool_name: tool_name.to_owned(),
            redacted_arguments: arguments,
            auth_decision: auth_decision.to_owned(),
            result_summary: Some(result),
            duration_ms: 1,
            created_at: "2026-01-31T00:00:00Z"
                .parse::<DateTime<Utc>>()
                .expect("fixed timestamp must parse"),
        }
    }
}
