//! PostgreSQL repositories and source-dataset adapters.

pub mod datasets;
mod postgres;

pub use postgres::{DatasetPersistenceSummary, PostgresMesRepository};
