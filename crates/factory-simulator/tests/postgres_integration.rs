use chrono::{Duration, TimeZone, Utc};
use factory_simulator::{DEFAULT_LAST_DATE, SimulatorConfig, generate_plan};
use mes_application::{MesReadRepository, SeedRepository};
use mes_infra::PostgresMesRepository;
use uuid::Uuid;

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL and an isolated PostgreSQL database"]
async fn repeatable_seed_is_queryable_and_tenant_scoped() {
    let database_url = std::env::var("TEST_DATABASE_URL")
        .expect("TEST_DATABASE_URL must be set for ignored integration tests");
    let repository = PostgresMesRepository::connect(&database_url)
        .await
        .expect("database must connect");
    repository.migrate().await.expect("migrations must run");

    let config = SimulatorConfig {
        seed: 42,
        days: 30,
        factory_slug: "integration-test".to_owned(),
        last_date: DEFAULT_LAST_DATE,
    };
    let plan = generate_plan(&config).expect("plan must generate");
    let first = repository
        .replace_seed_plan(&plan)
        .await
        .expect("first seed must persist");
    let second = repository
        .replace_seed_plan(&plan)
        .await
        .expect("repeated seed must replace deterministically");
    assert_eq!(first, second);
    assert_eq!(second.work_orders, 120);

    let line_a = plan.lines[0].id;
    let final_start = Utc.from_utc_datetime(
        &DEFAULT_LAST_DATE
            .and_hms_opt(0, 0, 0)
            .expect("constant time is valid"),
    );
    let final_totals = repository
        .production_totals(
            plan.tenant.id,
            plan.factory.id,
            line_a,
            final_start,
            final_start + Duration::days(1),
        )
        .await
        .expect("final totals must query");
    let previous_totals = repository
        .production_totals(
            plan.tenant.id,
            plan.factory.id,
            line_a,
            final_start - Duration::days(1),
            final_start,
        )
        .await
        .expect("previous totals must query");
    assert_eq!(final_totals.good_qty, 600);
    assert_eq!(previous_totals.good_qty, 1_000);
    assert_eq!(final_totals.record_count, 2);

    let denied_scope = repository
        .production_totals(
            Uuid::new_v4(),
            plan.factory.id,
            line_a,
            final_start,
            final_start + Duration::days(1),
        )
        .await
        .expect("wrong tenant query must safely return no rows");
    assert_eq!(denied_scope.record_count, 0);
    assert_eq!(denied_scope.good_qty, 0);

    sqlx::query("DELETE FROM tenants WHERE id = $1")
        .bind(plan.tenant.id)
        .execute(repository.pool())
        .await
        .expect("test tenant cleanup must succeed");
}
