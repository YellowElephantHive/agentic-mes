//! MES use cases, repository contracts, seed plans, and source-data contracts.

use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use mes_domain::{PlannedQuantity, Quantity, WorkOrderStatus};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum ApplicationError {
    #[error("repository error: {0}")]
    Repository(String),
    #[error("dataset error: {0}")]
    Dataset(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvaluationTrack {
    PublishedSynthetic,
    RealWorld,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceRecord {
    pub source_record_id: String,
    pub source_locator: String,
    pub raw_payload: Value,
    pub normalized_payload: Value,
    pub missing_fields: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatasetManifest {
    pub dataset_key: String,
    pub track: EvaluationTrack,
    pub name: String,
    pub version: String,
    pub license: String,
    pub source_url: String,
    pub source_files: Vec<String>,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DatasetImport {
    pub manifest: DatasetManifest,
    pub records: Vec<SourceRecord>,
}

pub trait DatasetAdapter {
    type Input;

    /// # Errors
    ///
    /// Returns [`ApplicationError::Dataset`] when source bytes or fields are invalid.
    fn import(&self, input: Self::Input) -> Result<DatasetImport, ApplicationError>;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TenantSeed {
    pub id: Uuid,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FactorySeed {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub name: String,
    pub slug: String,
    pub timezone: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LineSeed {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub factory_id: Uuid,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MachineSeed {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub factory_id: Uuid,
    pub line_id: Uuid,
    pub name: String,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProductSeed {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub sku: String,
    pub ideal_cycle_seconds: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkOrderSeed {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub factory_id: Uuid,
    pub line_id: Uuid,
    pub product_id: Uuid,
    pub status: WorkOrderStatus,
    pub planned_qty: PlannedQuantity,
    pub due_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProductionRecordSeed {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub factory_id: Uuid,
    pub line_id: Uuid,
    pub work_order_id: Uuid,
    pub machine_id: Uuid,
    pub occurred_at: DateTime<Utc>,
    pub good_qty: Quantity,
    pub reject_qty: Quantity,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DowntimeSeed {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub factory_id: Uuid,
    pub line_id: Uuid,
    pub machine_id: Uuid,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
    pub reason_code: String,
    pub source_scenario_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InspectionSeed {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub factory_id: Uuid,
    pub line_id: Uuid,
    pub work_order_id: Uuid,
    pub inspected_at: DateTime<Utc>,
    pub sample_size: Quantity,
    pub defect_count: Quantity,
    pub defect_code: Option<String>,
    pub source_scenario_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FactorySeedPlan {
    pub seed: u64,
    pub first_date: NaiveDate,
    pub last_date: NaiveDate,
    pub tenant: TenantSeed,
    pub factory: FactorySeed,
    pub lines: Vec<LineSeed>,
    pub machines: Vec<MachineSeed>,
    pub products: Vec<ProductSeed>,
    pub work_orders: Vec<WorkOrderSeed>,
    pub production_records: Vec<ProductionRecordSeed>,
    pub downtime_events: Vec<DowntimeSeed>,
    pub quality_inspections: Vec<InspectionSeed>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeedSummary {
    pub tenant_id: Uuid,
    pub factory_id: Uuid,
    pub work_orders: usize,
    pub production_records: usize,
    pub downtime_events: usize,
    pub quality_inspections: usize,
}

#[async_trait]
pub trait SeedRepository: Send + Sync {
    async fn replace_seed_plan(
        &self,
        plan: &FactorySeedPlan,
    ) -> Result<SeedSummary, ApplicationError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductionTotals {
    pub good_qty: i64,
    pub reject_qty: i64,
    pub record_count: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkOrderRead {
    pub id: Uuid,
    pub factory_id: Uuid,
    pub line_id: Uuid,
    pub product_id: Uuid,
    pub status: WorkOrderStatus,
    pub planned_qty: i64,
    pub completed_good_qty: i64,
    pub reject_qty: i64,
    pub due_at: DateTime<Utc>,
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProductionEvidence {
    pub record_id: Uuid,
    pub occurred_at: DateTime<Utc>,
    pub good_qty: i64,
    pub reject_qty: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProductionSummary {
    pub line_id: Uuid,
    pub start_at: DateTime<Utc>,
    pub end_at: DateTime<Utc>,
    pub good_qty: i64,
    pub reject_qty: i64,
    pub throughput_good_units_per_hour: f64,
    pub scrap_rate: Option<f64>,
    pub records: Vec<ProductionEvidence>,
    pub missing_data: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DowntimeEvidence {
    pub event_id: Uuid,
    pub machine_id: Uuid,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
    pub reason_code: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DowntimeSummary {
    pub line_id: Uuid,
    pub start_at: DateTime<Utc>,
    pub end_at: DateTime<Utc>,
    pub line_downtime_minutes: i64,
    pub events: Vec<DowntimeEvidence>,
    pub missing_data: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QualityEvidence {
    pub inspection_id: Uuid,
    pub inspected_at: DateTime<Utc>,
    pub sample_size: i64,
    pub defect_count: i64,
    pub defect_code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QualitySummary {
    pub line_id: Uuid,
    pub start_at: DateTime<Utc>,
    pub end_at: DateTime<Utc>,
    pub sample_size: i64,
    pub defect_count: i64,
    pub qc_sample_defect_rate: Option<f64>,
    pub inspections: Vec<QualityEvidence>,
    pub missing_data: bool,
}

#[async_trait]
pub trait MesReadRepository: Send + Sync {
    async fn get_work_order(
        &self,
        tenant_id: Uuid,
        factory_id: Uuid,
        work_order_id: Uuid,
    ) -> Result<Option<WorkOrderRead>, ApplicationError>;

    async fn production_summary(
        &self,
        tenant_id: Uuid,
        factory_id: Uuid,
        line_id: Uuid,
        start_at: DateTime<Utc>,
        end_at: DateTime<Utc>,
        limit: i64,
    ) -> Result<Option<ProductionSummary>, ApplicationError>;

    async fn downtime_summary(
        &self,
        tenant_id: Uuid,
        factory_id: Uuid,
        line_id: Uuid,
        start_at: DateTime<Utc>,
        end_at: DateTime<Utc>,
        limit: i64,
    ) -> Result<Option<DowntimeSummary>, ApplicationError>;

    async fn quality_summary(
        &self,
        tenant_id: Uuid,
        factory_id: Uuid,
        line_id: Uuid,
        start_at: DateTime<Utc>,
        end_at: DateTime<Utc>,
        limit: i64,
    ) -> Result<Option<QualitySummary>, ApplicationError>;

    async fn production_totals(
        &self,
        tenant_id: Uuid,
        factory_id: Uuid,
        line_id: Uuid,
        start_at: DateTime<Utc>,
        end_at: DateTime<Utc>,
    ) -> Result<ProductionTotals, ApplicationError>;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DatasetGroundTruth {
    pub dataset_key: String,
    pub import_id: Uuid,
    pub record_count: i64,
    pub outcome_counts: Value,
    pub records_with_missing: i64,
    pub evidence: Vec<String>,
}

#[async_trait]
pub trait DatasetReadRepository: Send + Sync {
    async fn dataset_ground_truth(
        &self,
        dataset_key: &str,
    ) -> Result<Option<DatasetGroundTruth>, ApplicationError>;
}

#[derive(Debug, Clone)]
pub struct AgentRunStart {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub user_id: String,
    pub agent_id: String,
    pub provider_name: String,
    pub data_policy: String,
}

#[derive(Debug, Clone)]
pub struct AgentToolAudit {
    pub id: Uuid,
    pub run_id: Uuid,
    pub tool_name: String,
    pub redacted_arguments: Value,
    pub auth_decision: String,
    pub result_summary: Option<Value>,
    pub duration_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentRunRead {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub user_id: String,
    pub agent_id: String,
    pub provider_name: String,
    pub data_policy: String,
    pub status: String,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub error_code: Option<String>,
    pub final_response: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentToolCallRead {
    pub id: Uuid,
    pub run_id: Uuid,
    pub tool_name: String,
    pub redacted_arguments: Value,
    pub auth_decision: String,
    pub result_summary: Option<Value>,
    pub duration_ms: i64,
    pub created_at: DateTime<Utc>,
}

#[async_trait]
pub trait AgentAuditRepository: Send + Sync {
    async fn start_run(&self, run: &AgentRunStart) -> Result<(), ApplicationError>;
    async fn record_tool_call(&self, audit: &AgentToolAudit) -> Result<(), ApplicationError>;
    async fn finish_run(
        &self,
        run_id: Uuid,
        status: &str,
        final_response: Option<&str>,
        error_code: Option<&str>,
    ) -> Result<(), ApplicationError>;
    async fn get_run(
        &self,
        tenant_id: Uuid,
        run_id: Uuid,
    ) -> Result<Option<AgentRunRead>, ApplicationError>;
    async fn get_tool_calls(
        &self,
        tenant_id: Uuid,
        run_id: Uuid,
    ) -> Result<Vec<AgentToolCallRead>, ApplicationError>;
}
