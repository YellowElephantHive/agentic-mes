//! MES entities, value types, and invariants.
//!
//! This crate is independent of databases, HTTP, agents, and LLM providers.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum DomainError {
    #[error("quantity must be non-negative")]
    NegativeQuantity,
    #[error("planned quantity must be positive")]
    NonPositivePlannedQuantity,
    #[error("defect count cannot exceed sample size")]
    DefectsExceedSample,
    #[error("time range end must not precede start")]
    InvalidTimeRange,
    #[error("invalid work-order transition from {from:?} to {to:?}")]
    InvalidWorkOrderTransition {
        from: WorkOrderStatus,
        to: WorkOrderStatus,
    },
    #[error("unknown disposition proposal type: {0}")]
    UnknownProposalType(String),
    #[error("unknown disposition recommendation code: {0}")]
    UnknownRecommendationCode(String),
    #[error("unknown disposition proposal status: {0}")]
    UnknownProposalStatus(String),
    #[error("unknown disposition evidence state: {0}")]
    UnknownEvidenceState(String),
    #[error("unknown disposition review decision: {0}")]
    UnknownReviewDecision(String),
    #[error("recommendation {recommendation:?} is not allowed for {proposal_type:?}")]
    RecommendationNotAllowed {
        proposal_type: ProposalType,
        recommendation: RecommendationCode,
    },
    #[error("unknown action-plan type: {0}")]
    UnknownActionPlanType(String),
    #[error("unknown action-plan status: {0}")]
    UnknownActionPlanStatus(String),
    #[error("unknown action-plan step: {0}")]
    UnknownActionPlanStep(String),
    #[error("step {step:?} is not allowed for {plan_type:?}")]
    ActionPlanStepNotAllowed {
        plan_type: ActionPlanType,
        step: ActionPlanStep,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionPlanType {
    MaintenanceInspectionPlan,
    QualityInvestigationPlan,
    InventoryReconciliationPlan,
}

impl ActionPlanType {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MaintenanceInspectionPlan => "maintenance_inspection_plan",
            Self::QualityInvestigationPlan => "quality_investigation_plan",
            Self::InventoryReconciliationPlan => "inventory_reconciliation_plan",
        }
    }

    /// Validates a declarative step against the frozen M9 registry.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::ActionPlanStepNotAllowed`] when the step does not
    /// belong to this plan type.
    pub fn validate_step(self, step: ActionPlanStep) -> Result<(), DomainError> {
        let allowed = matches!(
            (self, step),
            (
                Self::MaintenanceInspectionPlan,
                ActionPlanStep::MachineInspectionCheck
                    | ActionPlanStep::DowntimeHistoryReview
                    | ActionPlanStep::MaintenanceEvidenceCollection
                    | ActionPlanStep::MaintenanceRoleNotification
            ) | (
                Self::QualityInvestigationPlan,
                ActionPlanStep::QualityHistoryReview
                    | ActionPlanStep::QualitySampleCollection
                    | ActionPlanStep::ProductionRecordsReview
                    | ActionPlanStep::QualityRoleNotification
            ) | (
                Self::InventoryReconciliationPlan,
                ActionPlanStep::MaterialIdentityInspection
                    | ActionPlanStep::InventoryRecordsReconciliation
                    | ActionPlanStep::PhysicalObservationCollection
                    | ActionPlanStep::QualityRoleNotification
            )
        );
        if allowed {
            Ok(())
        } else {
            Err(DomainError::ActionPlanStepNotAllowed {
                plan_type: self,
                step,
            })
        }
    }
}

impl From<ProposalType> for ActionPlanType {
    fn from(value: ProposalType) -> Self {
        match value {
            ProposalType::MaintenanceInvestigation => Self::MaintenanceInspectionPlan,
            ProposalType::QualityInvestigation => Self::QualityInvestigationPlan,
            ProposalType::MaterialDiscrepancyInvestigation => Self::InventoryReconciliationPlan,
        }
    }
}

impl FromStr for ActionPlanType {
    type Err = DomainError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "maintenance_inspection_plan" => Ok(Self::MaintenanceInspectionPlan),
            "quality_investigation_plan" => Ok(Self::QualityInvestigationPlan),
            "inventory_reconciliation_plan" => Ok(Self::InventoryReconciliationPlan),
            _ => Err(DomainError::UnknownActionPlanType(value.to_owned())),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionPlanStatus {
    Draft,
    Validated,
    Simulated,
    PendingApproval,
    PlanApproved,
    PlanRejected,
    Expired,
    InvalidatedByDrift,
    Cancelled,
}

impl ActionPlanStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Validated => "validated",
            Self::Simulated => "simulated",
            Self::PendingApproval => "pending_approval",
            Self::PlanApproved => "plan_approved",
            Self::PlanRejected => "plan_rejected",
            Self::Expired => "expired",
            Self::InvalidatedByDrift => "invalidated_by_drift",
            Self::Cancelled => "cancelled",
        }
    }
}

impl FromStr for ActionPlanStatus {
    type Err = DomainError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "draft" => Ok(Self::Draft),
            "validated" => Ok(Self::Validated),
            "simulated" => Ok(Self::Simulated),
            "pending_approval" => Ok(Self::PendingApproval),
            "plan_approved" => Ok(Self::PlanApproved),
            "plan_rejected" => Ok(Self::PlanRejected),
            "expired" => Ok(Self::Expired),
            "invalidated_by_drift" => Ok(Self::InvalidatedByDrift),
            "cancelled" => Ok(Self::Cancelled),
            _ => Err(DomainError::UnknownActionPlanStatus(value.to_owned())),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionPlanStep {
    MachineInspectionCheck,
    DowntimeHistoryReview,
    MaintenanceEvidenceCollection,
    MaintenanceRoleNotification,
    QualityHistoryReview,
    QualitySampleCollection,
    ProductionRecordsReview,
    QualityRoleNotification,
    MaterialIdentityInspection,
    InventoryRecordsReconciliation,
    PhysicalObservationCollection,
}

impl ActionPlanStep {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MachineInspectionCheck => "machine_inspection_check",
            Self::DowntimeHistoryReview => "downtime_history_review",
            Self::MaintenanceEvidenceCollection => "maintenance_evidence_collection",
            Self::MaintenanceRoleNotification => "maintenance_role_notification",
            Self::QualityHistoryReview => "quality_history_review",
            Self::QualitySampleCollection => "quality_sample_collection",
            Self::ProductionRecordsReview => "production_records_review",
            Self::QualityRoleNotification => "quality_role_notification",
            Self::MaterialIdentityInspection => "material_identity_inspection",
            Self::InventoryRecordsReconciliation => "inventory_records_reconciliation",
            Self::PhysicalObservationCollection => "physical_observation_collection",
        }
    }
}

impl FromStr for ActionPlanStep {
    type Err = DomainError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "machine_inspection_check" => Ok(Self::MachineInspectionCheck),
            "downtime_history_review" => Ok(Self::DowntimeHistoryReview),
            "maintenance_evidence_collection" => Ok(Self::MaintenanceEvidenceCollection),
            "maintenance_role_notification" => Ok(Self::MaintenanceRoleNotification),
            "quality_history_review" => Ok(Self::QualityHistoryReview),
            "quality_sample_collection" => Ok(Self::QualitySampleCollection),
            "production_records_review" => Ok(Self::ProductionRecordsReview),
            "quality_role_notification" => Ok(Self::QualityRoleNotification),
            "material_identity_inspection" => Ok(Self::MaterialIdentityInspection),
            "inventory_records_reconciliation" => Ok(Self::InventoryRecordsReconciliation),
            "physical_observation_collection" => Ok(Self::PhysicalObservationCollection),
            _ => Err(DomainError::UnknownActionPlanStep(value.to_owned())),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalType {
    MaintenanceInvestigation,
    QualityInvestigation,
    MaterialDiscrepancyInvestigation,
}

impl ProposalType {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MaintenanceInvestigation => "maintenance_investigation",
            Self::QualityInvestigation => "quality_investigation",
            Self::MaterialDiscrepancyInvestigation => "material_discrepancy_investigation",
        }
    }

    /// Validates a recommendation against the frozen M8 registry.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::RecommendationNotAllowed`] when the action does
    /// not belong to this proposal type.
    pub fn validate_recommendation(
        self,
        recommendation: RecommendationCode,
    ) -> Result<(), DomainError> {
        let allowed = matches!(
            (self, recommendation),
            (
                Self::MaintenanceInvestigation,
                RecommendationCode::InspectMachine | RecommendationCode::RequestMaintenanceReview
            ) | (
                Self::QualityInvestigation,
                RecommendationCode::RequestQualityReview
                    | RecommendationCode::ReviewProductionRecords
            ) | (
                Self::MaterialDiscrepancyInvestigation,
                RecommendationCode::InspectMaterialIdentity
                    | RecommendationCode::ReconcileInventory
                    | RecommendationCode::RequestQualityReview
            )
        );
        if allowed {
            Ok(())
        } else {
            Err(DomainError::RecommendationNotAllowed {
                proposal_type: self,
                recommendation,
            })
        }
    }
}

impl FromStr for ProposalType {
    type Err = DomainError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "maintenance_investigation" => Ok(Self::MaintenanceInvestigation),
            "quality_investigation" => Ok(Self::QualityInvestigation),
            "material_discrepancy_investigation" => Ok(Self::MaterialDiscrepancyInvestigation),
            _ => Err(DomainError::UnknownProposalType(value.to_owned())),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecommendationCode {
    InspectMachine,
    RequestMaintenanceReview,
    RequestQualityReview,
    ReviewProductionRecords,
    InspectMaterialIdentity,
    ReconcileInventory,
}

impl RecommendationCode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InspectMachine => "inspect_machine",
            Self::RequestMaintenanceReview => "request_maintenance_review",
            Self::RequestQualityReview => "request_quality_review",
            Self::ReviewProductionRecords => "review_production_records",
            Self::InspectMaterialIdentity => "inspect_material_identity",
            Self::ReconcileInventory => "reconcile_inventory",
        }
    }
}

impl FromStr for RecommendationCode {
    type Err = DomainError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "inspect_machine" => Ok(Self::InspectMachine),
            "request_maintenance_review" => Ok(Self::RequestMaintenanceReview),
            "request_quality_review" => Ok(Self::RequestQualityReview),
            "review_production_records" => Ok(Self::ReviewProductionRecords),
            "inspect_material_identity" => Ok(Self::InspectMaterialIdentity),
            "reconcile_inventory" => Ok(Self::ReconcileInventory),
            _ => Err(DomainError::UnknownRecommendationCode(value.to_owned())),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalStatus {
    Draft,
    PendingReview,
    ReviewApproved,
    ReviewRejected,
    Expired,
    Cancelled,
}

impl ProposalStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::PendingReview => "pending_review",
            Self::ReviewApproved => "review_approved",
            Self::ReviewRejected => "review_rejected",
            Self::Expired => "expired",
            Self::Cancelled => "cancelled",
        }
    }
}

impl FromStr for ProposalStatus {
    type Err = DomainError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "draft" => Ok(Self::Draft),
            "pending_review" => Ok(Self::PendingReview),
            "review_approved" => Ok(Self::ReviewApproved),
            "review_rejected" => Ok(Self::ReviewRejected),
            "expired" => Ok(Self::Expired),
            "cancelled" => Ok(Self::Cancelled),
            _ => Err(DomainError::UnknownProposalStatus(value.to_owned())),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewDecision {
    Approved,
    Rejected,
}

impl ReviewDecision {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Approved => "approved",
            Self::Rejected => "rejected",
        }
    }

    #[must_use]
    pub const fn resulting_status(self) -> ProposalStatus {
        match self {
            Self::Approved => ProposalStatus::ReviewApproved,
            Self::Rejected => ProposalStatus::ReviewRejected,
        }
    }
}

impl FromStr for ReviewDecision {
    type Err = DomainError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "approved" => Ok(Self::Approved),
            "rejected" => Ok(Self::Rejected),
            _ => Err(DomainError::UnknownReviewDecision(value.to_owned())),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceState {
    Supported,
    SuspectedMismatch,
    Inconclusive,
    ConfirmedMatch,
    ConfirmedMismatch,
}

impl EvidenceState {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Supported => "supported",
            Self::SuspectedMismatch => "suspected_mismatch",
            Self::Inconclusive => "inconclusive",
            Self::ConfirmedMatch => "confirmed_match",
            Self::ConfirmedMismatch => "confirmed_mismatch",
        }
    }
}

impl FromStr for EvidenceState {
    type Err = DomainError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "supported" => Ok(Self::Supported),
            "suspected_mismatch" => Ok(Self::SuspectedMismatch),
            "inconclusive" => Ok(Self::Inconclusive),
            "confirmed_match" => Ok(Self::ConfirmedMatch),
            "confirmed_mismatch" => Ok(Self::ConfirmedMismatch),
            _ => Err(DomainError::UnknownEvidenceState(value.to_owned())),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkOrderStatus {
    Planned,
    Released,
    InProgress,
    Completed,
    Cancelled,
}

impl WorkOrderStatus {
    /// Validates a business transition without relying on an agent or database.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidWorkOrderTransition`] for a forbidden transition.
    pub fn transition_to(self, next: Self) -> Result<Self, DomainError> {
        let valid = matches!(
            (self, next),
            (Self::Planned, Self::Released | Self::Cancelled)
                | (Self::Released, Self::InProgress | Self::Cancelled)
                | (Self::InProgress, Self::Completed | Self::Cancelled)
        ) || self == next;

        if valid {
            Ok(next)
        } else {
            Err(DomainError::InvalidWorkOrderTransition {
                from: self,
                to: next,
            })
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Quantity(i64);

impl Quantity {
    /// # Errors
    ///
    /// Returns [`DomainError::NegativeQuantity`] when `value` is negative.
    pub fn new(value: i64) -> Result<Self, DomainError> {
        if value < 0 {
            Err(DomainError::NegativeQuantity)
        } else {
            Ok(Self(value))
        }
    }

    #[must_use]
    pub const fn get(self) -> i64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedQuantity(Quantity);

impl PlannedQuantity {
    /// # Errors
    ///
    /// Returns an error unless `value` is strictly positive.
    pub fn new(value: i64) -> Result<Self, DomainError> {
        if value == 0 {
            Err(DomainError::NonPositivePlannedQuantity)
        } else {
            Quantity::new(value).map(Self)
        }
    }

    #[must_use]
    pub const fn get(self) -> i64 {
        self.0.get()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct QualitySample {
    pub sample_size: Quantity,
    pub defect_count: Quantity,
}

impl QualitySample {
    /// # Errors
    ///
    /// Returns an error for negative values or when defects exceed the sample size.
    pub fn new(sample_size: i64, defect_count: i64) -> Result<Self, DomainError> {
        let sample_size = Quantity::new(sample_size)?;
        let defect_count = Quantity::new(defect_count)?;
        if defect_count.get() > sample_size.get() {
            return Err(DomainError::DefectsExceedSample);
        }
        Ok(Self {
            sample_size,
            defect_count,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeRange {
    pub start_at: DateTime<Utc>,
    pub end_at: DateTime<Utc>,
}

impl TimeRange {
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidTimeRange`] when the end precedes the start.
    pub fn new(start_at: DateTime<Utc>, end_at: DateTime<Utc>) -> Result<Self, DomainError> {
        if end_at < start_at {
            Err(DomainError::InvalidTimeRange)
        } else {
            Ok(Self { start_at, end_at })
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkOrder {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub factory_id: Uuid,
    pub line_id: Uuid,
    pub product_id: Uuid,
    pub status: WorkOrderStatus,
    pub planned_qty: PlannedQuantity,
    pub due_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::{
        ActionPlanStep, ActionPlanType, DomainError, PlannedQuantity, ProposalType, QualitySample,
        Quantity, RecommendationCode, WorkOrderStatus,
    };

    #[test]
    fn quantities_reject_impossible_values() {
        assert_eq!(Quantity::new(-1), Err(DomainError::NegativeQuantity));
        assert_eq!(
            PlannedQuantity::new(0),
            Err(DomainError::NonPositivePlannedQuantity)
        );
        assert_eq!(
            QualitySample::new(4, 5),
            Err(DomainError::DefectsExceedSample)
        );
    }

    #[test]
    fn work_order_transitions_are_enforced() {
        assert_eq!(
            WorkOrderStatus::Planned.transition_to(WorkOrderStatus::Released),
            Ok(WorkOrderStatus::Released)
        );
        assert!(matches!(
            WorkOrderStatus::Completed.transition_to(WorkOrderStatus::InProgress),
            Err(DomainError::InvalidWorkOrderTransition { .. })
        ));
    }

    #[test]
    fn disposition_registry_rejects_cross_type_recommendations() {
        assert!(
            ProposalType::MaintenanceInvestigation
                .validate_recommendation(RecommendationCode::InspectMachine)
                .is_ok()
        );
        assert!(matches!(
            ProposalType::QualityInvestigation
                .validate_recommendation(RecommendationCode::InspectMachine),
            Err(DomainError::RecommendationNotAllowed { .. })
        ));
    }

    #[test]
    fn action_plan_registry_is_derived_and_rejects_cross_type_steps() {
        assert_eq!(
            ActionPlanType::from(ProposalType::MaintenanceInvestigation),
            ActionPlanType::MaintenanceInspectionPlan
        );
        assert!(
            ActionPlanType::MaintenanceInspectionPlan
                .validate_step(ActionPlanStep::MachineInspectionCheck)
                .is_ok()
        );
        assert!(matches!(
            ActionPlanType::QualityInvestigationPlan
                .validate_step(ActionPlanStep::MachineInspectionCheck),
            Err(DomainError::ActionPlanStepNotAllowed { .. })
        ));
    }
}
