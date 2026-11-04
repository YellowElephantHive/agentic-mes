//! MES entities, value types, and invariants.
//!
//! This crate is independent of databases, HTTP, agents, and LLM providers.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
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
    use super::{DomainError, PlannedQuantity, QualitySample, Quantity, WorkOrderStatus};

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
}
