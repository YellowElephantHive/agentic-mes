use async_trait::async_trait;
use chrono::{DateTime, Utc};
use mes_application::{
    ACTION_PLAN_SIMULATION_POLICY_VERSION, ACTION_PLAN_SIMULATION_SEED,
    ACTION_PLAN_VALIDATION_POLICY_VERSION, ActionPlanCreateResult, ActionPlanDraft, ActionPlanRead,
    ActionPlanRepository, ActionPlanReviewCommand, ActionPlanReviewRead,
    ActionPlanSimulationCommand, ActionPlanSimulationRead, ActionPlanSimulationResult,
    ActionPlanSnapshotRead, ActionPlanStatus, ActionPlanStep, ActionPlanTransitionCommand,
    ActionPlanTransitionResult, ActionPlanType, ActionPlanValidationCommand,
    ActionPlanValidationResult, AgentAuditRepository, AgentRunRead, AgentRunStart, AgentToolAudit,
    AgentToolCallRead, ApplicationError, AuthorizationAuditRepository, AuthorizationDecisionAudit,
    DatasetGroundTruth, DatasetImport, DatasetReadRepository, DispositionCreateResult,
    DispositionProposalDraft, DispositionProposalRead, DispositionProposalRepository,
    DispositionReviewCommand, DispositionReviewRead, DispositionTransitionCommand,
    DispositionTransitionResult, DomainFeedbackDraft, DomainFeedbackRead, DowntimeEvidence,
    DowntimeSummary, EvaluationTrack, EvidenceState, ExpertSandboxRepository, FactorySeedPlan,
    IdentityAccess, IdentityRepository, MesReadRepository, ProductionEvidence, ProductionSummary,
    ProductionTotals, ProposalStatus, ProposalType, QualityEvidence, QualitySummary,
    RecommendationCode, ReviewDecision, SandboxFactoryRead, SandboxLineRead, SeedRepository,
    SeedSummary, WorkOrderRead, WorkQueueFilter, WorkQueueItemRead,
};
use mes_domain::WorkOrderStatus;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Row, Transaction, postgres::PgPoolOptions};
use uuid::Uuid;

use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatasetPersistenceSummary {
    pub import_id: Uuid,
    pub record_count: usize,
}

#[derive(Debug, Clone)]
pub struct PostgresMesRepository {
    pool: PgPool,
}

impl PostgresMesRepository {
    /// # Errors
    ///
    /// Returns a repository error when PostgreSQL cannot be reached.
    pub async fn connect(database_url: &str) -> Result<Self, ApplicationError> {
        let pool = PgPoolOptions::new()
            .max_connections(10)
            .connect(database_url)
            .await
            .map_err(repository_error)?;
        Ok(Self { pool })
    }

    #[must_use]
    pub fn from_pool(pool: PgPool) -> Self {
        Self { pool }
    }

    /// # Errors
    ///
    /// Returns a repository error when a migration cannot be applied.
    pub async fn migrate(&self) -> Result<(), ApplicationError> {
        sqlx::migrate!("../../migrations")
            .run(&self.pool)
            .await
            .map_err(repository_error)
    }

    /// Persists immutable source records. Existing records with the same source
    /// identity and checksum are left unchanged.
    ///
    /// # Errors
    ///
    /// Returns a repository error when the manifest or records cannot be persisted.
    pub async fn persist_dataset_import(
        &self,
        tenant_id: Option<Uuid>,
        import: &DatasetImport,
    ) -> Result<DatasetPersistenceSummary, ApplicationError> {
        let import_key = format!(
            "{}:{}:{}",
            import.manifest.dataset_key, import.manifest.version, import.manifest.sha256
        );
        let import_id = Uuid::new_v5(&Uuid::NAMESPACE_URL, import_key.as_bytes());
        let track = match import.manifest.track {
            EvaluationTrack::PublishedSynthetic => "published_synthetic",
            EvaluationTrack::RealWorld => "real_world",
        };
        let record_count = i64::try_from(import.records.len()).map_err(repository_error)?;
        let mut transaction = self.pool.begin().await.map_err(repository_error)?;

        sqlx::query(
            r"
            INSERT INTO dataset_imports (
                id, tenant_id, dataset_key, evaluation_track, dataset_name,
                dataset_version, license, source_url, source_files, sha256, record_count
            ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)
            ON CONFLICT (dataset_key, dataset_version, sha256) DO NOTHING
            ",
        )
        .bind(import_id)
        .bind(tenant_id)
        .bind(&import.manifest.dataset_key)
        .bind(track)
        .bind(&import.manifest.name)
        .bind(&import.manifest.version)
        .bind(&import.manifest.license)
        .bind(&import.manifest.source_url)
        .bind(&import.manifest.source_files)
        .bind(&import.manifest.sha256)
        .bind(record_count)
        .execute(&mut *transaction)
        .await
        .map_err(repository_error)?;

        for record in &import.records {
            let record_key = format!("{import_id}:{}", record.source_record_id);
            let record_id = Uuid::new_v5(&Uuid::NAMESPACE_URL, record_key.as_bytes());
            sqlx::query(
                r"
                INSERT INTO dataset_source_records (
                    id, import_id, source_record_id, source_locator, raw_payload,
                    normalized_payload, missing_fields
                ) VALUES ($1,$2,$3,$4,$5,$6,$7)
                ON CONFLICT (import_id, source_record_id) DO NOTHING
                ",
            )
            .bind(record_id)
            .bind(import_id)
            .bind(&record.source_record_id)
            .bind(&record.source_locator)
            .bind(&record.raw_payload)
            .bind(&record.normalized_payload)
            .bind(&record.missing_fields)
            .execute(&mut *transaction)
            .await
            .map_err(repository_error)?;
        }

        transaction.commit().await.map_err(repository_error)?;
        Ok(DatasetPersistenceSummary {
            import_id,
            record_count: import.records.len(),
        })
    }

    #[must_use]
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }
}

#[async_trait]
impl SeedRepository for PostgresMesRepository {
    #[allow(clippy::too_many_lines)]
    async fn replace_seed_plan(
        &self,
        plan: &FactorySeedPlan,
    ) -> Result<SeedSummary, ApplicationError> {
        let mut transaction = self.pool.begin().await.map_err(repository_error)?;
        sqlx::query(
            "INSERT INTO tenants (id, name) VALUES ($1,$2) ON CONFLICT (id) DO UPDATE SET name=EXCLUDED.name",
        )
            .bind(plan.tenant.id)
            .bind(&plan.tenant.name)
            .execute(&mut *transaction)
            .await
            .map_err(repository_error)?;
        sqlx::query("DELETE FROM factories WHERE id = $1 AND tenant_id = $2")
            .bind(plan.factory.id)
            .bind(plan.tenant.id)
            .execute(&mut *transaction)
            .await
            .map_err(repository_error)?;
        sqlx::query("DELETE FROM products WHERE tenant_id = $1")
            .bind(plan.tenant.id)
            .execute(&mut *transaction)
            .await
            .map_err(repository_error)?;
        sqlx::query(
            "INSERT INTO factories (id, tenant_id, name, slug, timezone) VALUES ($1,$2,$3,$4,$5)",
        )
        .bind(plan.factory.id)
        .bind(plan.factory.tenant_id)
        .bind(&plan.factory.name)
        .bind(&plan.factory.slug)
        .bind(&plan.factory.timezone)
        .execute(&mut *transaction)
        .await
        .map_err(repository_error)?;

        for line in &plan.lines {
            sqlx::query(
                "INSERT INTO production_lines (id, tenant_id, factory_id, name) VALUES ($1,$2,$3,$4)",
            )
            .bind(line.id)
            .bind(line.tenant_id)
            .bind(line.factory_id)
            .bind(&line.name)
            .execute(&mut *transaction)
            .await
            .map_err(repository_error)?;
        }
        for machine in &plan.machines {
            sqlx::query(
                "INSERT INTO machines (id, tenant_id, factory_id, line_id, name, status) VALUES ($1,$2,$3,$4,$5,$6)",
            )
            .bind(machine.id)
            .bind(machine.tenant_id)
            .bind(machine.factory_id)
            .bind(machine.line_id)
            .bind(&machine.name)
            .bind(&machine.status)
            .execute(&mut *transaction)
            .await
            .map_err(repository_error)?;
        }
        for product in &plan.products {
            sqlx::query(
                "INSERT INTO products (id, tenant_id, sku, ideal_cycle_seconds) VALUES ($1,$2,$3,$4)",
            )
            .bind(product.id)
            .bind(product.tenant_id)
            .bind(&product.sku)
            .bind(product.ideal_cycle_seconds)
            .execute(&mut *transaction)
            .await
            .map_err(repository_error)?;
        }
        for work_order in &plan.work_orders {
            sqlx::query(
                r"
                INSERT INTO work_orders (
                    id, tenant_id, factory_id, line_id, product_id, status, planned_qty, due_at
                ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)
                ",
            )
            .bind(work_order.id)
            .bind(work_order.tenant_id)
            .bind(work_order.factory_id)
            .bind(work_order.line_id)
            .bind(work_order.product_id)
            .bind(status_name(work_order.status))
            .bind(work_order.planned_qty.get())
            .bind(work_order.due_at)
            .execute(&mut *transaction)
            .await
            .map_err(repository_error)?;
        }
        for record in &plan.production_records {
            sqlx::query(
                r"
                INSERT INTO production_records (
                    id, tenant_id, factory_id, line_id, work_order_id, machine_id,
                    occurred_at, good_qty, reject_qty
                ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)
                ",
            )
            .bind(record.id)
            .bind(record.tenant_id)
            .bind(record.factory_id)
            .bind(record.line_id)
            .bind(record.work_order_id)
            .bind(record.machine_id)
            .bind(record.occurred_at)
            .bind(record.good_qty.get())
            .bind(record.reject_qty.get())
            .execute(&mut *transaction)
            .await
            .map_err(repository_error)?;
        }
        for event in &plan.downtime_events {
            sqlx::query(
                r"
                INSERT INTO downtime_events (
                    id, tenant_id, factory_id, line_id, machine_id, started_at,
                    ended_at, reason_code, source_scenario_id
                ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)
                ",
            )
            .bind(event.id)
            .bind(event.tenant_id)
            .bind(event.factory_id)
            .bind(event.line_id)
            .bind(event.machine_id)
            .bind(event.started_at)
            .bind(event.ended_at)
            .bind(&event.reason_code)
            .bind(&event.source_scenario_id)
            .execute(&mut *transaction)
            .await
            .map_err(repository_error)?;
        }
        for inspection in &plan.quality_inspections {
            sqlx::query(
                r"
                INSERT INTO quality_inspections (
                    id, tenant_id, factory_id, line_id, work_order_id, inspected_at,
                    sample_size, defect_count, defect_code, source_scenario_id
                ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)
                ",
            )
            .bind(inspection.id)
            .bind(inspection.tenant_id)
            .bind(inspection.factory_id)
            .bind(inspection.line_id)
            .bind(inspection.work_order_id)
            .bind(inspection.inspected_at)
            .bind(inspection.sample_size.get())
            .bind(inspection.defect_count.get())
            .bind(&inspection.defect_code)
            .bind(&inspection.source_scenario_id)
            .execute(&mut *transaction)
            .await
            .map_err(repository_error)?;
        }
        let identity = &plan.development_identity;
        sqlx::query(
            "INSERT INTO users (id, display_name, status) VALUES ($1,$2,'active') \
             ON CONFLICT (id) DO UPDATE SET display_name=EXCLUDED.display_name, status='active', updated_at=now()",
        )
        .bind(identity.user_id)
        .bind("Development user")
        .execute(&mut *transaction)
        .await
        .map_err(repository_error)?;
        sqlx::query(
            "INSERT INTO external_identities (id, user_id, issuer, subject) VALUES ($1,$2,$3,$4) \
             ON CONFLICT (issuer, subject) DO UPDATE SET user_id=EXCLUDED.user_id",
        )
        .bind(identity.external_identity_id)
        .bind(identity.user_id)
        .bind(&identity.issuer)
        .bind(&identity.subject)
        .execute(&mut *transaction)
        .await
        .map_err(repository_error)?;
        sqlx::query(
            "INSERT INTO tenant_memberships (id, user_id, tenant_id, status, all_factories) \
             VALUES ($1,$2,$3,'active',false) ON CONFLICT (user_id, tenant_id) \
             DO UPDATE SET status='active', all_factories=false",
        )
        .bind(identity.membership_id)
        .bind(identity.user_id)
        .bind(plan.tenant.id)
        .execute(&mut *transaction)
        .await
        .map_err(repository_error)?;
        sqlx::query("DELETE FROM tenant_membership_roles WHERE membership_id=$1")
            .bind(identity.membership_id)
            .execute(&mut *transaction)
            .await
            .map_err(repository_error)?;
        for role in &identity.roles {
            sqlx::query("INSERT INTO tenant_membership_roles (membership_id, role) VALUES ($1,$2)")
                .bind(identity.membership_id)
                .bind(role)
                .execute(&mut *transaction)
                .await
                .map_err(repository_error)?;
        }
        sqlx::query(
            "INSERT INTO membership_factory_scopes (membership_id, tenant_id, factory_id) \
             VALUES ($1,$2,$3) ON CONFLICT DO NOTHING",
        )
        .bind(identity.membership_id)
        .bind(plan.tenant.id)
        .bind(plan.factory.id)
        .execute(&mut *transaction)
        .await
        .map_err(repository_error)?;
        transaction.commit().await.map_err(repository_error)?;

        Ok(SeedSummary {
            tenant_id: plan.tenant.id,
            factory_id: plan.factory.id,
            work_orders: plan.work_orders.len(),
            production_records: plan.production_records.len(),
            downtime_events: plan.downtime_events.len(),
            quality_inspections: plan.quality_inspections.len(),
        })
    }
}

#[async_trait]
impl IdentityRepository for PostgresMesRepository {
    async fn resolve_identity(
        &self,
        issuer: &str,
        subject: &str,
        tenant_id: Uuid,
        factory_id: Uuid,
    ) -> Result<Option<IdentityAccess>, ApplicationError> {
        let row = sqlx::query(
            r"
            SELECT u.id AS user_id, ei.id AS external_identity_id, tm.id AS membership_id,
                   tm.all_factories
            FROM external_identities ei
            JOIN users u ON u.id=ei.user_id AND u.status='active'
            JOIN tenant_memberships tm ON tm.user_id=u.id AND tm.status='active'
            JOIN factories f ON f.id=$4 AND f.tenant_id=tm.tenant_id
            LEFT JOIN membership_factory_scopes mfs
              ON mfs.membership_id=tm.id AND mfs.tenant_id=tm.tenant_id AND mfs.factory_id=f.id
            WHERE ei.issuer=$1 AND ei.subject=$2 AND tm.tenant_id=$3
              AND (tm.all_factories OR mfs.factory_id IS NOT NULL)
            ",
        )
        .bind(issuer)
        .bind(subject)
        .bind(tenant_id)
        .bind(factory_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(repository_error)?;
        let Some(row) = row else { return Ok(None) };
        let membership_id: Uuid = row.get("membership_id");
        let roles = sqlx::query_scalar::<_, String>(
            "SELECT role FROM tenant_membership_roles WHERE membership_id=$1 ORDER BY role",
        )
        .bind(membership_id)
        .fetch_all(&self.pool)
        .await
        .map_err(repository_error)?;
        Ok(Some(IdentityAccess {
            user_id: row.get("user_id"),
            external_identity_id: row.get("external_identity_id"),
            tenant_id,
            permitted_factory_ids: vec![factory_id],
            roles,
        }))
    }
}

#[async_trait]
impl AuthorizationAuditRepository for PostgresMesRepository {
    async fn record_authorization_decision(
        &self,
        audit: &AuthorizationDecisionAudit,
    ) -> Result<(), ApplicationError> {
        sqlx::query(
            r"
            INSERT INTO authorization_decision_audits (
                id, request_id, user_id, external_identity_id, tenant_id, factory_id,
                auth_method, issuer, roles, capabilities, action, resource_type,
                resource_id, outcome, denial_category, duration_ms
            ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16)
            ",
        )
        .bind(audit.id)
        .bind(audit.request_id)
        .bind(audit.user_id)
        .bind(audit.external_identity_id)
        .bind(audit.tenant_id)
        .bind(audit.factory_id)
        .bind(&audit.auth_method)
        .bind(&audit.issuer)
        .bind(&audit.roles)
        .bind(&audit.capabilities)
        .bind(&audit.action)
        .bind(&audit.resource_type)
        .bind(&audit.resource_id)
        .bind(&audit.outcome)
        .bind(&audit.denial_category)
        .bind(audit.duration_ms)
        .execute(&self.pool)
        .await
        .map_err(repository_error)?;
        Ok(())
    }
}

#[async_trait]
impl MesReadRepository for PostgresMesRepository {
    async fn get_work_order(
        &self,
        tenant_id: Uuid,
        factory_id: Uuid,
        work_order_id: Uuid,
    ) -> Result<Option<WorkOrderRead>, ApplicationError> {
        let row = sqlx::query(
            r"
            SELECT wo.id, wo.factory_id, wo.line_id, wo.product_id, wo.status,
                   wo.planned_qty, wo.due_at,
                   COALESCE(SUM(pr.good_qty), 0)::BIGINT AS completed_good_qty,
                   COALESCE(SUM(pr.reject_qty), 0)::BIGINT AS reject_qty,
                   COALESCE(
                     array_agg('production_record:' || pr.id::text)
                       FILTER (WHERE pr.id IS NOT NULL),
                     ARRAY[]::TEXT[]
                   ) AS evidence
            FROM work_orders wo
            LEFT JOIN production_records pr
              ON pr.work_order_id = wo.id AND pr.tenant_id = wo.tenant_id
            WHERE wo.id = $1 AND wo.tenant_id = $2 AND wo.factory_id = $3
            GROUP BY wo.id
            ",
        )
        .bind(work_order_id)
        .bind(tenant_id)
        .bind(factory_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(repository_error)?;

        row.map(|row| {
            Ok(WorkOrderRead {
                id: row.get("id"),
                factory_id: row.get("factory_id"),
                line_id: row.get("line_id"),
                product_id: row.get("product_id"),
                status: parse_status(row.get::<String, _>("status").as_str())?,
                planned_qty: row.get("planned_qty"),
                completed_good_qty: row.get("completed_good_qty"),
                reject_qty: row.get("reject_qty"),
                due_at: row.get("due_at"),
                evidence: row.get("evidence"),
            })
        })
        .transpose()
    }

    #[allow(clippy::cast_precision_loss)]
    async fn production_summary(
        &self,
        tenant_id: Uuid,
        factory_id: Uuid,
        line_id: Uuid,
        start_at: DateTime<Utc>,
        end_at: DateTime<Utc>,
        limit: i64,
    ) -> Result<Option<ProductionSummary>, ApplicationError> {
        if !line_in_scope(&self.pool, tenant_id, factory_id, line_id).await? {
            return Ok(None);
        }
        let rows = sqlx::query(
            r"
            SELECT id, occurred_at, good_qty, reject_qty
            FROM production_records
            WHERE tenant_id=$1 AND factory_id=$2 AND line_id=$3
              AND occurred_at >= $4 AND occurred_at < $5
            ORDER BY occurred_at, id
            LIMIT $6
            ",
        )
        .bind(tenant_id)
        .bind(factory_id)
        .bind(line_id)
        .bind(start_at)
        .bind(end_at)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(repository_error)?;
        let records: Vec<_> = rows
            .into_iter()
            .map(|row| ProductionEvidence {
                record_id: row.get("id"),
                occurred_at: row.get("occurred_at"),
                good_qty: row.get("good_qty"),
                reject_qty: row.get("reject_qty"),
            })
            .collect();
        let good_qty = records.iter().map(|record| record.good_qty).sum();
        let reject_qty = records.iter().map(|record| record.reject_qty).sum();
        let total = good_qty + reject_qty;
        let hours = (end_at - start_at).num_seconds() as f64 / 3_600.0;
        Ok(Some(ProductionSummary {
            line_id,
            start_at,
            end_at,
            good_qty,
            reject_qty,
            throughput_good_units_per_hour: good_qty as f64 / hours,
            scrap_rate: (total > 0).then_some(reject_qty as f64 / total as f64),
            missing_data: records.is_empty(),
            records,
        }))
    }

    async fn downtime_summary(
        &self,
        tenant_id: Uuid,
        factory_id: Uuid,
        line_id: Uuid,
        start_at: DateTime<Utc>,
        end_at: DateTime<Utc>,
        limit: i64,
    ) -> Result<Option<DowntimeSummary>, ApplicationError> {
        if !line_in_scope(&self.pool, tenant_id, factory_id, line_id).await? {
            return Ok(None);
        }
        let rows = sqlx::query(
            r"
            SELECT id, machine_id, started_at, ended_at, reason_code
            FROM downtime_events
            WHERE tenant_id=$1 AND factory_id=$2 AND line_id=$3
              AND started_at < $5 AND ended_at > $4
            ORDER BY started_at, id
            LIMIT $6
            ",
        )
        .bind(tenant_id)
        .bind(factory_id)
        .bind(line_id)
        .bind(start_at)
        .bind(end_at)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(repository_error)?;
        let events: Vec<_> = rows
            .into_iter()
            .map(|row| DowntimeEvidence {
                event_id: row.get("id"),
                machine_id: row.get("machine_id"),
                started_at: row.get("started_at"),
                ended_at: row.get("ended_at"),
                reason_code: row.get("reason_code"),
            })
            .collect();
        let line_downtime_minutes = union_minutes(&events, start_at, end_at);
        Ok(Some(DowntimeSummary {
            line_id,
            start_at,
            end_at,
            line_downtime_minutes,
            missing_data: events.is_empty(),
            events,
        }))
    }

    #[allow(clippy::cast_precision_loss)]
    async fn quality_summary(
        &self,
        tenant_id: Uuid,
        factory_id: Uuid,
        line_id: Uuid,
        start_at: DateTime<Utc>,
        end_at: DateTime<Utc>,
        limit: i64,
    ) -> Result<Option<QualitySummary>, ApplicationError> {
        if !line_in_scope(&self.pool, tenant_id, factory_id, line_id).await? {
            return Ok(None);
        }
        let rows = sqlx::query(
            r"
            SELECT id, inspected_at, sample_size, defect_count, defect_code
            FROM quality_inspections
            WHERE tenant_id=$1 AND factory_id=$2 AND line_id=$3
              AND inspected_at >= $4 AND inspected_at < $5
            ORDER BY inspected_at, id
            LIMIT $6
            ",
        )
        .bind(tenant_id)
        .bind(factory_id)
        .bind(line_id)
        .bind(start_at)
        .bind(end_at)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(repository_error)?;
        let inspections: Vec<_> = rows
            .into_iter()
            .map(|row| QualityEvidence {
                inspection_id: row.get("id"),
                inspected_at: row.get("inspected_at"),
                sample_size: row.get("sample_size"),
                defect_count: row.get("defect_count"),
                defect_code: row.get("defect_code"),
            })
            .collect();
        let sample_size = inspections.iter().map(|item| item.sample_size).sum();
        let defect_count = inspections.iter().map(|item| item.defect_count).sum();
        Ok(Some(QualitySummary {
            line_id,
            start_at,
            end_at,
            sample_size,
            defect_count,
            qc_sample_defect_rate: (sample_size > 0)
                .then_some(defect_count as f64 / sample_size as f64),
            missing_data: inspections.is_empty(),
            inspections,
        }))
    }

    async fn production_totals(
        &self,
        tenant_id: Uuid,
        factory_id: Uuid,
        line_id: Uuid,
        start_at: DateTime<Utc>,
        end_at: DateTime<Utc>,
    ) -> Result<ProductionTotals, ApplicationError> {
        let row = sqlx::query(
            r"
            SELECT
                COALESCE(SUM(good_qty), 0)::BIGINT AS good_qty,
                COALESCE(SUM(reject_qty), 0)::BIGINT AS reject_qty,
                COUNT(*)::BIGINT AS record_count
            FROM production_records
            WHERE tenant_id = $1 AND factory_id = $2 AND line_id = $3
              AND occurred_at >= $4 AND occurred_at < $5
            ",
        )
        .bind(tenant_id)
        .bind(factory_id)
        .bind(line_id)
        .bind(start_at)
        .bind(end_at)
        .fetch_one(&self.pool)
        .await
        .map_err(repository_error)?;

        Ok(ProductionTotals {
            good_qty: row.get("good_qty"),
            reject_qty: row.get("reject_qty"),
            record_count: row.get("record_count"),
        })
    }
}

#[async_trait]
impl DatasetReadRepository for PostgresMesRepository {
    async fn dataset_ground_truth(
        &self,
        dataset_key: &str,
    ) -> Result<Option<DatasetGroundTruth>, ApplicationError> {
        let Some(import) = sqlx::query(
            "SELECT id, record_count FROM dataset_imports WHERE dataset_key=$1 ORDER BY imported_at DESC LIMIT 1",
        )
        .bind(dataset_key)
        .fetch_optional(&self.pool)
        .await
        .map_err(repository_error)?
        else {
            return Ok(None);
        };
        let import_id: Uuid = import.get("id");
        let record_count: i64 = import.get("record_count");
        let records_with_missing: i64 = sqlx::query_scalar(
            "SELECT COUNT(*)::BIGINT FROM dataset_source_records WHERE import_id=$1 AND cardinality(missing_fields)>0",
        )
        .bind(import_id)
        .fetch_one(&self.pool)
        .await
        .map_err(repository_error)?;
        let outcome_counts = if dataset_key == "uci-ai4i-2020" {
            let failures: i64 = sqlx::query_scalar(
                "SELECT COUNT(*)::BIGINT FROM dataset_source_records WHERE import_id=$1 AND normalized_payload->>'machine_failure'='1'",
            )
            .bind(import_id)
            .fetch_one(&self.pool)
            .await
            .map_err(repository_error)?;
            serde_json::json!({"machine_failure": failures, "no_machine_failure": record_count - failures})
        } else if dataset_key == "uci-secom" {
            let failures: i64 = sqlx::query_scalar(
                "SELECT COUNT(*)::BIGINT FROM dataset_source_records WHERE import_id=$1 AND normalized_payload->>'line_test_result'='fail'",
            )
            .bind(import_id)
            .fetch_one(&self.pool)
            .await
            .map_err(repository_error)?;
            serde_json::json!({"fail": failures, "pass": record_count - failures})
        } else {
            serde_json::json!({})
        };
        let evidence = sqlx::query_scalar::<_, String>(
            "SELECT 'dataset_record:' || source_record_id FROM dataset_source_records WHERE import_id=$1 ORDER BY source_record_id LIMIT 10",
        )
        .bind(import_id)
        .fetch_all(&self.pool)
        .await
        .map_err(repository_error)?;
        Ok(Some(DatasetGroundTruth {
            dataset_key: dataset_key.to_owned(),
            import_id,
            record_count,
            outcome_counts,
            records_with_missing,
            evidence,
        }))
    }
}

#[async_trait]
impl AgentAuditRepository for PostgresMesRepository {
    async fn start_run(&self, run: &AgentRunStart) -> Result<(), ApplicationError> {
        sqlx::query(
            "INSERT INTO agent_runs (id, tenant_id, factory_id, user_id, agent_id, provider_name, data_policy, status) VALUES ($1,$2,$3,$4,$5,$6,$7,'running')",
        )
        .bind(run.id)
        .bind(run.tenant_id)
        .bind(run.factory_id)
        .bind(&run.user_id)
        .bind(&run.agent_id)
        .bind(&run.provider_name)
        .bind(&run.data_policy)
        .execute(&self.pool)
        .await
        .map_err(repository_error)?;
        Ok(())
    }

    async fn record_tool_call(&self, audit: &AgentToolAudit) -> Result<(), ApplicationError> {
        sqlx::query(
            r"
            INSERT INTO agent_tool_calls (
                id, run_id, tool_name, redacted_arguments, auth_decision,
                result_summary, duration_ms
            ) VALUES ($1,$2,$3,$4,$5,$6,$7)
            ",
        )
        .bind(audit.id)
        .bind(audit.run_id)
        .bind(&audit.tool_name)
        .bind(&audit.redacted_arguments)
        .bind(&audit.auth_decision)
        .bind(&audit.result_summary)
        .bind(audit.duration_ms)
        .execute(&self.pool)
        .await
        .map_err(repository_error)?;
        Ok(())
    }

    async fn finish_run(
        &self,
        run_id: Uuid,
        status: &str,
        final_response: Option<&str>,
        error_code: Option<&str>,
    ) -> Result<(), ApplicationError> {
        sqlx::query(
            "UPDATE agent_runs SET status=$2, final_response=$3, error_code=$4, finished_at=now() WHERE id=$1",
        )
        .bind(run_id)
        .bind(status)
        .bind(final_response)
        .bind(error_code)
        .execute(&self.pool)
        .await
        .map_err(repository_error)?;
        Ok(())
    }

    async fn get_run(
        &self,
        tenant_id: Uuid,
        run_id: Uuid,
    ) -> Result<Option<AgentRunRead>, ApplicationError> {
        let row = sqlx::query(
            "SELECT id, tenant_id, factory_id, user_id, agent_id, provider_name, data_policy, status, started_at, finished_at, \
             error_code, final_response FROM agent_runs WHERE id=$1 AND tenant_id=$2",
        )
        .bind(run_id)
        .bind(tenant_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(repository_error)?;
        Ok(row.map(|row| AgentRunRead {
            id: row.get("id"),
            tenant_id: row.get("tenant_id"),
            factory_id: row.get("factory_id"),
            user_id: row.get("user_id"),
            agent_id: row.get("agent_id"),
            provider_name: row.get("provider_name"),
            data_policy: row.get("data_policy"),
            status: row.get("status"),
            started_at: row.get("started_at"),
            finished_at: row.get("finished_at"),
            error_code: row.get("error_code"),
            final_response: row.get("final_response"),
        }))
    }

    async fn get_tool_calls(
        &self,
        tenant_id: Uuid,
        run_id: Uuid,
    ) -> Result<Vec<AgentToolCallRead>, ApplicationError> {
        let rows = sqlx::query(
            "SELECT c.id, c.run_id, c.tool_name, c.redacted_arguments, c.auth_decision, \
             c.result_summary, c.duration_ms, c.created_at \
             FROM agent_tool_calls c JOIN agent_runs r ON r.id=c.run_id \
             WHERE c.run_id=$1 AND r.tenant_id=$2 ORDER BY c.created_at, c.id",
        )
        .bind(run_id)
        .bind(tenant_id)
        .fetch_all(&self.pool)
        .await
        .map_err(repository_error)?;
        Ok(rows
            .into_iter()
            .map(|row| AgentToolCallRead {
                id: row.get("id"),
                run_id: row.get("run_id"),
                tool_name: row.get("tool_name"),
                redacted_arguments: row.get("redacted_arguments"),
                auth_decision: row.get("auth_decision"),
                result_summary: row.get("result_summary"),
                duration_ms: row.get("duration_ms"),
                created_at: row.get("created_at"),
            })
            .collect())
    }
}

#[async_trait]
impl DispositionProposalRepository for PostgresMesRepository {
    #[allow(clippy::too_many_lines)]
    async fn create_disposition_draft(
        &self,
        draft: &DispositionProposalDraft,
        idempotency_key: &str,
        request_fingerprint: &str,
    ) -> Result<DispositionCreateResult, ApplicationError> {
        let operation = "disposition_proposal.create";
        let lock_key = format!(
            "{}:{}:{operation}:{idempotency_key}",
            draft.tenant_id, draft.initiated_by_user_id
        );
        let mut transaction = self.pool.begin().await.map_err(repository_error)?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(&lock_key)
            .execute(&mut *transaction)
            .await
            .map_err(repository_error)?;
        let request_hash: String =
            sqlx::query_scalar("SELECT encode(digest(convert_to($1, 'UTF8'), 'sha256'), 'hex')")
                .bind(request_fingerprint)
                .fetch_one(&mut *transaction)
                .await
                .map_err(repository_error)?;
        let existing = sqlx::query(
            "SELECT request_hash, resource_id FROM idempotency_records \
             WHERE tenant_id=$1 AND actor_user_id=$2 AND operation=$3 AND idempotency_key=$4",
        )
        .bind(draft.tenant_id)
        .bind(draft.initiated_by_user_id)
        .bind(operation)
        .bind(idempotency_key)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(repository_error)?;
        let (proposal_id, replayed) = if let Some(existing) = existing {
            if existing.get::<String, _>("request_hash") != request_hash {
                return Err(ApplicationError::Conflict(
                    "idempotency key was already used with a different request".to_owned(),
                ));
            }
            (existing.get("resource_id"), true)
        } else {
            let inserted = sqlx::query(
                r"
                INSERT INTO disposition_proposals (
                    id, tenant_id, factory_id, initiated_by_user_id, source_run_id,
                    proposal_type, status, evidence_state, summary, expires_at
                )
                SELECT $1,$2,$3,$4,$5,$6,'draft',$7,$8,$9
                FROM agent_runs r
                WHERE r.id=$5 AND r.tenant_id=$2 AND r.factory_id=$3
                  AND r.status='completed'
                ",
            )
            .bind(draft.id)
            .bind(draft.tenant_id)
            .bind(draft.factory_id)
            .bind(draft.initiated_by_user_id)
            .bind(draft.source_run_id)
            .bind(draft.proposal_type.as_str())
            .bind(draft.evidence_state.as_str())
            .bind(&draft.summary)
            .bind(draft.expires_at)
            .execute(&mut *transaction)
            .await
            .map_err(repository_error)?;
            if inserted.rows_affected() != 1 {
                return Err(ApplicationError::Precondition(
                    "source run must be completed in the selected tenant and factory".to_owned(),
                ));
            }
            for (ordinal, target) in draft.target_refs.iter().enumerate() {
                let ordinal = i32::try_from(ordinal).map_err(repository_error)?;
                sqlx::query(
                    "INSERT INTO disposition_proposal_targets \
                     (proposal_id, ordinal, resource_type, resource_id) VALUES ($1,$2,$3,$4)",
                )
                .bind(draft.id)
                .bind(ordinal)
                .bind(&target.resource_type)
                .bind(&target.resource_id)
                .execute(&mut *transaction)
                .await
                .map_err(repository_error)?;
            }
            for (ordinal, evidence) in draft.evidence_refs.iter().enumerate() {
                let ordinal = i32::try_from(ordinal).map_err(repository_error)?;
                sqlx::query(
                    "INSERT INTO disposition_proposal_evidence \
                     (proposal_id, ordinal, evidence_type, evidence_id, source_run_id, \
                      agent_tool_call_id) VALUES ($1,$2,$3,$4,$5,$6)",
                )
                .bind(draft.id)
                .bind(ordinal)
                .bind(&evidence.tool_name)
                .bind(evidence.tool_call_id.to_string())
                .bind(draft.source_run_id)
                .bind(evidence.tool_call_id)
                .execute(&mut *transaction)
                .await
                .map_err(repository_error)?;
            }
            for (ordinal, action) in draft.recommended_actions.iter().enumerate() {
                let ordinal = i32::try_from(ordinal).map_err(repository_error)?;
                sqlx::query(
                    "INSERT INTO disposition_proposal_actions \
                     (proposal_id, ordinal, recommendation_code) VALUES ($1,$2,$3)",
                )
                .bind(draft.id)
                .bind(ordinal)
                .bind(action.as_str())
                .execute(&mut *transaction)
                .await
                .map_err(repository_error)?;
            }
            sqlx::query(
                r"
                INSERT INTO idempotency_records (
                    id, tenant_id, actor_user_id, operation, idempotency_key,
                    request_hash, resource_type, resource_id, response_status
                ) VALUES ($1,$2,$3,$4,$5,$6,'disposition_proposal',$7,201)
                ",
            )
            .bind(Uuid::new_v4())
            .bind(draft.tenant_id)
            .bind(draft.initiated_by_user_id)
            .bind(operation)
            .bind(idempotency_key)
            .bind(&request_hash)
            .bind(draft.id)
            .execute(&mut *transaction)
            .await
            .map_err(repository_error)?;
            (draft.id, false)
        };
        transaction.commit().await.map_err(repository_error)?;
        let proposal = self
            .get_disposition_proposal(draft.tenant_id, draft.factory_id, proposal_id)
            .await?
            .ok_or_else(|| {
                ApplicationError::Repository(
                    "persisted disposition proposal could not be read".to_owned(),
                )
            })?;
        Ok(DispositionCreateResult { proposal, replayed })
    }

    async fn get_disposition_proposal(
        &self,
        tenant_id: Uuid,
        factory_id: Uuid,
        proposal_id: Uuid,
    ) -> Result<Option<DispositionProposalRead>, ApplicationError> {
        let row = sqlx::query(
            r"
            SELECT id, version, tenant_id, factory_id, initiated_by_user_id,
                   source_run_id, supersedes_proposal_id, proposal_type, status,
                   evidence_state, summary, expires_at, created_at, submitted_at,
                   finished_at
            FROM disposition_proposals
            WHERE id=$1 AND tenant_id=$2 AND factory_id=$3
            ",
        )
        .bind(proposal_id)
        .bind(tenant_id)
        .bind(factory_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(repository_error)?;
        let Some(row) = row else { return Ok(None) };
        let action_values = sqlx::query_scalar::<_, String>(
            "SELECT recommendation_code FROM disposition_proposal_actions \
             WHERE proposal_id=$1 ORDER BY ordinal",
        )
        .bind(proposal_id)
        .fetch_all(&self.pool)
        .await
        .map_err(repository_error)?;
        let recommended_actions = action_values
            .into_iter()
            .map(|value| {
                value
                    .parse::<RecommendationCode>()
                    .map_err(repository_error)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let target_refs = sqlx::query(
            "SELECT resource_type, resource_id FROM disposition_proposal_targets \
             WHERE proposal_id=$1 ORDER BY ordinal",
        )
        .bind(proposal_id)
        .fetch_all(&self.pool)
        .await
        .map_err(repository_error)?
        .into_iter()
        .map(|row| mes_application::DispositionTargetRef {
            resource_type: row.get("resource_type"),
            resource_id: row.get("resource_id"),
        })
        .collect();
        let evidence_refs = sqlx::query(
            "SELECT evidence_type, agent_tool_call_id \
             FROM disposition_proposal_evidence WHERE proposal_id=$1 ORDER BY ordinal",
        )
        .bind(proposal_id)
        .fetch_all(&self.pool)
        .await
        .map_err(repository_error)?
        .into_iter()
        .map(|row| mes_application::DispositionEvidenceRef {
            tool_call_id: row.get("agent_tool_call_id"),
            tool_name: row.get("evidence_type"),
        })
        .collect();
        let proposal_type = row
            .get::<String, _>("proposal_type")
            .parse::<ProposalType>()
            .map_err(repository_error)?;
        let status = row
            .get::<String, _>("status")
            .parse::<ProposalStatus>()
            .map_err(repository_error)?;
        let evidence_state = row
            .get::<String, _>("evidence_state")
            .parse::<EvidenceState>()
            .map_err(repository_error)?;
        Ok(Some(DispositionProposalRead {
            id: row.get("id"),
            version: row.get("version"),
            tenant_id: row.get("tenant_id"),
            factory_id: row.get("factory_id"),
            initiated_by_user_id: row.get("initiated_by_user_id"),
            source_run_id: row.get("source_run_id"),
            supersedes_proposal_id: row.get("supersedes_proposal_id"),
            proposal_type,
            status,
            target_refs,
            evidence_refs,
            evidence_state,
            recommended_actions,
            summary: row.get("summary"),
            expires_at: row.get("expires_at"),
            created_at: row.get("created_at"),
            submitted_at: row.get("submitted_at"),
            finished_at: row.get("finished_at"),
        }))
    }

    async fn expire_disposition_proposal(
        &self,
        tenant_id: Uuid,
        factory_id: Uuid,
        proposal_id: Uuid,
    ) -> Result<Option<DispositionProposalRead>, ApplicationError> {
        expire_pending_proposal(&self.pool, tenant_id, factory_id, proposal_id).await?;
        self.get_disposition_proposal(tenant_id, factory_id, proposal_id)
            .await
    }

    async fn submit_disposition_proposal(
        &self,
        command: &DispositionTransitionCommand,
    ) -> Result<DispositionTransitionResult, ApplicationError> {
        let operation = "disposition_proposal.submit";
        let mut transaction = self.pool.begin().await.map_err(repository_error)?;
        if idempotency_replay(&mut transaction, command, operation).await? {
            transaction.commit().await.map_err(repository_error)?;
            return transition_result(self, command, false, true).await;
        }
        let row = lock_transition_row(&mut transaction, command).await?;
        validate_actor_version_status(&row, command, true, &[ProposalStatus::Draft])?;
        if row.get::<DateTime<Utc>, _>("expires_at") <= Utc::now() {
            return Err(ApplicationError::Precondition(
                "expired draft cannot be submitted".to_owned(),
            ));
        }
        let prior_version: i64 = row.get("version");
        let source_run_id: Uuid = row.get("source_run_id");
        let evidence_count =
            transition_evidence_count(&mut transaction, command.proposal_id).await?;
        sqlx::query(
            "UPDATE disposition_proposals SET status='pending_review', version=version+1, \
             submitted_at=now() WHERE id=$1",
        )
        .bind(command.proposal_id)
        .execute(&mut *transaction)
        .await
        .map_err(repository_error)?;
        record_idempotency(&mut transaction, command, operation).await?;
        record_transition(
            &mut transaction,
            command,
            source_run_id,
            "submit",
            ProposalStatus::Draft,
            ProposalStatus::PendingReview,
            prior_version,
            evidence_count,
        )
        .await?;
        transaction.commit().await.map_err(repository_error)?;
        transition_result(self, command, false, false).await
    }

    async fn cancel_disposition_proposal(
        &self,
        command: &DispositionTransitionCommand,
    ) -> Result<DispositionTransitionResult, ApplicationError> {
        let operation = "disposition_proposal.cancel";
        let mut transaction = self.pool.begin().await.map_err(repository_error)?;
        if idempotency_replay(&mut transaction, command, operation).await? {
            transaction.commit().await.map_err(repository_error)?;
            return transition_result(self, command, false, true).await;
        }
        let row = lock_transition_row(&mut transaction, command).await?;
        validate_actor_version_status(
            &row,
            command,
            true,
            &[ProposalStatus::Draft, ProposalStatus::PendingReview],
        )?;
        let prior_status = row
            .get::<String, _>("status")
            .parse::<ProposalStatus>()
            .map_err(repository_error)?;
        let prior_version: i64 = row.get("version");
        let source_run_id: Uuid = row.get("source_run_id");
        let evidence_count =
            transition_evidence_count(&mut transaction, command.proposal_id).await?;
        sqlx::query(
            "UPDATE disposition_proposals SET status='cancelled', version=version+1, \
             finished_at=now() WHERE id=$1",
        )
        .bind(command.proposal_id)
        .execute(&mut *transaction)
        .await
        .map_err(repository_error)?;
        record_idempotency(&mut transaction, command, operation).await?;
        record_transition(
            &mut transaction,
            command,
            source_run_id,
            "cancel",
            prior_status,
            ProposalStatus::Cancelled,
            prior_version,
            evidence_count,
        )
        .await?;
        transaction.commit().await.map_err(repository_error)?;
        transition_result(self, command, false, false).await
    }

    async fn review_disposition_proposal(
        &self,
        command: &DispositionReviewCommand,
    ) -> Result<DispositionTransitionResult, ApplicationError> {
        let operation = "disposition_proposal.review";
        let transition = &command.transition;
        let mut transaction = self.pool.begin().await.map_err(repository_error)?;
        if idempotency_replay(&mut transaction, transition, operation).await? {
            transaction.commit().await.map_err(repository_error)?;
            return transition_result(self, transition, true, true).await;
        }
        let row = lock_transition_row(&mut transaction, transition).await?;
        validate_actor_version_status(&row, transition, false, &[ProposalStatus::PendingReview])?;
        let prior_version: i64 = row.get("version");
        let source_run_id: Uuid = row.get("source_run_id");
        let resulting_status = command.decision.resulting_status();
        let review_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO disposition_reviews \
             (id, proposal_id, proposal_version, reviewer_user_id, decision, rationale, \
              tenant_id, factory_id) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)",
        )
        .bind(review_id)
        .bind(transition.proposal_id)
        .bind(prior_version)
        .bind(transition.actor_user_id)
        .bind(command.decision.as_str())
        .bind(&command.rationale)
        .bind(transition.tenant_id)
        .bind(transition.factory_id)
        .execute(&mut *transaction)
        .await
        .map_err(repository_error)?;
        sqlx::query(
            "UPDATE disposition_proposals SET status=$2, version=version+1, \
             finished_at=now() WHERE id=$1",
        )
        .bind(transition.proposal_id)
        .bind(resulting_status.as_str())
        .execute(&mut *transaction)
        .await
        .map_err(repository_error)?;
        let evidence_count =
            transition_evidence_count(&mut transaction, transition.proposal_id).await?;
        record_idempotency(&mut transaction, transition, operation).await?;
        record_transition(
            &mut transaction,
            transition,
            source_run_id,
            "review",
            ProposalStatus::PendingReview,
            resulting_status,
            prior_version,
            evidence_count,
        )
        .await?;
        transaction.commit().await.map_err(repository_error)?;
        transition_result(self, transition, true, false).await
    }
}

#[async_trait]
impl ActionPlanRepository for PostgresMesRepository {
    #[allow(clippy::too_many_lines)]
    async fn create_action_plan_draft(
        &self,
        draft: &ActionPlanDraft,
        idempotency_key: &str,
        request_fingerprint: &str,
    ) -> Result<ActionPlanCreateResult, ApplicationError> {
        if draft.steps.is_empty() || draft.steps.len() > 16 {
            return Err(ApplicationError::Precondition(
                "compiled action-plan steps are missing or exceed the persistence bound".to_owned(),
            ));
        }
        for step in &draft.steps {
            draft
                .plan_type
                .validate_step(*step)
                .map_err(repository_error)?;
        }
        if draft.compiler_policy_version.is_empty() || draft.compiler_policy_version.len() > 64 {
            return Err(ApplicationError::Precondition(
                "compiler policy version is missing or exceeds the persistence bound".to_owned(),
            ));
        }
        let operation = "action_plan.create";
        let lock_key = format!(
            "{}:{}:{operation}:{idempotency_key}",
            draft.tenant_id, draft.created_by_user_id
        );
        let request_hash = fingerprint_hash(request_fingerprint);
        let mut transaction = self.pool.begin().await.map_err(repository_error)?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(lock_key)
            .execute(&mut *transaction)
            .await
            .map_err(repository_error)?;
        let existing = sqlx::query(
            "SELECT request_hash, resource_id FROM idempotency_records \
             WHERE tenant_id=$1 AND actor_user_id=$2 AND operation=$3 AND idempotency_key=$4",
        )
        .bind(draft.tenant_id)
        .bind(draft.created_by_user_id)
        .bind(operation)
        .bind(idempotency_key)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(repository_error)?;
        let (plan_id, replayed) = if let Some(existing) = existing {
            if existing.get::<String, _>("request_hash") != request_hash {
                return Err(ApplicationError::Conflict(
                    "idempotency key was already used with a different request".to_owned(),
                ));
            }
            (existing.get("resource_id"), true)
        } else {
            let inserted = sqlx::query(
                "INSERT INTO action_plans (\
                    id, tenant_id, factory_id, created_by_user_id, source_proposal_id, \
                    source_proposal_version, source_review_id, plan_type, status, \
                    source_evidence_state, compiler_policy_version, compiled_at, summary, \
                    expires_at\
                 ) \
                 SELECT $1,$2,$3,$4,p.id,p.version,r.id,$6,'draft',$7,$8,$9,$10,$11 \
                 FROM disposition_proposals p \
                 JOIN disposition_reviews r ON r.proposal_id=p.id \
                    AND r.tenant_id=p.tenant_id AND r.factory_id=p.factory_id \
                 WHERE p.id=$5 AND p.tenant_id=$2 AND p.factory_id=$3 \
                   AND p.version=$12 AND p.status='review_approved' \
                   AND r.decision='approved'",
            )
            .bind(draft.id)
            .bind(draft.tenant_id)
            .bind(draft.factory_id)
            .bind(draft.created_by_user_id)
            .bind(draft.source_proposal_id)
            .bind(draft.plan_type.as_str())
            .bind(draft.source_evidence_state.as_str())
            .bind(&draft.compiler_policy_version)
            .bind(draft.compiled_at)
            .bind(&draft.summary)
            .bind(draft.expires_at)
            .bind(draft.source_proposal_version)
            .execute(&mut *transaction)
            .await
            .map_err(repository_error)?;
            if inserted.rows_affected() != 1 {
                return Err(ApplicationError::Precondition(
                    "source proposal must be review-approved in the selected scope".to_owned(),
                ));
            }
            sqlx::query(
                "INSERT INTO action_plan_targets (plan_id, ordinal, resource_type, resource_id) \
                 SELECT $1, ordinal, resource_type, resource_id \
                 FROM disposition_proposal_targets WHERE proposal_id=$2 ORDER BY ordinal",
            )
            .bind(draft.id)
            .bind(draft.source_proposal_id)
            .execute(&mut *transaction)
            .await
            .map_err(repository_error)?;
            let step_codes = draft
                .steps
                .iter()
                .map(|step| step.as_str().to_owned())
                .collect::<Vec<_>>();
            sqlx::query(
                "INSERT INTO action_plan_steps (plan_id, ordinal, step_code) \
                 SELECT $1, (ordinality - 1)::INTEGER, step_code \
                 FROM unnest($2::TEXT[]) WITH ORDINALITY AS compiled(step_code, ordinality)",
            )
            .bind(draft.id)
            .bind(step_codes)
            .execute(&mut *transaction)
            .await
            .map_err(repository_error)?;
            sqlx::query(
                "INSERT INTO action_plan_evidence (\
                    plan_id, ordinal, source_proposal_id, source_run_id, \
                    agent_tool_call_id, evidence_type\
                 ) \
                 SELECT $1, ordinal, $2, source_run_id, agent_tool_call_id, evidence_type \
                 FROM disposition_proposal_evidence WHERE proposal_id=$2 ORDER BY ordinal",
            )
            .bind(draft.id)
            .bind(draft.source_proposal_id)
            .execute(&mut *transaction)
            .await
            .map_err(repository_error)?;
            sqlx::query(
                "INSERT INTO idempotency_records (\
                    id, tenant_id, actor_user_id, operation, idempotency_key, request_hash, \
                    resource_type, resource_id, response_status\
                 ) VALUES ($1,$2,$3,$4,$5,$6,'action_plan',$7,201)",
            )
            .bind(Uuid::new_v4())
            .bind(draft.tenant_id)
            .bind(draft.created_by_user_id)
            .bind(operation)
            .bind(idempotency_key)
            .bind(request_hash)
            .bind(draft.id)
            .execute(&mut *transaction)
            .await
            .map_err(repository_error)?;
            (draft.id, false)
        };
        transaction.commit().await.map_err(repository_error)?;
        let plan = self
            .get_action_plan(draft.tenant_id, draft.factory_id, plan_id)
            .await?
            .ok_or_else(|| {
                ApplicationError::Repository("persisted action plan could not be read".to_owned())
            })?;
        Ok(ActionPlanCreateResult { plan, replayed })
    }

    async fn get_action_plan(
        &self,
        tenant_id: Uuid,
        factory_id: Uuid,
        plan_id: Uuid,
    ) -> Result<Option<ActionPlanRead>, ApplicationError> {
        let row = sqlx::query(
            "SELECT id, version, tenant_id, factory_id, created_by_user_id, \
                    source_proposal_id, source_proposal_version, source_review_id, \
                    plan_type, status, source_snapshot_hash, latest_simulation_id, \
                    source_evidence_state, compiler_policy_version, compiled_at, summary, \
                    expires_at, created_at, validated_at, submitted_at, finished_at \
             FROM action_plans WHERE id=$1 AND tenant_id=$2 AND factory_id=$3",
        )
        .bind(plan_id)
        .bind(tenant_id)
        .bind(factory_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(repository_error)?;
        let Some(row) = row else { return Ok(None) };
        let plan_type = row
            .get::<String, _>("plan_type")
            .parse::<ActionPlanType>()
            .map_err(repository_error)?;
        let status = row
            .get::<String, _>("status")
            .parse::<ActionPlanStatus>()
            .map_err(repository_error)?;
        let target_refs = sqlx::query(
            "SELECT resource_type, resource_id FROM action_plan_targets \
             WHERE plan_id=$1 ORDER BY ordinal",
        )
        .bind(plan_id)
        .fetch_all(&self.pool)
        .await
        .map_err(repository_error)?
        .into_iter()
        .map(|row| mes_application::DispositionTargetRef {
            resource_type: row.get("resource_type"),
            resource_id: row.get("resource_id"),
        })
        .collect();
        let evidence_refs = sqlx::query(
            "SELECT evidence_type, agent_tool_call_id FROM action_plan_evidence \
             WHERE plan_id=$1 ORDER BY ordinal",
        )
        .bind(plan_id)
        .fetch_all(&self.pool)
        .await
        .map_err(repository_error)?
        .into_iter()
        .map(|row| mes_application::DispositionEvidenceRef {
            tool_call_id: row.get("agent_tool_call_id"),
            tool_name: row.get("evidence_type"),
        })
        .collect();
        let step_values = sqlx::query_scalar::<_, String>(
            "SELECT step_code FROM action_plan_steps WHERE plan_id=$1 ORDER BY ordinal",
        )
        .bind(plan_id)
        .fetch_all(&self.pool)
        .await
        .map_err(repository_error)?;
        let steps = step_values
            .into_iter()
            .map(|value| value.parse::<ActionPlanStep>().map_err(repository_error))
            .collect::<Result<Vec<_>, _>>()?;
        for step in &steps {
            plan_type.validate_step(*step).map_err(repository_error)?;
        }
        let source_evidence_state = row
            .get::<Option<String>, _>("source_evidence_state")
            .map(|value| value.parse::<EvidenceState>().map_err(repository_error))
            .transpose()?;
        Ok(Some(ActionPlanRead {
            id: row.get("id"),
            version: row.get("version"),
            tenant_id: row.get("tenant_id"),
            factory_id: row.get("factory_id"),
            created_by_user_id: row.get("created_by_user_id"),
            source_proposal_id: row.get("source_proposal_id"),
            source_proposal_version: row.get("source_proposal_version"),
            source_review_id: row.get("source_review_id"),
            plan_type,
            status,
            target_refs,
            evidence_refs,
            steps,
            source_evidence_state,
            compiler_policy_version: row.get("compiler_policy_version"),
            compiled_at: row.get("compiled_at"),
            source_snapshot_hash: row.get("source_snapshot_hash"),
            latest_simulation_id: row.get("latest_simulation_id"),
            summary: row.get("summary"),
            expires_at: row.get("expires_at"),
            created_at: row.get("created_at"),
            validated_at: row.get("validated_at"),
            submitted_at: row.get("submitted_at"),
            finished_at: row.get("finished_at"),
        }))
    }

    #[allow(clippy::too_many_lines)]
    async fn validate_action_plan(
        &self,
        command: &ActionPlanValidationCommand,
    ) -> Result<ActionPlanValidationResult, ApplicationError> {
        let operation = "action_plan.validate";
        let request_hash = fingerprint_hash(&command.request_fingerprint);
        let lock_key = format!(
            "{}:{}:{operation}:{}",
            command.tenant_id, command.actor_user_id, command.idempotency_key
        );
        let mut transaction = self.pool.begin().await.map_err(repository_error)?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(lock_key)
            .execute(&mut *transaction)
            .await
            .map_err(repository_error)?;
        if let Some(existing) = sqlx::query(
            "SELECT request_hash, resource_id, response_status FROM idempotency_records \
             WHERE tenant_id=$1 AND actor_user_id=$2 AND operation=$3 AND idempotency_key=$4",
        )
        .bind(command.tenant_id)
        .bind(command.actor_user_id)
        .bind(operation)
        .bind(&command.idempotency_key)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(repository_error)?
        {
            if existing.get::<String, _>("request_hash") != request_hash {
                return Err(ApplicationError::Conflict(
                    "idempotency key was already used with a different request".to_owned(),
                ));
            }
            let snapshot_id: Uuid = existing.get("resource_id");
            let response_status: i32 = existing.get("response_status");
            transaction.commit().await.map_err(repository_error)?;
            if response_status == 409 {
                return Err(ApplicationError::Drift(
                    "trusted manufacturing state differs from the validated snapshot".to_owned(),
                ));
            }
            let plan = self
                .get_action_plan(command.tenant_id, command.factory_id, command.plan_id)
                .await?
                .ok_or_else(|| {
                    ApplicationError::Repository("validated plan disappeared".to_owned())
                })?;
            let snapshot = read_action_plan_snapshot(&self.pool, snapshot_id).await?;
            return Ok(ActionPlanValidationResult {
                plan,
                snapshot,
                replayed: true,
            });
        }

        let plan_row = sqlx::query(
            "SELECT id, version, status, created_by_user_id, expires_at, \
                    source_snapshot_hash, compiler_policy_version, compiled_at \
             FROM action_plans WHERE id=$1 AND tenant_id=$2 AND factory_id=$3 FOR UPDATE",
        )
        .bind(command.plan_id)
        .bind(command.tenant_id)
        .bind(command.factory_id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(repository_error)?
        .ok_or_else(|| {
            ApplicationError::Precondition("action plan was not found in scope".to_owned())
        })?;
        if plan_row.get::<Uuid, _>("created_by_user_id") != command.actor_user_id {
            return Err(ApplicationError::Precondition(
                "only the plan creator can validate this plan".to_owned(),
            ));
        }
        let prior_version: i64 = plan_row.get("version");
        if prior_version != command.expected_version {
            return Err(ApplicationError::Conflict(
                "action-plan version does not match If-Match".to_owned(),
            ));
        }
        let prior_status = plan_row
            .get::<String, _>("status")
            .parse::<ActionPlanStatus>()
            .map_err(repository_error)?;
        if !matches!(
            prior_status,
            ActionPlanStatus::Draft | ActionPlanStatus::Validated
        ) {
            return Err(ApplicationError::Precondition(
                "only draft or validated plans can be validated".to_owned(),
            ));
        }
        if plan_row.get::<DateTime<Utc>, _>("expires_at") <= Utc::now() {
            return Err(ApplicationError::Precondition(
                "expired action plan cannot be validated".to_owned(),
            ));
        }
        if plan_row
            .get::<Option<String>, _>("compiler_policy_version")
            .is_none()
            || plan_row
                .get::<Option<DateTime<Utc>>, _>("compiled_at")
                .is_none()
        {
            return Err(ApplicationError::Precondition(
                "uncompiled action plan cannot be validated".to_owned(),
            ));
        }

        let built = build_action_plan_snapshot(&mut transaction, command).await?;
        let snapshot_id = Uuid::new_v4();
        let baseline_hash = plan_row.get::<Option<String>, _>("source_snapshot_hash");
        let drifted = prior_status == ActionPlanStatus::Validated
            && baseline_hash.as_deref() != Some(built.snapshot_hash.as_str());
        let resulting_status = if drifted {
            ActionPlanStatus::InvalidatedByDrift
        } else {
            ActionPlanStatus::Validated
        };
        let resulting_version = if prior_status == ActionPlanStatus::Draft || drifted {
            prior_version + 1
        } else {
            prior_version
        };
        sqlx::query(
            "INSERT INTO action_plan_snapshots (\
                id, plan_id, plan_version, validation_policy_version, snapshot_hash, \
                canonical_payload, missing_fields, target_count, evidence_count\
             ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)",
        )
        .bind(snapshot_id)
        .bind(command.plan_id)
        .bind(resulting_version)
        .bind(ACTION_PLAN_VALIDATION_POLICY_VERSION)
        .bind(&built.snapshot_hash)
        .bind(&built.canonical_payload)
        .bind(&built.missing_fields)
        .bind(built.target_count)
        .bind(built.evidence_count)
        .execute(&mut *transaction)
        .await
        .map_err(repository_error)?;
        if prior_status == ActionPlanStatus::Draft {
            sqlx::query(
                "UPDATE action_plans SET status='validated', version=version+1, \
                 source_snapshot_hash=$2, validated_at=now() WHERE id=$1",
            )
            .bind(command.plan_id)
            .bind(&built.snapshot_hash)
            .execute(&mut *transaction)
            .await
            .map_err(repository_error)?;
        } else if drifted {
            sqlx::query(
                "UPDATE action_plans SET status='invalidated_by_drift', version=version+1, \
                 finished_at=now() WHERE id=$1",
            )
            .bind(command.plan_id)
            .execute(&mut *transaction)
            .await
            .map_err(repository_error)?;
        }
        let response_status = if drifted { 409 } else { 200 };
        sqlx::query(
            "INSERT INTO idempotency_records (\
                id, tenant_id, actor_user_id, operation, idempotency_key, request_hash, \
                resource_type, resource_id, response_status\
             ) VALUES ($1,$2,$3,$4,$5,$6,'action_plan_snapshot',$7,$8)",
        )
        .bind(Uuid::new_v4())
        .bind(command.tenant_id)
        .bind(command.actor_user_id)
        .bind(operation)
        .bind(&command.idempotency_key)
        .bind(request_hash)
        .bind(snapshot_id)
        .bind(response_status)
        .execute(&mut *transaction)
        .await
        .map_err(repository_error)?;
        sqlx::query(
            "INSERT INTO action_plan_validation_audits (\
                id, request_id, tenant_id, factory_id, plan_id, actor_user_id, action, \
                prior_status, resulting_status, prior_version, resulting_version, \
                snapshot_id, snapshot_hash, validation_policy_version, idempotency_outcome, \
                target_count, evidence_count, missing_field_count\
             ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,'created',$15,$16,$17)",
        )
        .bind(Uuid::new_v4())
        .bind(command.request_id)
        .bind(command.tenant_id)
        .bind(command.factory_id)
        .bind(command.plan_id)
        .bind(command.actor_user_id)
        .bind(if drifted {
            "drift_invalidate"
        } else {
            "validate"
        })
        .bind(prior_status.as_str())
        .bind(resulting_status.as_str())
        .bind(prior_version)
        .bind(resulting_version)
        .bind(snapshot_id)
        .bind(&built.snapshot_hash)
        .bind(ACTION_PLAN_VALIDATION_POLICY_VERSION)
        .bind(built.target_count)
        .bind(built.evidence_count)
        .bind(i32::try_from(built.missing_fields.len()).map_err(repository_error)?)
        .execute(&mut *transaction)
        .await
        .map_err(repository_error)?;
        transaction.commit().await.map_err(repository_error)?;
        if drifted {
            return Err(ApplicationError::Drift(
                "trusted manufacturing state differs from the validated snapshot".to_owned(),
            ));
        }
        let plan = self
            .get_action_plan(command.tenant_id, command.factory_id, command.plan_id)
            .await?
            .ok_or_else(|| ApplicationError::Repository("validated plan disappeared".to_owned()))?;
        let snapshot = read_action_plan_snapshot(&self.pool, snapshot_id).await?;
        Ok(ActionPlanValidationResult {
            plan,
            snapshot,
            replayed: false,
        })
    }

    #[allow(clippy::too_many_lines)]
    async fn simulate_action_plan(
        &self,
        command: &ActionPlanSimulationCommand,
    ) -> Result<ActionPlanSimulationResult, ApplicationError> {
        let operation = "action_plan.simulate";
        let request_hash = fingerprint_hash(&command.request_fingerprint);
        let lock_key = format!(
            "{}:{}:{operation}:{}",
            command.tenant_id, command.actor_user_id, command.idempotency_key
        );
        let mut transaction = self.pool.begin().await.map_err(repository_error)?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(lock_key)
            .execute(&mut *transaction)
            .await
            .map_err(repository_error)?;
        if let Some(existing) = sqlx::query(
            "SELECT request_hash, resource_id, response_status FROM idempotency_records \
             WHERE tenant_id=$1 AND actor_user_id=$2 AND operation=$3 AND idempotency_key=$4",
        )
        .bind(command.tenant_id)
        .bind(command.actor_user_id)
        .bind(operation)
        .bind(&command.idempotency_key)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(repository_error)?
        {
            if existing.get::<String, _>("request_hash") != request_hash {
                return Err(ApplicationError::Conflict(
                    "idempotency key was already used with a different request".to_owned(),
                ));
            }
            let resource_id: Uuid = existing.get("resource_id");
            let response_status: i32 = existing.get("response_status");
            transaction.commit().await.map_err(repository_error)?;
            if response_status == 409 {
                return Err(ApplicationError::Drift(
                    "trusted manufacturing state differs from the validated snapshot".to_owned(),
                ));
            }
            let plan = self
                .get_action_plan(command.tenant_id, command.factory_id, command.plan_id)
                .await?
                .ok_or_else(|| {
                    ApplicationError::Repository("simulated plan disappeared".to_owned())
                })?;
            let simulation = read_action_plan_simulation(&self.pool, resource_id).await?;
            return Ok(ActionPlanSimulationResult {
                plan,
                simulation,
                replayed: true,
            });
        }

        let plan_row = sqlx::query(
            "SELECT version, status, created_by_user_id, expires_at, source_snapshot_hash, \
                    plan_type FROM action_plans \
             WHERE id=$1 AND tenant_id=$2 AND factory_id=$3 FOR UPDATE",
        )
        .bind(command.plan_id)
        .bind(command.tenant_id)
        .bind(command.factory_id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(repository_error)?
        .ok_or_else(|| {
            ApplicationError::Precondition("action plan was not found in scope".to_owned())
        })?;
        if plan_row.get::<Uuid, _>("created_by_user_id") != command.actor_user_id {
            return Err(ApplicationError::Precondition(
                "only the plan creator can simulate this plan".to_owned(),
            ));
        }
        let prior_version: i64 = plan_row.get("version");
        if prior_version != command.expected_version {
            return Err(ApplicationError::Conflict(
                "action-plan version does not match If-Match".to_owned(),
            ));
        }
        let prior_status = plan_row
            .get::<String, _>("status")
            .parse::<ActionPlanStatus>()
            .map_err(repository_error)?;
        if prior_status != ActionPlanStatus::Validated {
            return Err(ApplicationError::Precondition(
                "only a validated action plan can be simulated".to_owned(),
            ));
        }
        if plan_row.get::<DateTime<Utc>, _>("expires_at") <= Utc::now() {
            return Err(ApplicationError::Precondition(
                "expired action plan cannot be simulated".to_owned(),
            ));
        }
        let baseline_hash = plan_row
            .get::<Option<String>, _>("source_snapshot_hash")
            .ok_or_else(|| {
                ApplicationError::Precondition("validated snapshot is missing".to_owned())
            })?;
        let snapshot_command = ActionPlanValidationCommand {
            request_id: command.request_id,
            tenant_id: command.tenant_id,
            factory_id: command.factory_id,
            actor_user_id: command.actor_user_id,
            plan_id: command.plan_id,
            expected_version: command.expected_version,
            idempotency_key: command.idempotency_key.clone(),
            request_fingerprint: command.request_fingerprint.clone(),
        };
        let built = build_action_plan_snapshot(&mut transaction, &snapshot_command).await?;
        if built.snapshot_hash != baseline_hash {
            let observed_snapshot_id = Uuid::new_v4();
            sqlx::query(
                "INSERT INTO action_plan_snapshots (\
                    id, plan_id, plan_version, validation_policy_version, snapshot_hash, \
                    canonical_payload, missing_fields, target_count, evidence_count\
                 ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)",
            )
            .bind(observed_snapshot_id)
            .bind(command.plan_id)
            .bind(prior_version + 1)
            .bind(ACTION_PLAN_VALIDATION_POLICY_VERSION)
            .bind(&built.snapshot_hash)
            .bind(&built.canonical_payload)
            .bind(&built.missing_fields)
            .bind(built.target_count)
            .bind(built.evidence_count)
            .execute(&mut *transaction)
            .await
            .map_err(repository_error)?;
            sqlx::query(
                "UPDATE action_plans SET status='invalidated_by_drift', version=version+1, \
                 finished_at=now() WHERE id=$1",
            )
            .bind(command.plan_id)
            .execute(&mut *transaction)
            .await
            .map_err(repository_error)?;
            sqlx::query(
                "INSERT INTO idempotency_records (\
                    id, tenant_id, actor_user_id, operation, idempotency_key, request_hash, \
                    resource_type, resource_id, response_status\
                 ) VALUES ($1,$2,$3,$4,$5,$6,'action_plan_snapshot',$7,409)",
            )
            .bind(Uuid::new_v4())
            .bind(command.tenant_id)
            .bind(command.actor_user_id)
            .bind(operation)
            .bind(&command.idempotency_key)
            .bind(request_hash)
            .bind(observed_snapshot_id)
            .execute(&mut *transaction)
            .await
            .map_err(repository_error)?;
            sqlx::query(
                "INSERT INTO action_plan_simulation_audits (\
                    id, request_id, tenant_id, factory_id, plan_id, actor_user_id, action, \
                    prior_status, resulting_status, prior_version, resulting_version, \
                    source_snapshot_hash, observed_snapshot_hash, simulation_policy_version, \
                    idempotency_outcome, protected_state_unchanged\
                 ) VALUES ($1,$2,$3,$4,$5,$6,'drift_invalidate','validated',\
                    'invalidated_by_drift',$7,$8,$9,$10,$11,'created',TRUE)",
            )
            .bind(Uuid::new_v4())
            .bind(command.request_id)
            .bind(command.tenant_id)
            .bind(command.factory_id)
            .bind(command.plan_id)
            .bind(command.actor_user_id)
            .bind(prior_version)
            .bind(prior_version + 1)
            .bind(&baseline_hash)
            .bind(&built.snapshot_hash)
            .bind(ACTION_PLAN_SIMULATION_POLICY_VERSION)
            .execute(&mut *transaction)
            .await
            .map_err(repository_error)?;
            transaction.commit().await.map_err(repository_error)?;
            return Err(ApplicationError::Drift(
                "trusted manufacturing state differs from the validated snapshot".to_owned(),
            ));
        }

        let started_at = Utc::now();
        let target_rows = sqlx::query(
            "SELECT resource_type, resource_id FROM action_plan_targets \
             WHERE plan_id=$1 ORDER BY ordinal",
        )
        .bind(command.plan_id)
        .fetch_all(&mut *transaction)
        .await
        .map_err(repository_error)?;
        let affected_resources = target_rows
            .iter()
            .map(|row| mes_application::DispositionTargetRef {
                resource_type: row.get("resource_type"),
                resource_id: row.get("resource_id"),
            })
            .collect::<Vec<_>>();
        let step_rows = sqlx::query_scalar::<_, String>(
            "SELECT step_code FROM action_plan_steps WHERE plan_id=$1 ORDER BY ordinal",
        )
        .bind(command.plan_id)
        .fetch_all(&mut *transaction)
        .await
        .map_err(repository_error)?;
        let impact_preview = step_rows
            .iter()
            .map(|step| format!("preview_only:{step}"))
            .collect::<Vec<_>>();
        let plan_type = plan_row
            .get::<String, _>("plan_type")
            .parse::<ActionPlanType>()
            .map_err(repository_error)?;
        let required_human_roles = match plan_type {
            ActionPlanType::MaintenanceInspectionPlan => vec!["maintenance_reviewer".to_owned()],
            ActionPlanType::QualityInvestigationPlan
            | ActionPlanType::InventoryReconciliationPlan => {
                vec!["quality_reviewer".to_owned()]
            }
        };
        let precondition_findings = vec![
            "validated_snapshot_matches_current_trusted_state".to_owned(),
            "simulation_has_no_operational_write_authority".to_owned(),
        ];
        let missing_data_findings = built.missing_fields.clone();
        let canonical_input = json!({
            "simulation_policy_version": ACTION_PLAN_SIMULATION_POLICY_VERSION,
            "deterministic_seed": ACTION_PLAN_SIMULATION_SEED,
            "plan_id": command.plan_id,
            "plan_version": prior_version,
            "source_snapshot_hash": baseline_hash,
            "affected_resources": affected_resources,
            "steps": step_rows,
            "missing_data_findings": missing_data_findings,
        });
        let input_hash = hex::encode(Sha256::digest(
            serde_json::to_vec(&canonical_input).map_err(repository_error)?,
        ));
        let canonical_output = json!({
            "input_hash": input_hash,
            "affected_resources": affected_resources,
            "precondition_findings": precondition_findings,
            "missing_data_findings": missing_data_findings,
            "required_human_roles": required_human_roles,
            "impact_preview": impact_preview,
            "completion_status": "completed",
            "disclaimer": "impact preview only; no physical action occurred",
        });
        let output_hash = hex::encode(Sha256::digest(
            serde_json::to_vec(&canonical_output).map_err(repository_error)?,
        ));
        let protected_check =
            build_action_plan_snapshot(&mut transaction, &snapshot_command).await?;
        if protected_check.snapshot_hash != baseline_hash {
            return Err(ApplicationError::Repository(
                "protected trusted state changed during isolated simulation".to_owned(),
            ));
        }
        let completed_at = Utc::now();
        let duration_ms = (completed_at - started_at).num_milliseconds().max(0);
        let simulation_id = Uuid::new_v4();
        let affected_json = serde_json::to_value(&affected_resources).map_err(repository_error)?;
        sqlx::query(
            "INSERT INTO action_plan_simulations (\
                id, plan_id, plan_version, source_snapshot_hash, simulation_policy_version, \
                deterministic_seed, canonical_input_hash, canonical_output_hash, \
                affected_resources, precondition_findings, missing_data_findings, \
                required_human_roles, impact_preview, completion_status, \
                protected_state_unchanged, started_at, completed_at, duration_ms\
             ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,'completed',TRUE,$14,$15,$16)",
        )
        .bind(simulation_id)
        .bind(command.plan_id)
        .bind(prior_version + 1)
        .bind(&baseline_hash)
        .bind(ACTION_PLAN_SIMULATION_POLICY_VERSION)
        .bind(ACTION_PLAN_SIMULATION_SEED)
        .bind(&input_hash)
        .bind(&output_hash)
        .bind(affected_json)
        .bind(&precondition_findings)
        .bind(&missing_data_findings)
        .bind(&required_human_roles)
        .bind(&impact_preview)
        .bind(started_at)
        .bind(completed_at)
        .bind(duration_ms)
        .execute(&mut *transaction)
        .await
        .map_err(repository_error)?;
        sqlx::query(
            "UPDATE action_plans SET status='simulated', version=version+1, \
             latest_simulation_id=$2 WHERE id=$1",
        )
        .bind(command.plan_id)
        .bind(simulation_id)
        .execute(&mut *transaction)
        .await
        .map_err(repository_error)?;
        sqlx::query(
            "INSERT INTO idempotency_records (\
                id, tenant_id, actor_user_id, operation, idempotency_key, request_hash, \
                resource_type, resource_id, response_status\
             ) VALUES ($1,$2,$3,$4,$5,$6,'action_plan_simulation',$7,200)",
        )
        .bind(Uuid::new_v4())
        .bind(command.tenant_id)
        .bind(command.actor_user_id)
        .bind(operation)
        .bind(&command.idempotency_key)
        .bind(request_hash)
        .bind(simulation_id)
        .execute(&mut *transaction)
        .await
        .map_err(repository_error)?;
        sqlx::query(
            "INSERT INTO action_plan_simulation_audits (\
                id, request_id, tenant_id, factory_id, plan_id, actor_user_id, action, \
                prior_status, resulting_status, prior_version, resulting_version, \
                simulation_id, source_snapshot_hash, observed_snapshot_hash, input_hash, \
                output_hash, simulation_policy_version, idempotency_outcome, \
                protected_state_unchanged\
             ) VALUES ($1,$2,$3,$4,$5,$6,'simulate','validated','simulated',$7,$8,$9,$10,$11,\
                $12,$13,$14,'created',TRUE)",
        )
        .bind(Uuid::new_v4())
        .bind(command.request_id)
        .bind(command.tenant_id)
        .bind(command.factory_id)
        .bind(command.plan_id)
        .bind(command.actor_user_id)
        .bind(prior_version)
        .bind(prior_version + 1)
        .bind(simulation_id)
        .bind(&baseline_hash)
        .bind(&baseline_hash)
        .bind(&input_hash)
        .bind(&output_hash)
        .bind(ACTION_PLAN_SIMULATION_POLICY_VERSION)
        .execute(&mut *transaction)
        .await
        .map_err(repository_error)?;
        transaction.commit().await.map_err(repository_error)?;
        let plan = self
            .get_action_plan(command.tenant_id, command.factory_id, command.plan_id)
            .await?
            .ok_or_else(|| ApplicationError::Repository("simulated plan disappeared".to_owned()))?;
        let simulation = read_action_plan_simulation(&self.pool, simulation_id).await?;
        Ok(ActionPlanSimulationResult {
            plan,
            simulation,
            replayed: false,
        })
    }

    async fn expire_action_plan(
        &self,
        tenant_id: Uuid,
        factory_id: Uuid,
        plan_id: Uuid,
    ) -> Result<Option<ActionPlanRead>, ApplicationError> {
        let mut transaction = self.pool.begin().await.map_err(repository_error)?;
        let expired = sqlx::query(
            "SELECT status, version FROM action_plans WHERE id=$1 AND tenant_id=$2 \
             AND factory_id=$3 AND expires_at<=now() \
             AND status IN ('draft','validated','simulated','pending_approval') FOR UPDATE",
        )
        .bind(plan_id)
        .bind(tenant_id)
        .bind(factory_id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(repository_error)?;
        if let Some(row) = expired {
            let prior_version: i64 = row.get("version");
            let prior_status: String = row.get("status");
            sqlx::query(
                "UPDATE action_plans SET status='expired', version=version+1, finished_at=now() \
                 WHERE id=$1",
            )
            .bind(plan_id)
            .execute(&mut *transaction)
            .await
            .map_err(repository_error)?;
            sqlx::query(
                "INSERT INTO action_plan_transition_audits (\
                    id, request_id, tenant_id, factory_id, plan_id, action, prior_status, \
                    resulting_status, prior_version, resulting_version, idempotency_outcome\
                 ) VALUES ($1,$2,$3,$4,$5,'expire',$6,'expired',$7,$8,'not_applicable')",
            )
            .bind(Uuid::new_v4())
            .bind(Uuid::new_v4())
            .bind(tenant_id)
            .bind(factory_id)
            .bind(plan_id)
            .bind(prior_status)
            .bind(prior_version)
            .bind(prior_version + 1)
            .execute(&mut *transaction)
            .await
            .map_err(repository_error)?;
        }
        transaction.commit().await.map_err(repository_error)?;
        self.get_action_plan(tenant_id, factory_id, plan_id).await
    }

    async fn submit_action_plan(
        &self,
        command: &ActionPlanTransitionCommand,
    ) -> Result<ActionPlanTransitionResult, ApplicationError> {
        let operation = "action_plan.submit";
        let mut transaction = self.pool.begin().await.map_err(repository_error)?;
        if action_plan_transition_replay(&mut transaction, command, operation).await? {
            transaction.commit().await.map_err(repository_error)?;
            let plan = self
                .get_action_plan(command.tenant_id, command.factory_id, command.plan_id)
                .await?
                .ok_or_else(|| {
                    ApplicationError::Repository("submitted plan disappeared".to_owned())
                })?;
            return Ok(ActionPlanTransitionResult {
                plan,
                review: None,
                replayed: true,
            });
        }
        let row = lock_action_plan(&mut transaction, command).await?;
        verify_action_plan_actor_version(&row, command, true)?;
        let prior_status = parse_action_plan_row_status(&row)?;
        if prior_status != ActionPlanStatus::Simulated {
            return Err(ApplicationError::Precondition(
                "only a simulated action plan can be submitted".to_owned(),
            ));
        }
        ensure_action_plan_not_expired(&row)?;
        let baseline_hash = required_snapshot_hash(&row)?;
        let simulation_id = row
            .get::<Option<Uuid>, _>("latest_simulation_id")
            .ok_or_else(|| {
                ApplicationError::Precondition("completed simulation is missing".to_owned())
            })?;
        verify_current_simulation(
            &mut transaction,
            command.plan_id,
            simulation_id,
            &baseline_hash,
        )
        .await?;
        let observed = rebuild_transition_snapshot(&mut transaction, command).await?;
        if observed.snapshot_hash != baseline_hash {
            persist_transition_drift(
                &mut transaction,
                command,
                operation,
                prior_status,
                &baseline_hash,
                &observed,
                Some(simulation_id),
            )
            .await?;
            transaction.commit().await.map_err(repository_error)?;
            return Err(ApplicationError::Drift(
                "trusted manufacturing state differs from the validated snapshot".to_owned(),
            ));
        }
        let prior_version: i64 = row.get("version");
        sqlx::query(
            "UPDATE action_plans SET status='pending_approval', version=version+1, \
             submitted_at=now() WHERE id=$1",
        )
        .bind(command.plan_id)
        .execute(&mut *transaction)
        .await
        .map_err(repository_error)?;
        persist_action_plan_transition(
            &mut transaction,
            command,
            operation,
            "submit",
            prior_status,
            ActionPlanStatus::PendingApproval,
            prior_version,
            Some(&baseline_hash),
            Some(simulation_id),
            None,
            None,
        )
        .await?;
        transaction.commit().await.map_err(repository_error)?;
        let plan = self
            .get_action_plan(command.tenant_id, command.factory_id, command.plan_id)
            .await?
            .ok_or_else(|| ApplicationError::Repository("submitted plan disappeared".to_owned()))?;
        Ok(ActionPlanTransitionResult {
            plan,
            review: None,
            replayed: false,
        })
    }

    async fn cancel_action_plan(
        &self,
        command: &ActionPlanTransitionCommand,
    ) -> Result<ActionPlanTransitionResult, ApplicationError> {
        let operation = "action_plan.cancel";
        let mut transaction = self.pool.begin().await.map_err(repository_error)?;
        if action_plan_transition_replay(&mut transaction, command, operation).await? {
            transaction.commit().await.map_err(repository_error)?;
            let plan = self
                .get_action_plan(command.tenant_id, command.factory_id, command.plan_id)
                .await?
                .ok_or_else(|| {
                    ApplicationError::Repository("cancelled plan disappeared".to_owned())
                })?;
            return Ok(ActionPlanTransitionResult {
                plan,
                review: None,
                replayed: true,
            });
        }
        let row = lock_action_plan(&mut transaction, command).await?;
        verify_action_plan_actor_version(&row, command, true)?;
        let prior_status = parse_action_plan_row_status(&row)?;
        if !matches!(
            prior_status,
            ActionPlanStatus::Draft | ActionPlanStatus::Validated | ActionPlanStatus::Simulated
        ) {
            return Err(ApplicationError::Precondition(
                "action plan cannot be cancelled from its current state".to_owned(),
            ));
        }
        ensure_action_plan_not_expired(&row)?;
        let prior_version: i64 = row.get("version");
        sqlx::query(
            "UPDATE action_plans SET status='cancelled', version=version+1, finished_at=now() \
             WHERE id=$1",
        )
        .bind(command.plan_id)
        .execute(&mut *transaction)
        .await
        .map_err(repository_error)?;
        persist_action_plan_transition(
            &mut transaction,
            command,
            operation,
            "cancel",
            prior_status,
            ActionPlanStatus::Cancelled,
            prior_version,
            row.get::<Option<String>, _>("source_snapshot_hash")
                .as_deref(),
            row.get("latest_simulation_id"),
            None,
            None,
        )
        .await?;
        transaction.commit().await.map_err(repository_error)?;
        let plan = self
            .get_action_plan(command.tenant_id, command.factory_id, command.plan_id)
            .await?
            .ok_or_else(|| ApplicationError::Repository("cancelled plan disappeared".to_owned()))?;
        Ok(ActionPlanTransitionResult {
            plan,
            review: None,
            replayed: false,
        })
    }

    #[allow(clippy::too_many_lines)]
    async fn review_action_plan(
        &self,
        command: &ActionPlanReviewCommand,
    ) -> Result<ActionPlanTransitionResult, ApplicationError> {
        let operation = "action_plan.review";
        let transition = &command.transition;
        let mut transaction = self.pool.begin().await.map_err(repository_error)?;
        if action_plan_transition_replay(&mut transaction, transition, operation).await? {
            let review_id = sqlx::query_scalar::<_, Uuid>(
                "SELECT resource_id FROM idempotency_records WHERE tenant_id=$1 \
                 AND actor_user_id=$2 AND operation=$3 AND idempotency_key=$4",
            )
            .bind(transition.tenant_id)
            .bind(transition.actor_user_id)
            .bind(operation)
            .bind(&transition.idempotency_key)
            .fetch_one(&mut *transaction)
            .await
            .map_err(repository_error)?;
            transaction.commit().await.map_err(repository_error)?;
            let plan = self
                .get_action_plan(
                    transition.tenant_id,
                    transition.factory_id,
                    transition.plan_id,
                )
                .await?
                .ok_or_else(|| {
                    ApplicationError::Repository("reviewed plan disappeared".to_owned())
                })?;
            return Ok(ActionPlanTransitionResult {
                plan,
                review: Some(read_action_plan_review(&self.pool, review_id).await?),
                replayed: true,
            });
        }
        let row = lock_action_plan(&mut transaction, transition).await?;
        verify_action_plan_actor_version(&row, transition, false)?;
        if row.get::<Uuid, _>("created_by_user_id") == transition.actor_user_id {
            return Err(ApplicationError::Precondition(
                "plan creator cannot review their own plan".to_owned(),
            ));
        }
        let prior_status = parse_action_plan_row_status(&row)?;
        if prior_status != ActionPlanStatus::PendingApproval {
            return Err(ApplicationError::Precondition(
                "only a pending action plan can be reviewed".to_owned(),
            ));
        }
        ensure_action_plan_not_expired(&row)?;
        let baseline_hash = required_snapshot_hash(&row)?;
        let simulation_id = row
            .get::<Option<Uuid>, _>("latest_simulation_id")
            .ok_or_else(|| {
                ApplicationError::Precondition("completed simulation is missing".to_owned())
            })?;
        verify_current_simulation(
            &mut transaction,
            transition.plan_id,
            simulation_id,
            &baseline_hash,
        )
        .await?;
        let observed = rebuild_transition_snapshot(&mut transaction, transition).await?;
        if observed.snapshot_hash != baseline_hash {
            persist_transition_drift(
                &mut transaction,
                transition,
                operation,
                prior_status,
                &baseline_hash,
                &observed,
                Some(simulation_id),
            )
            .await?;
            transaction.commit().await.map_err(repository_error)?;
            return Err(ApplicationError::Drift(
                "trusted manufacturing state differs from the validated snapshot".to_owned(),
            ));
        }
        let prior_version: i64 = row.get("version");
        let review_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO action_plan_reviews (\
                id, plan_id, plan_version, tenant_id, factory_id, reviewer_user_id, decision, \
                rationale, source_snapshot_hash, simulation_id\
             ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)",
        )
        .bind(review_id)
        .bind(transition.plan_id)
        .bind(prior_version)
        .bind(transition.tenant_id)
        .bind(transition.factory_id)
        .bind(transition.actor_user_id)
        .bind(command.decision.as_str())
        .bind(&command.rationale)
        .bind(&baseline_hash)
        .bind(simulation_id)
        .execute(&mut *transaction)
        .await
        .map_err(repository_error)?;
        let resulting_status = match command.decision {
            ReviewDecision::Approved => ActionPlanStatus::PlanApproved,
            ReviewDecision::Rejected => ActionPlanStatus::PlanRejected,
        };
        sqlx::query(
            "UPDATE action_plans SET status=$2, version=version+1, finished_at=now() WHERE id=$1",
        )
        .bind(transition.plan_id)
        .bind(resulting_status.as_str())
        .execute(&mut *transaction)
        .await
        .map_err(repository_error)?;
        persist_action_plan_transition(
            &mut transaction,
            transition,
            operation,
            "review",
            prior_status,
            resulting_status,
            prior_version,
            Some(&baseline_hash),
            Some(simulation_id),
            Some(review_id),
            Some(command.decision),
        )
        .await?;
        transaction.commit().await.map_err(repository_error)?;
        let plan = self
            .get_action_plan(
                transition.tenant_id,
                transition.factory_id,
                transition.plan_id,
            )
            .await?
            .ok_or_else(|| ApplicationError::Repository("reviewed plan disappeared".to_owned()))?;
        Ok(ActionPlanTransitionResult {
            plan,
            review: Some(read_action_plan_review(&self.pool, review_id).await?),
            replayed: false,
        })
    }
}

#[async_trait]
impl ExpertSandboxRepository for PostgresMesRepository {
    async fn list_sandbox_scope(
        &self,
        tenant_id: Uuid,
        factory_ids: &[Uuid],
    ) -> Result<Vec<SandboxFactoryRead>, ApplicationError> {
        let rows = sqlx::query(
            "SELECT f.id AS factory_id, f.name AS factory_name, f.slug, f.timezone, \
                    l.id AS line_id, l.name AS line_name \
             FROM factories f JOIN production_lines l ON l.factory_id=f.id AND l.tenant_id=f.tenant_id \
             WHERE f.tenant_id=$1 AND f.id=ANY($2) ORDER BY f.name,l.name",
        )
        .bind(tenant_id)
        .bind(factory_ids)
        .fetch_all(&self.pool)
        .await
        .map_err(repository_error)?;
        let mut factories = BTreeMap::<Uuid, SandboxFactoryRead>::new();
        for row in rows {
            let factory_id: Uuid = row.get("factory_id");
            factories
                .entry(factory_id)
                .or_insert_with(|| SandboxFactoryRead {
                    id: factory_id,
                    name: row.get("factory_name"),
                    slug: row.get("slug"),
                    timezone: row.get("timezone"),
                    lines: Vec::new(),
                })
                .lines
                .push(SandboxLineRead {
                    id: row.get("line_id"),
                    factory_id,
                    name: row.get("line_name"),
                });
        }
        Ok(factories.into_values().collect())
    }

    async fn list_work_queue(
        &self,
        tenant_id: Uuid,
        factory_id: Uuid,
        actor_user_id: Uuid,
        tenant_visibility: bool,
        filter: &WorkQueueFilter,
    ) -> Result<Vec<WorkQueueItemRead>, ApplicationError> {
        let rows = sqlx::query(
            r"
            WITH queue AS (
                SELECT id, 'agent_run'::TEXT AS record_type, status, NULL::BIGINT AS version,
                       user_id AS actor_user_id, NULL::UUID AS source_id,
                       final_response AS summary, started_at AS created_at
                FROM agent_runs
                WHERE tenant_id=$1 AND factory_id=$2 AND ($4 OR user_id=$3::TEXT)
                UNION ALL
                SELECT id, 'disposition_proposal', status, version,
                       initiated_by_user_id::TEXT, source_run_id, summary, created_at
                FROM disposition_proposals
                WHERE tenant_id=$1 AND factory_id=$2
                  AND ($4 OR initiated_by_user_id=$3)
                UNION ALL
                SELECT id, 'action_plan', status, version,
                       created_by_user_id::TEXT, source_proposal_id, summary, created_at
                FROM action_plans
                WHERE tenant_id=$1 AND factory_id=$2
                  AND ($4 OR created_by_user_id=$3)
            )
            SELECT * FROM queue
            WHERE ($5::TEXT IS NULL OR status=$5)
              AND ($6::TEXT IS NULL OR record_type=$6)
            ORDER BY created_at DESC, id DESC LIMIT $7 OFFSET $8
            ",
        )
        .bind(tenant_id)
        .bind(factory_id)
        .bind(actor_user_id)
        .bind(tenant_visibility)
        .bind(filter.status.as_deref())
        .bind(filter.record_type.as_deref())
        .bind(filter.limit)
        .bind(filter.offset)
        .fetch_all(&self.pool)
        .await
        .map_err(repository_error)?;
        Ok(rows
            .into_iter()
            .map(|row| WorkQueueItemRead {
                id: row.get("id"),
                record_type: row.get("record_type"),
                status: row.get("status"),
                version: row.get("version"),
                actor_user_id: row.get("actor_user_id"),
                source_id: row.get("source_id"),
                summary: row.get("summary"),
                created_at: row.get("created_at"),
            })
            .collect())
    }

    async fn create_domain_feedback(
        &self,
        draft: &DomainFeedbackDraft,
    ) -> Result<DomainFeedbackRead, ApplicationError> {
        let row = sqlx::query(
            r"
            INSERT INTO domain_feedback (
                id, tenant_id, factory_id, actor_user_id, run_id, proposal_id, plan_id,
                evidence_rating, realism_rating, next_source, missing_factor,
                consent_for_research, trace_metadata
            )
            SELECT $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13
            WHERE EXISTS (
                SELECT 1 FROM agent_runs WHERE id=$5 AND tenant_id=$2 AND factory_id=$3
            )
              AND ($6::UUID IS NULL OR EXISTS (
                SELECT 1 FROM disposition_proposals
                WHERE id=$6 AND tenant_id=$2 AND factory_id=$3 AND source_run_id=$5
              ))
              AND ($7::UUID IS NULL OR EXISTS (
                SELECT 1 FROM action_plans
                WHERE id=$7 AND tenant_id=$2 AND factory_id=$3
                  AND ($6::UUID IS NULL OR source_proposal_id=$6)
              ))
            RETURNING id,run_id,proposal_id,plan_id,evidence_rating,realism_rating,
                      next_source,missing_factor,consent_for_research,trace_metadata,created_at
            ",
        )
        .bind(draft.id)
        .bind(draft.tenant_id)
        .bind(draft.factory_id)
        .bind(draft.actor_user_id)
        .bind(draft.run_id)
        .bind(draft.proposal_id)
        .bind(draft.plan_id)
        .bind(&draft.evidence_rating)
        .bind(&draft.realism_rating)
        .bind(&draft.next_source)
        .bind(&draft.missing_factor)
        .bind(draft.consent_for_research)
        .bind(&draft.trace_metadata)
        .fetch_optional(&self.pool)
        .await
        .map_err(repository_error)?
        .ok_or_else(|| {
            ApplicationError::Precondition("feedback subject binding is invalid".to_owned())
        })?;
        Ok(DomainFeedbackRead {
            id: row.get("id"),
            run_id: row.get("run_id"),
            proposal_id: row.get("proposal_id"),
            plan_id: row.get("plan_id"),
            evidence_rating: row.get("evidence_rating"),
            realism_rating: row.get("realism_rating"),
            next_source: row.get("next_source"),
            missing_factor: row.get("missing_factor"),
            consent_for_research: row.get("consent_for_research"),
            trace_metadata: row.get("trace_metadata"),
            created_at: row.get("created_at"),
        })
    }
}

async fn action_plan_transition_replay(
    transaction: &mut Transaction<'_, Postgres>,
    command: &ActionPlanTransitionCommand,
    operation: &str,
) -> Result<bool, ApplicationError> {
    let lock_key = format!(
        "{}:{}:{operation}:{}",
        command.tenant_id, command.actor_user_id, command.idempotency_key
    );
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(lock_key)
        .execute(&mut **transaction)
        .await
        .map_err(repository_error)?;
    let existing = sqlx::query(
        "SELECT request_hash, response_status FROM idempotency_records \
         WHERE tenant_id=$1 AND actor_user_id=$2 AND operation=$3 AND idempotency_key=$4",
    )
    .bind(command.tenant_id)
    .bind(command.actor_user_id)
    .bind(operation)
    .bind(&command.idempotency_key)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(repository_error)?;
    let Some(existing) = existing else {
        return Ok(false);
    };
    if existing.get::<String, _>("request_hash") != fingerprint_hash(&command.request_fingerprint) {
        return Err(ApplicationError::Conflict(
            "idempotency key was already used with a different request".to_owned(),
        ));
    }
    if existing.get::<i32, _>("response_status") == 409 {
        return Err(ApplicationError::Drift(
            "trusted manufacturing state differs from the validated snapshot".to_owned(),
        ));
    }
    Ok(true)
}

async fn lock_action_plan(
    transaction: &mut Transaction<'_, Postgres>,
    command: &ActionPlanTransitionCommand,
) -> Result<sqlx::postgres::PgRow, ApplicationError> {
    sqlx::query(
        "SELECT version, status, created_by_user_id, expires_at, source_snapshot_hash, \
                latest_simulation_id FROM action_plans \
         WHERE id=$1 AND tenant_id=$2 AND factory_id=$3 FOR UPDATE",
    )
    .bind(command.plan_id)
    .bind(command.tenant_id)
    .bind(command.factory_id)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(repository_error)?
    .ok_or_else(|| ApplicationError::Precondition("action plan was not found in scope".to_owned()))
}

fn verify_action_plan_actor_version(
    row: &sqlx::postgres::PgRow,
    command: &ActionPlanTransitionCommand,
    require_creator: bool,
) -> Result<(), ApplicationError> {
    if require_creator && row.get::<Uuid, _>("created_by_user_id") != command.actor_user_id {
        return Err(ApplicationError::Precondition(
            "only the plan creator can perform this transition".to_owned(),
        ));
    }
    if row.get::<i64, _>("version") != command.expected_version {
        return Err(ApplicationError::Conflict(
            "action-plan version does not match If-Match".to_owned(),
        ));
    }
    Ok(())
}

fn parse_action_plan_row_status(
    row: &sqlx::postgres::PgRow,
) -> Result<ActionPlanStatus, ApplicationError> {
    row.get::<String, _>("status")
        .parse::<ActionPlanStatus>()
        .map_err(repository_error)
}

fn ensure_action_plan_not_expired(row: &sqlx::postgres::PgRow) -> Result<(), ApplicationError> {
    if row.get::<DateTime<Utc>, _>("expires_at") <= Utc::now() {
        return Err(ApplicationError::Precondition(
            "expired action plan cannot transition".to_owned(),
        ));
    }
    Ok(())
}

fn required_snapshot_hash(row: &sqlx::postgres::PgRow) -> Result<String, ApplicationError> {
    row.get::<Option<String>, _>("source_snapshot_hash")
        .ok_or_else(|| ApplicationError::Precondition("validated snapshot is missing".to_owned()))
}

async fn verify_current_simulation(
    transaction: &mut Transaction<'_, Postgres>,
    plan_id: Uuid,
    simulation_id: Uuid,
    snapshot_hash: &str,
) -> Result<(), ApplicationError> {
    let valid = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM action_plan_simulations WHERE id=$1 AND plan_id=$2 \
            AND source_snapshot_hash=$3 AND completion_status='completed' \
            AND protected_state_unchanged)",
    )
    .bind(simulation_id)
    .bind(plan_id)
    .bind(snapshot_hash)
    .fetch_one(&mut **transaction)
    .await
    .map_err(repository_error)?;
    if !valid {
        return Err(ApplicationError::Precondition(
            "current completed simulation does not match the validated snapshot".to_owned(),
        ));
    }
    Ok(())
}

async fn rebuild_transition_snapshot(
    transaction: &mut Transaction<'_, Postgres>,
    command: &ActionPlanTransitionCommand,
) -> Result<BuiltActionPlanSnapshot, ApplicationError> {
    build_action_plan_snapshot(
        transaction,
        &ActionPlanValidationCommand {
            request_id: command.request_id,
            tenant_id: command.tenant_id,
            factory_id: command.factory_id,
            actor_user_id: command.actor_user_id,
            plan_id: command.plan_id,
            expected_version: command.expected_version,
            idempotency_key: command.idempotency_key.clone(),
            request_fingerprint: command.request_fingerprint.clone(),
        },
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn persist_action_plan_transition(
    transaction: &mut Transaction<'_, Postgres>,
    command: &ActionPlanTransitionCommand,
    operation: &str,
    action: &str,
    prior_status: ActionPlanStatus,
    resulting_status: ActionPlanStatus,
    prior_version: i64,
    snapshot_hash: Option<&str>,
    simulation_id: Option<Uuid>,
    review_id: Option<Uuid>,
    decision: Option<ReviewDecision>,
) -> Result<(), ApplicationError> {
    let resource_id = review_id.unwrap_or(command.plan_id);
    sqlx::query(
        "INSERT INTO idempotency_records (\
            id, tenant_id, actor_user_id, operation, idempotency_key, request_hash, \
            resource_type, resource_id, response_status\
         ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,200)",
    )
    .bind(Uuid::new_v4())
    .bind(command.tenant_id)
    .bind(command.actor_user_id)
    .bind(operation)
    .bind(&command.idempotency_key)
    .bind(fingerprint_hash(&command.request_fingerprint))
    .bind(if review_id.is_some() {
        "action_plan_review"
    } else {
        "action_plan"
    })
    .bind(resource_id)
    .execute(&mut **transaction)
    .await
    .map_err(repository_error)?;
    sqlx::query(
        "INSERT INTO action_plan_transition_audits (\
            id, request_id, tenant_id, factory_id, plan_id, actor_user_id, action, \
            prior_status, resulting_status, prior_version, resulting_version, snapshot_hash, \
            simulation_id, review_id, decision, idempotency_outcome\
         ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,'created')",
    )
    .bind(Uuid::new_v4())
    .bind(command.request_id)
    .bind(command.tenant_id)
    .bind(command.factory_id)
    .bind(command.plan_id)
    .bind(command.actor_user_id)
    .bind(action)
    .bind(prior_status.as_str())
    .bind(resulting_status.as_str())
    .bind(prior_version)
    .bind(prior_version + 1)
    .bind(snapshot_hash)
    .bind(simulation_id)
    .bind(review_id)
    .bind(decision.map(ReviewDecision::as_str))
    .execute(&mut **transaction)
    .await
    .map_err(repository_error)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn persist_transition_drift(
    transaction: &mut Transaction<'_, Postgres>,
    command: &ActionPlanTransitionCommand,
    operation: &str,
    prior_status: ActionPlanStatus,
    baseline_hash: &str,
    observed: &BuiltActionPlanSnapshot,
    simulation_id: Option<Uuid>,
) -> Result<(), ApplicationError> {
    let snapshot_id = Uuid::new_v4();
    let prior_version = command.expected_version;
    sqlx::query(
        "INSERT INTO action_plan_snapshots (\
            id, plan_id, plan_version, validation_policy_version, snapshot_hash, \
            canonical_payload, missing_fields, target_count, evidence_count\
         ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)",
    )
    .bind(snapshot_id)
    .bind(command.plan_id)
    .bind(prior_version + 1)
    .bind(ACTION_PLAN_VALIDATION_POLICY_VERSION)
    .bind(&observed.snapshot_hash)
    .bind(&observed.canonical_payload)
    .bind(&observed.missing_fields)
    .bind(observed.target_count)
    .bind(observed.evidence_count)
    .execute(&mut **transaction)
    .await
    .map_err(repository_error)?;
    sqlx::query(
        "UPDATE action_plans SET status='invalidated_by_drift', version=version+1, \
         finished_at=now() WHERE id=$1",
    )
    .bind(command.plan_id)
    .execute(&mut **transaction)
    .await
    .map_err(repository_error)?;
    sqlx::query(
        "INSERT INTO idempotency_records (\
            id, tenant_id, actor_user_id, operation, idempotency_key, request_hash, \
            resource_type, resource_id, response_status\
         ) VALUES ($1,$2,$3,$4,$5,$6,'action_plan_snapshot',$7,409)",
    )
    .bind(Uuid::new_v4())
    .bind(command.tenant_id)
    .bind(command.actor_user_id)
    .bind(operation)
    .bind(&command.idempotency_key)
    .bind(fingerprint_hash(&command.request_fingerprint))
    .bind(snapshot_id)
    .execute(&mut **transaction)
    .await
    .map_err(repository_error)?;
    sqlx::query(
        "INSERT INTO action_plan_transition_audits (\
            id, request_id, tenant_id, factory_id, plan_id, actor_user_id, action, \
            prior_status, resulting_status, prior_version, resulting_version, snapshot_hash, \
            simulation_id, idempotency_outcome\
         ) VALUES ($1,$2,$3,$4,$5,$6,'drift_invalidate',$7,'invalidated_by_drift',\
            $8,$9,$10,$11,'created')",
    )
    .bind(Uuid::new_v4())
    .bind(command.request_id)
    .bind(command.tenant_id)
    .bind(command.factory_id)
    .bind(command.plan_id)
    .bind(command.actor_user_id)
    .bind(prior_status.as_str())
    .bind(prior_version)
    .bind(prior_version + 1)
    .bind(baseline_hash)
    .bind(simulation_id)
    .execute(&mut **transaction)
    .await
    .map_err(repository_error)?;
    Ok(())
}

async fn read_action_plan_review(
    pool: &PgPool,
    review_id: Uuid,
) -> Result<ActionPlanReviewRead, ApplicationError> {
    let row = sqlx::query(
        "SELECT id, plan_id, plan_version, reviewer_user_id, decision, rationale, created_at \
         FROM action_plan_reviews WHERE id=$1",
    )
    .bind(review_id)
    .fetch_optional(pool)
    .await
    .map_err(repository_error)?
    .ok_or_else(|| ApplicationError::Repository("action-plan review disappeared".to_owned()))?;
    Ok(ActionPlanReviewRead {
        id: row.get("id"),
        plan_id: row.get("plan_id"),
        plan_version: row.get("plan_version"),
        reviewer_user_id: row.get("reviewer_user_id"),
        decision: row
            .get::<String, _>("decision")
            .parse::<ReviewDecision>()
            .map_err(repository_error)?,
        rationale: row.get("rationale"),
        created_at: row.get("created_at"),
    })
}

struct BuiltActionPlanSnapshot {
    snapshot_hash: String,
    canonical_payload: Value,
    missing_fields: Vec<String>,
    target_count: i32,
    evidence_count: i32,
}

#[allow(clippy::too_many_lines)]
async fn build_action_plan_snapshot(
    transaction: &mut Transaction<'_, Postgres>,
    command: &ActionPlanValidationCommand,
) -> Result<BuiltActionPlanSnapshot, ApplicationError> {
    let source = sqlx::query(
        "SELECT a.plan_type, a.compiler_policy_version, a.source_evidence_state, \
                p.id AS proposal_id, p.version AS proposal_version, p.status AS proposal_status, \
                p.proposal_type, p.evidence_state AS proposal_evidence_state, r.id AS review_id, \
                r.decision AS review_decision, r.proposal_version AS reviewed_version \
         FROM action_plans a \
         JOIN disposition_proposals p ON (p.id,p.tenant_id,p.factory_id,p.version) = \
            (a.source_proposal_id,a.tenant_id,a.factory_id,a.source_proposal_version) \
         JOIN disposition_reviews r ON (r.id,r.proposal_id,r.tenant_id,r.factory_id) = \
            (a.source_review_id,a.source_proposal_id,a.tenant_id,a.factory_id) \
         WHERE a.id=$1 AND a.tenant_id=$2 AND a.factory_id=$3",
    )
    .bind(command.plan_id)
    .bind(command.tenant_id)
    .bind(command.factory_id)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(repository_error)?
    .ok_or_else(|| {
        ApplicationError::Precondition("trusted source binding is missing".to_owned())
    })?;
    if source.get::<String, _>("proposal_status") != "review_approved"
        || source.get::<String, _>("review_decision") != "approved"
    {
        return Err(ApplicationError::Precondition(
            "source proposal and review are no longer approved".to_owned(),
        ));
    }
    let reviewed_version: i64 = source.get("reviewed_version");
    let proposal_version: i64 = source.get("proposal_version");
    if reviewed_version + 1 != proposal_version {
        return Err(ApplicationError::Precondition(
            "source review does not match the approved proposal history".to_owned(),
        ));
    }
    let plan_type = source
        .get::<String, _>("plan_type")
        .parse::<ActionPlanType>()
        .map_err(repository_error)?;
    let proposal_type = source
        .get::<String, _>("proposal_type")
        .parse::<ProposalType>()
        .map_err(repository_error)?;
    if ActionPlanType::from(proposal_type) != plan_type
        || source.get::<String, _>("source_evidence_state")
            != source.get::<String, _>("proposal_evidence_state")
    {
        return Err(ApplicationError::Precondition(
            "compiled plan type or evidence state no longer matches its source proposal".to_owned(),
        ));
    }
    let step_rows = sqlx::query(
        "SELECT ordinal, step_code FROM action_plan_steps WHERE plan_id=$1 ORDER BY ordinal",
    )
    .bind(command.plan_id)
    .fetch_all(&mut **transaction)
    .await
    .map_err(repository_error)?;
    if step_rows.is_empty() || step_rows.len() > 16 {
        return Err(ApplicationError::Precondition(
            "compiled step profile is missing or exceeds its bound".to_owned(),
        ));
    }
    let mut steps = Vec::with_capacity(step_rows.len());
    let mut typed_steps = Vec::with_capacity(step_rows.len());
    for row in step_rows {
        let step_code: String = row.get("step_code");
        let step = step_code
            .parse::<ActionPlanStep>()
            .map_err(repository_error)?;
        plan_type.validate_step(step).map_err(repository_error)?;
        typed_steps.push(step);
        steps.push(json!({
            "ordinal": row.get::<i32, _>("ordinal"),
            "step_code": step_code,
        }));
    }
    if typed_steps.as_slice() != plan_type.profile_steps() {
        return Err(ApplicationError::Precondition(
            "compiled step order does not match the frozen plan profile".to_owned(),
        ));
    }

    let target_rows = sqlx::query(
        "SELECT ordinal, resource_type, resource_id FROM action_plan_targets \
         WHERE plan_id=$1 ORDER BY ordinal",
    )
    .bind(command.plan_id)
    .fetch_all(&mut **transaction)
    .await
    .map_err(repository_error)?;
    if target_rows.is_empty() || target_rows.len() > 16 {
        return Err(ApplicationError::Precondition(
            "action-plan targets are missing or exceed their bound".to_owned(),
        ));
    }
    let mut targets = Vec::with_capacity(target_rows.len());
    let mut missing_fields = Vec::new();
    for row in target_rows {
        let resource_type: String = row.get("resource_type");
        let resource_id: String = row.get("resource_id");
        let (state, target_missing) = target_snapshot(
            transaction,
            command.tenant_id,
            command.factory_id,
            &resource_type,
            &resource_id,
        )
        .await?;
        missing_fields.extend(target_missing);
        targets.push(json!({
            "ordinal": row.get::<i32, _>("ordinal"),
            "resource_type": resource_type,
            "resource_id": resource_id,
            "state": state,
        }));
    }

    let evidence_rows = sqlx::query(
        "SELECT e.ordinal, e.source_run_id, e.agent_tool_call_id, e.evidence_type, \
                c.tool_name, c.auth_decision, c.result_summary IS NOT NULL AS result_present, \
                c.created_at, encode(digest(COALESCE(c.result_summary::TEXT,''),'sha256'),'hex') \
                    AS result_hash \
         FROM action_plan_evidence e \
         JOIN agent_tool_calls c ON c.id=e.agent_tool_call_id AND c.run_id=e.source_run_id \
         WHERE e.plan_id=$1 ORDER BY e.ordinal",
    )
    .bind(command.plan_id)
    .fetch_all(&mut **transaction)
    .await
    .map_err(repository_error)?;
    if evidence_rows.is_empty() || evidence_rows.len() > 8 {
        return Err(ApplicationError::Precondition(
            "action-plan evidence is missing or exceeds its bound".to_owned(),
        ));
    }
    let mut evidence = Vec::with_capacity(evidence_rows.len());
    for row in evidence_rows {
        let evidence_type: String = row.get("evidence_type");
        let tool_name: String = row.get("tool_name");
        let authorized = row.get::<String, _>("auth_decision") == "authorized";
        let result_present: bool = row.get("result_present");
        if evidence_type != tool_name || !authorized || !result_present {
            return Err(ApplicationError::Precondition(
                "action-plan evidence no longer resolves to an authorized executed result"
                    .to_owned(),
            ));
        }
        evidence.push(json!({
            "ordinal": row.get::<i32, _>("ordinal"),
            "source_run_id": row.get::<Uuid, _>("source_run_id"),
            "tool_call_id": row.get::<Uuid, _>("agent_tool_call_id"),
            "tool_name": tool_name,
            "created_at": row.get::<DateTime<Utc>, _>("created_at"),
            "result_hash": row.get::<String, _>("result_hash"),
        }));
    }
    if plan_type == ActionPlanType::InventoryReconciliationPlan {
        missing_fields.push("inventory_adapter_unavailable".to_owned());
        missing_fields.push("physical_material_identity_unavailable".to_owned());
    }
    missing_fields.sort();
    missing_fields.dedup();
    if missing_fields.len() > 32 {
        return Err(ApplicationError::Precondition(
            "snapshot missingness exceeds its bound".to_owned(),
        ));
    }
    let canonical_payload = json!({
        "validation_policy_version": ACTION_PLAN_VALIDATION_POLICY_VERSION,
        "plan": {
            "id": command.plan_id,
            "plan_type": plan_type.as_str(),
            "compiler_policy_version": source.get::<String, _>("compiler_policy_version"),
            "source_evidence_state": source.get::<String, _>("source_evidence_state"),
            "steps": steps,
        },
        "source": {
            "proposal_id": source.get::<Uuid, _>("proposal_id"),
            "proposal_version": proposal_version,
            "proposal_status": source.get::<String, _>("proposal_status"),
            "proposal_evidence_state": source.get::<String, _>("proposal_evidence_state"),
            "review_id": source.get::<Uuid, _>("review_id"),
            "review_decision": source.get::<String, _>("review_decision"),
            "reviewed_version": reviewed_version,
        },
        "targets": targets,
        "evidence": evidence,
        "missing_fields": missing_fields,
    });
    let serialized = serde_json::to_vec(&canonical_payload).map_err(repository_error)?;
    if serialized.len() > 65_536 {
        return Err(ApplicationError::Precondition(
            "canonical snapshot exceeds its size bound".to_owned(),
        ));
    }
    let snapshot_hash = hex::encode(Sha256::digest(&serialized));
    Ok(BuiltActionPlanSnapshot {
        snapshot_hash,
        canonical_payload,
        missing_fields,
        target_count: i32::try_from(targets.len()).map_err(repository_error)?,
        evidence_count: i32::try_from(evidence.len()).map_err(repository_error)?,
    })
}

async fn target_snapshot(
    transaction: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    factory_id: Uuid,
    resource_type: &str,
    resource_id: &str,
) -> Result<(Value, Vec<String>), ApplicationError> {
    let parsed_id = Uuid::parse_str(resource_id).map_err(|_| {
        ApplicationError::Precondition("action-plan target ID is not a UUID".to_owned())
    })?;
    match resource_type {
        "production_line" => {
            let row = sqlx::query(
                "SELECT l.name, l.created_at, \
                    (SELECT COUNT(*)::BIGINT FROM work_orders w WHERE w.tenant_id=$2 AND w.factory_id=$3 AND w.line_id=l.id) AS work_order_count, \
                    (SELECT encode(digest(COALESCE(string_agg(concat_ws('|',w.id::TEXT,w.status,w.planned_qty::TEXT,w.due_at::TEXT,w.updated_at::TEXT),',' ORDER BY w.id),''),'sha256'),'hex') FROM work_orders w WHERE w.tenant_id=$2 AND w.factory_id=$3 AND w.line_id=l.id) AS work_orders_hash, \
                    (SELECT COUNT(*)::BIGINT FROM production_records p WHERE p.tenant_id=$2 AND p.factory_id=$3 AND p.line_id=l.id) AS production_count, \
                    (SELECT encode(digest(COALESCE(string_agg(concat_ws('|',p.id::TEXT,p.occurred_at::TEXT,p.good_qty::TEXT,p.reject_qty::TEXT),',' ORDER BY p.id),''),'sha256'),'hex') FROM production_records p WHERE p.tenant_id=$2 AND p.factory_id=$3 AND p.line_id=l.id) AS production_hash, \
                    (SELECT COUNT(*)::BIGINT FROM downtime_events d WHERE d.tenant_id=$2 AND d.factory_id=$3 AND d.line_id=l.id) AS downtime_count, \
                    (SELECT encode(digest(COALESCE(string_agg(concat_ws('|',d.id::TEXT,d.machine_id::TEXT,d.started_at::TEXT,d.ended_at::TEXT,d.reason_code),',' ORDER BY d.id),''),'sha256'),'hex') FROM downtime_events d WHERE d.tenant_id=$2 AND d.factory_id=$3 AND d.line_id=l.id) AS downtime_hash, \
                    (SELECT COUNT(*)::BIGINT FROM quality_inspections q WHERE q.tenant_id=$2 AND q.factory_id=$3 AND q.line_id=l.id) AS quality_count, \
                    (SELECT encode(digest(COALESCE(string_agg(concat_ws('|',q.id::TEXT,q.inspected_at::TEXT,q.sample_size::TEXT,q.defect_count::TEXT,COALESCE(q.defect_code,'')),',' ORDER BY q.id),''),'sha256'),'hex') FROM quality_inspections q WHERE q.tenant_id=$2 AND q.factory_id=$3 AND q.line_id=l.id) AS quality_hash \
                 FROM production_lines l WHERE l.id=$1 AND l.tenant_id=$2 AND l.factory_id=$3",
            )
            .bind(parsed_id)
            .bind(tenant_id)
            .bind(factory_id)
            .fetch_optional(&mut **transaction)
            .await
            .map_err(repository_error)?
            .ok_or_else(|| ApplicationError::Precondition("production-line target is unresolved".to_owned()))?;
            Ok((
                json!({
                    "resolved": true,
                    "name": row.get::<String, _>("name"),
                    "created_at": row.get::<DateTime<Utc>, _>("created_at"),
                    "work_orders": {"count": row.get::<i64, _>("work_order_count"), "hash": row.get::<String, _>("work_orders_hash")},
                    "production_records": {"count": row.get::<i64, _>("production_count"), "hash": row.get::<String, _>("production_hash")},
                    "downtime_events": {"count": row.get::<i64, _>("downtime_count"), "hash": row.get::<String, _>("downtime_hash")},
                    "quality_inspections": {"count": row.get::<i64, _>("quality_count"), "hash": row.get::<String, _>("quality_hash")},
                }),
                Vec::new(),
            ))
        }
        "machine" => {
            let row = sqlx::query(
                "SELECT m.line_id, m.name, m.status, m.created_at, \
                    (SELECT COUNT(*)::BIGINT FROM downtime_events d WHERE d.tenant_id=$2 AND d.factory_id=$3 AND d.machine_id=m.id) AS downtime_count, \
                    (SELECT encode(digest(COALESCE(string_agg(concat_ws('|',d.id::TEXT,d.started_at::TEXT,d.ended_at::TEXT,d.reason_code),',' ORDER BY d.id),''),'sha256'),'hex') FROM downtime_events d WHERE d.tenant_id=$2 AND d.factory_id=$3 AND d.machine_id=m.id) AS downtime_hash \
                 FROM machines m WHERE m.id=$1 AND m.tenant_id=$2 AND m.factory_id=$3",
            )
            .bind(parsed_id)
            .bind(tenant_id)
            .bind(factory_id)
            .fetch_optional(&mut **transaction)
            .await
            .map_err(repository_error)?
            .ok_or_else(|| ApplicationError::Precondition("machine target is unresolved".to_owned()))?;
            Ok((
                json!({
                    "resolved": true,
                    "line_id": row.get::<Uuid, _>("line_id"),
                    "name": row.get::<String, _>("name"),
                    "status": row.get::<String, _>("status"),
                    "created_at": row.get::<DateTime<Utc>, _>("created_at"),
                    "downtime_events": {"count": row.get::<i64, _>("downtime_count"), "hash": row.get::<String, _>("downtime_hash")},
                }),
                Vec::new(),
            ))
        }
        _ => Err(ApplicationError::Precondition(
            "unsupported action-plan target type".to_owned(),
        )),
    }
}

async fn read_action_plan_snapshot(
    pool: &PgPool,
    snapshot_id: Uuid,
) -> Result<ActionPlanSnapshotRead, ApplicationError> {
    let row = sqlx::query(
        "SELECT id, plan_id, plan_version, validation_policy_version, snapshot_hash, \
                canonical_payload, missing_fields, target_count, evidence_count, created_at \
         FROM action_plan_snapshots WHERE id=$1",
    )
    .bind(snapshot_id)
    .fetch_optional(pool)
    .await
    .map_err(repository_error)?
    .ok_or_else(|| ApplicationError::Repository("action-plan snapshot disappeared".to_owned()))?;
    Ok(ActionPlanSnapshotRead {
        id: row.get("id"),
        plan_id: row.get("plan_id"),
        plan_version: row.get("plan_version"),
        validation_policy_version: row.get("validation_policy_version"),
        snapshot_hash: row.get("snapshot_hash"),
        canonical_payload: row.get("canonical_payload"),
        missing_fields: row.get("missing_fields"),
        target_count: row.get("target_count"),
        evidence_count: row.get("evidence_count"),
        created_at: row.get("created_at"),
    })
}

async fn read_action_plan_simulation(
    pool: &PgPool,
    simulation_id: Uuid,
) -> Result<ActionPlanSimulationRead, ApplicationError> {
    let row = sqlx::query(
        "SELECT id, plan_id, plan_version, source_snapshot_hash, \
                simulation_policy_version, deterministic_seed, canonical_input_hash, \
                canonical_output_hash, affected_resources, precondition_findings, \
                missing_data_findings, required_human_roles, impact_preview, \
                completion_status, protected_state_unchanged, started_at, completed_at, duration_ms \
         FROM action_plan_simulations WHERE id=$1",
    )
    .bind(simulation_id)
    .fetch_optional(pool)
    .await
    .map_err(repository_error)?
    .ok_or_else(|| {
        ApplicationError::Repository("action-plan simulation disappeared".to_owned())
    })?;
    Ok(ActionPlanSimulationRead {
        id: row.get("id"),
        plan_id: row.get("plan_id"),
        plan_version: row.get("plan_version"),
        source_snapshot_hash: row.get("source_snapshot_hash"),
        simulation_policy_version: row.get("simulation_policy_version"),
        deterministic_seed: row.get("deterministic_seed"),
        canonical_input_hash: row.get("canonical_input_hash"),
        canonical_output_hash: row.get("canonical_output_hash"),
        affected_resources: serde_json::from_value(row.get("affected_resources"))
            .map_err(repository_error)?,
        precondition_findings: row.get("precondition_findings"),
        missing_data_findings: row.get("missing_data_findings"),
        required_human_roles: row.get("required_human_roles"),
        impact_preview: row.get("impact_preview"),
        completion_status: row.get("completion_status"),
        protected_state_unchanged: row.get("protected_state_unchanged"),
        started_at: row.get("started_at"),
        completed_at: row.get("completed_at"),
        duration_ms: row.get("duration_ms"),
    })
}

async fn idempotency_replay(
    transaction: &mut Transaction<'_, Postgres>,
    command: &DispositionTransitionCommand,
    operation: &str,
) -> Result<bool, ApplicationError> {
    let lock_key = format!(
        "{}:{}:{operation}:{}",
        command.tenant_id, command.actor_user_id, command.idempotency_key
    );
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(lock_key)
        .execute(&mut **transaction)
        .await
        .map_err(repository_error)?;
    let request_hash = fingerprint_hash(&command.request_fingerprint);
    let existing = sqlx::query(
        "SELECT request_hash, resource_id FROM idempotency_records \
         WHERE tenant_id=$1 AND actor_user_id=$2 AND operation=$3 AND idempotency_key=$4",
    )
    .bind(command.tenant_id)
    .bind(command.actor_user_id)
    .bind(operation)
    .bind(&command.idempotency_key)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(repository_error)?;
    let Some(existing) = existing else {
        return Ok(false);
    };
    if existing.get::<String, _>("request_hash") != request_hash
        || existing.get::<Uuid, _>("resource_id") != command.proposal_id
    {
        return Err(ApplicationError::Conflict(
            "idempotency key was already used with a different request".to_owned(),
        ));
    }
    Ok(true)
}

async fn record_idempotency(
    transaction: &mut Transaction<'_, Postgres>,
    command: &DispositionTransitionCommand,
    operation: &str,
) -> Result<(), ApplicationError> {
    sqlx::query(
        "INSERT INTO idempotency_records \
         (id, tenant_id, actor_user_id, operation, idempotency_key, request_hash, \
          resource_type, resource_id, response_status) \
         VALUES ($1,$2,$3,$4,$5,$6,'disposition_proposal',$7,200)",
    )
    .bind(Uuid::new_v4())
    .bind(command.tenant_id)
    .bind(command.actor_user_id)
    .bind(operation)
    .bind(&command.idempotency_key)
    .bind(fingerprint_hash(&command.request_fingerprint))
    .bind(command.proposal_id)
    .execute(&mut **transaction)
    .await
    .map_err(repository_error)?;
    Ok(())
}

async fn lock_transition_row(
    transaction: &mut Transaction<'_, Postgres>,
    command: &DispositionTransitionCommand,
) -> Result<sqlx::postgres::PgRow, ApplicationError> {
    sqlx::query(
        "SELECT status, version, initiated_by_user_id, source_run_id, expires_at \
         FROM disposition_proposals \
         WHERE id=$1 AND tenant_id=$2 AND factory_id=$3 FOR UPDATE",
    )
    .bind(command.proposal_id)
    .bind(command.tenant_id)
    .bind(command.factory_id)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(repository_error)?
    .ok_or_else(|| ApplicationError::Precondition("proposal is outside scope".to_owned()))
}

fn validate_actor_version_status(
    row: &sqlx::postgres::PgRow,
    command: &DispositionTransitionCommand,
    require_initiator: bool,
    allowed_statuses: &[ProposalStatus],
) -> Result<(), ApplicationError> {
    let initiated_by: Uuid = row.get("initiated_by_user_id");
    let actor_matches = initiated_by == command.actor_user_id;
    if actor_matches != require_initiator {
        return Err(ApplicationError::Precondition(
            "proposal actor violates separation of duties".to_owned(),
        ));
    }
    if row.get::<i64, _>("version") != command.expected_version {
        return Err(ApplicationError::Conflict(
            "proposal version is stale".to_owned(),
        ));
    }
    let status = row
        .get::<String, _>("status")
        .parse::<ProposalStatus>()
        .map_err(repository_error)?;
    if !allowed_statuses.contains(&status) {
        return Err(ApplicationError::Precondition(
            "proposal status does not allow this transition".to_owned(),
        ));
    }
    Ok(())
}

async fn transition_evidence_count(
    transaction: &mut Transaction<'_, Postgres>,
    proposal_id: Uuid,
) -> Result<i32, ApplicationError> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)::BIGINT FROM disposition_proposal_evidence WHERE proposal_id=$1",
    )
    .bind(proposal_id)
    .fetch_one(&mut **transaction)
    .await
    .map_err(repository_error)?;
    i32::try_from(count).map_err(repository_error)
}

#[allow(clippy::too_many_arguments)]
async fn record_transition(
    transaction: &mut Transaction<'_, Postgres>,
    command: &DispositionTransitionCommand,
    source_run_id: Uuid,
    action: &str,
    prior_status: ProposalStatus,
    resulting_status: ProposalStatus,
    prior_version: i64,
    evidence_count: i32,
) -> Result<(), ApplicationError> {
    sqlx::query(
        "INSERT INTO disposition_transition_audits \
         (id, request_id, tenant_id, factory_id, proposal_id, source_run_id, actor_user_id, \
          action, prior_status, resulting_status, prior_version, resulting_version, outcome, \
          idempotency_outcome, evidence_reference_count) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,'completed','created',$13)",
    )
    .bind(Uuid::new_v4())
    .bind(command.request_id)
    .bind(command.tenant_id)
    .bind(command.factory_id)
    .bind(command.proposal_id)
    .bind(source_run_id)
    .bind(command.actor_user_id)
    .bind(action)
    .bind(prior_status.as_str())
    .bind(resulting_status.as_str())
    .bind(prior_version)
    .bind(prior_version + 1)
    .bind(evidence_count)
    .execute(&mut **transaction)
    .await
    .map_err(repository_error)?;
    Ok(())
}

async fn transition_result(
    repository: &PostgresMesRepository,
    command: &DispositionTransitionCommand,
    include_review: bool,
    replayed: bool,
) -> Result<DispositionTransitionResult, ApplicationError> {
    let proposal = repository
        .get_disposition_proposal(command.tenant_id, command.factory_id, command.proposal_id)
        .await?
        .ok_or_else(|| ApplicationError::Repository("transitioned proposal missing".to_owned()))?;
    let review = if include_review {
        get_disposition_review(&repository.pool, command.proposal_id).await?
    } else {
        None
    };
    Ok(DispositionTransitionResult {
        proposal,
        review,
        replayed,
    })
}

async fn get_disposition_review(
    pool: &PgPool,
    proposal_id: Uuid,
) -> Result<Option<DispositionReviewRead>, ApplicationError> {
    let row = sqlx::query(
        "SELECT id, proposal_id, proposal_version, reviewer_user_id, decision, rationale, \
         created_at FROM disposition_reviews WHERE proposal_id=$1",
    )
    .bind(proposal_id)
    .fetch_optional(pool)
    .await
    .map_err(repository_error)?;
    let Some(row) = row else { return Ok(None) };
    Ok(Some(DispositionReviewRead {
        id: row.get("id"),
        proposal_id: row.get("proposal_id"),
        proposal_version: row.get("proposal_version"),
        reviewer_user_id: row.get("reviewer_user_id"),
        decision: row
            .get::<String, _>("decision")
            .parse::<ReviewDecision>()
            .map_err(repository_error)?,
        rationale: row.get("rationale"),
        created_at: row.get("created_at"),
    }))
}

async fn expire_pending_proposal(
    pool: &PgPool,
    tenant_id: Uuid,
    factory_id: Uuid,
    proposal_id: Uuid,
) -> Result<(), ApplicationError> {
    let mut transaction = pool.begin().await.map_err(repository_error)?;
    let expired = sqlx::query(
        "UPDATE disposition_proposals \
         SET status='expired', version=version+1, finished_at=now() \
         WHERE id=$1 AND tenant_id=$2 AND factory_id=$3 \
           AND status='pending_review' AND expires_at <= now() \
         RETURNING source_run_id, version-1 AS prior_version",
    )
    .bind(proposal_id)
    .bind(tenant_id)
    .bind(factory_id)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(repository_error)?;
    if let Some(expired) = expired {
        let prior_version: i64 = expired.get("prior_version");
        let evidence_count = transition_evidence_count(&mut transaction, proposal_id).await?;
        sqlx::query(
            "INSERT INTO disposition_transition_audits \
             (id, request_id, tenant_id, factory_id, proposal_id, source_run_id, actor_user_id, \
              action, prior_status, resulting_status, prior_version, resulting_version, outcome, \
              idempotency_outcome, evidence_reference_count) \
             VALUES ($1,$2,$3,$4,$5,$6,NULL,'expire','pending_review','expired',$7,$8, \
                     'completed','not_applicable',$9)",
        )
        .bind(Uuid::new_v4())
        .bind(Uuid::new_v4())
        .bind(tenant_id)
        .bind(factory_id)
        .bind(proposal_id)
        .bind(expired.get::<Uuid, _>("source_run_id"))
        .bind(prior_version)
        .bind(prior_version + 1)
        .bind(evidence_count)
        .execute(&mut *transaction)
        .await
        .map_err(repository_error)?;
    }
    transaction.commit().await.map_err(repository_error)
}

fn fingerprint_hash(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}

async fn line_in_scope(
    pool: &PgPool,
    tenant_id: Uuid,
    factory_id: Uuid,
    line_id: Uuid,
) -> Result<bool, ApplicationError> {
    sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM production_lines WHERE id=$1 AND tenant_id=$2 AND factory_id=$3)",
    )
    .bind(line_id)
    .bind(tenant_id)
    .bind(factory_id)
    .fetch_one(pool)
    .await
    .map_err(repository_error)
}

fn union_minutes(
    events: &[DowntimeEvidence],
    window_start: DateTime<Utc>,
    window_end: DateTime<Utc>,
) -> i64 {
    let mut intervals: Vec<_> = events
        .iter()
        .map(|event| {
            (
                event.started_at.max(window_start),
                event.ended_at.min(window_end),
            )
        })
        .filter(|(start, end)| end > start)
        .collect();
    intervals.sort_by_key(|(start, _)| *start);
    let mut total_seconds = 0;
    let Some((mut current_start, mut current_end)) = intervals.first().copied() else {
        return 0;
    };
    for (start, end) in intervals.into_iter().skip(1) {
        if start <= current_end {
            current_end = current_end.max(end);
        } else {
            total_seconds += (current_end - current_start).num_seconds();
            current_start = start;
            current_end = end;
        }
    }
    total_seconds += (current_end - current_start).num_seconds();
    total_seconds / 60
}

const fn status_name(status: WorkOrderStatus) -> &'static str {
    match status {
        WorkOrderStatus::Planned => "planned",
        WorkOrderStatus::Released => "released",
        WorkOrderStatus::InProgress => "in_progress",
        WorkOrderStatus::Completed => "completed",
        WorkOrderStatus::Cancelled => "cancelled",
    }
}

fn repository_error(error: impl std::fmt::Display) -> ApplicationError {
    ApplicationError::Repository(error.to_string())
}

fn parse_status(value: &str) -> Result<WorkOrderStatus, ApplicationError> {
    match value {
        "planned" => Ok(WorkOrderStatus::Planned),
        "released" => Ok(WorkOrderStatus::Released),
        "in_progress" => Ok(WorkOrderStatus::InProgress),
        "completed" => Ok(WorkOrderStatus::Completed),
        "cancelled" => Ok(WorkOrderStatus::Cancelled),
        _ => Err(ApplicationError::Repository(format!(
            "database returned invalid work-order status: {value}"
        ))),
    }
}
