use async_trait::async_trait;
use chrono::{DateTime, Utc};
use mes_application::{
    AgentAuditRepository, AgentRunRead, AgentRunStart, AgentToolAudit, AgentToolCallRead,
    ApplicationError, AuthorizationAuditRepository, AuthorizationDecisionAudit, DatasetGroundTruth,
    DatasetImport, DatasetReadRepository, DispositionCreateResult, DispositionProposalDraft,
    DispositionProposalRead, DispositionProposalRepository, DispositionReviewCommand,
    DispositionReviewRead, DispositionTransitionCommand, DispositionTransitionResult,
    DowntimeEvidence, DowntimeSummary, EvaluationTrack, EvidenceState, FactorySeedPlan,
    IdentityAccess, IdentityRepository, MesReadRepository, ProductionEvidence, ProductionSummary,
    ProductionTotals, ProposalStatus, ProposalType, QualityEvidence, QualitySummary,
    RecommendationCode, ReviewDecision, SeedRepository, SeedSummary, WorkOrderRead,
};
use mes_domain::WorkOrderStatus;
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Row, Transaction, postgres::PgPoolOptions};
use uuid::Uuid;

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
