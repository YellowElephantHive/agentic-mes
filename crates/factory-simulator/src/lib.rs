use chrono::{Duration, NaiveDate, TimeZone, Utc};
use mes_application::{
    DowntimeSeed, FactorySeed, FactorySeedPlan, InspectionSeed, LineSeed, MachineSeed, ProductSeed,
    ProductionRecordSeed, TenantSeed, WorkOrderSeed,
};
use mes_domain::{PlannedQuantity, Quantity, WorkOrderStatus};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use uuid::Uuid;

pub const DEFAULT_LAST_DATE: NaiveDate = NaiveDate::from_ymd_opt(2026, 1, 30).unwrap();

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimulatorConfig {
    pub seed: u64,
    pub days: u32,
    pub factory_slug: String,
    pub last_date: NaiveDate,
}

/// Builds a deterministic, internally coherent factory seed plan.
///
/// # Errors
///
/// Returns an error when `days` is zero.
///
/// # Panics
///
/// Panics only if fixed, compile-time-valid quantities, dates, or small indexes
/// cannot be represented by their declared types.
#[allow(clippy::too_many_lines)]
pub fn generate_plan(config: &SimulatorConfig) -> Result<FactorySeedPlan, String> {
    if config.days == 0 {
        return Err("days must be positive".to_owned());
    }
    let first_date = config.last_date - Duration::days(i64::from(config.days - 1));
    let mut rng = ChaCha8Rng::seed_from_u64(config.seed);
    let tenant_id = stable_id(config, "tenant");
    let factory_id = stable_id(config, "factory");
    let tenant = TenantSeed {
        id: tenant_id,
        name: format!("Simulator tenant {} ({})", config.seed, config.factory_slug),
    };
    let factory = FactorySeed {
        id: factory_id,
        tenant_id,
        name: "Demo Factory".to_owned(),
        slug: config.factory_slug.clone(),
        timezone: "America/New_York".to_owned(),
    };
    let lines: Vec<_> = ["Line A", "Line B", "Line C"]
        .into_iter()
        .enumerate()
        .map(|(index, name)| LineSeed {
            id: stable_id(config, &format!("line-{index}")),
            tenant_id,
            factory_id,
            name: name.to_owned(),
        })
        .collect();
    let machines: Vec<_> = lines
        .iter()
        .enumerate()
        .flat_map(|(line_index, line)| {
            (0..2).map(move |machine_index| MachineSeed {
                id: stable_id(config, &format!("machine-{line_index}-{machine_index}")),
                tenant_id,
                factory_id,
                line_id: line.id,
                name: format!("{}-M{}", line.name.replace(' ', "-"), machine_index + 1),
                status: "available".to_owned(),
            })
        })
        .collect();
    let products: Vec<_> = (0..8)
        .map(|index| ProductSeed {
            id: stable_id(config, &format!("product-{index}")),
            tenant_id,
            sku: format!("SKU-{:03}", index + 1),
            ideal_cycle_seconds: 24.0 + f64::from(index),
        })
        .collect();
    let work_orders: Vec<_> = (0..120)
        .map(|index| {
            let line = &lines[index % lines.len()];
            let due_date = first_date + Duration::days(i64::try_from(index % 30).unwrap());
            WorkOrderSeed {
                id: stable_id(config, &format!("work-order-{index}")),
                tenant_id,
                factory_id,
                line_id: line.id,
                product_id: products[index % products.len()].id,
                status: WorkOrderStatus::Completed,
                planned_qty: PlannedQuantity::new(1_200).expect("constant is positive"),
                due_at: Utc.from_utc_datetime(
                    &due_date
                        .and_hms_opt(23, 0, 0)
                        .expect("constant time is valid"),
                ),
            }
        })
        .collect();

    let mut production_records = Vec::new();
    let mut downtime_events = Vec::new();
    let mut quality_inspections = Vec::new();
    for day_offset in 0..config.days {
        let date = first_date + Duration::days(i64::from(day_offset));
        for (line_index, line) in lines.iter().enumerate() {
            let order_index = usize::try_from(day_offset).unwrap() * 3 + line_index;
            let work_order = &work_orders[order_index];
            for machine_index in 0..2 {
                let machine = &machines[line_index * 2 + machine_index];
                let good_qty = scenario_good_qty(
                    config,
                    day_offset,
                    line_index,
                    rng.random_range(-30_i64..=30),
                );
                let reject_qty = if line_index == 1 && day_offset == 14 {
                    20
                } else {
                    5 + rng.random_range(0_i64..=4)
                };
                production_records.push(ProductionRecordSeed {
                    id: stable_id(
                        config,
                        &format!("production-{day_offset}-{line_index}-{machine_index}"),
                    ),
                    tenant_id,
                    factory_id,
                    line_id: line.id,
                    work_order_id: work_order.id,
                    machine_id: machine.id,
                    occurred_at: at_utc(
                        date,
                        16,
                        u32::try_from(machine_index).expect("machine index is at most one") * 10,
                    ),
                    good_qty: Quantity::new(good_qty).expect("generated quantity is non-negative"),
                    reject_qty: Quantity::new(reject_qty)
                        .expect("generated quantity is non-negative"),
                });
            }
            let spike = line_index == 1 && day_offset == 14;
            quality_inspections.push(InspectionSeed {
                id: stable_id(config, &format!("inspection-{day_offset}-{line_index}")),
                tenant_id,
                factory_id,
                line_id: line.id,
                work_order_id: work_order.id,
                inspected_at: at_utc(date, 17, 0),
                sample_size: Quantity::new(100).expect("constant is non-negative"),
                defect_count: Quantity::new(if spike { 20 } else { 2 })
                    .expect("constant is non-negative"),
                defect_code: spike.then(|| "SURFACE_DEFECT".to_owned()),
                source_scenario_id: spike.then(|| "SCN-QUALITY-SPIKE-LINE-B".to_owned()),
            });
        }
    }

    add_line_a_drop_scenarios(
        config,
        tenant_id,
        factory_id,
        &lines,
        &machines,
        &mut downtime_events,
    );

    Ok(FactorySeedPlan {
        seed: config.seed,
        first_date,
        last_date: config.last_date,
        tenant,
        factory,
        lines,
        machines,
        products,
        work_orders,
        production_records,
        downtime_events,
        quality_inspections,
    })
}

fn scenario_good_qty(
    config: &SimulatorConfig,
    day_offset: u32,
    line_index: usize,
    random_delta: i64,
) -> i64 {
    if line_index == 0 && day_offset == config.days - 1 {
        300
    } else if line_index == 0 && day_offset == config.days.saturating_sub(2) {
        500
    } else {
        500 + random_delta
    }
}

fn add_line_a_drop_scenarios(
    config: &SimulatorConfig,
    tenant_id: Uuid,
    factory_id: Uuid,
    lines: &[LineSeed],
    machines: &[MachineSeed],
    events: &mut Vec<DowntimeSeed>,
) {
    let date = config.last_date;
    events.push(DowntimeSeed {
        id: stable_id(config, "scenario-machine-failure"),
        tenant_id,
        factory_id,
        line_id: lines[0].id,
        machine_id: machines[0].id,
        started_at: at_utc(date, 8, 0),
        ended_at: at_utc(date, 14, 0),
        reason_code: "MACHINE_FAILURE".to_owned(),
        source_scenario_id: "SCN-LINE-A-MACHINE-FAILURE".to_owned(),
    });
    events.push(DowntimeSeed {
        id: stable_id(config, "scenario-material-shortage"),
        tenant_id,
        factory_id,
        line_id: lines[0].id,
        machine_id: machines[1].id,
        started_at: at_utc(date, 10, 0),
        ended_at: at_utc(date, 12, 0),
        reason_code: "MATERIAL_SHORTAGE".to_owned(),
        source_scenario_id: "SCN-LINE-A-MATERIAL-SHORTAGE".to_owned(),
    });
}

fn stable_id(config: &SimulatorConfig, key: &str) -> Uuid {
    let name = format!("{}:{}:{key}", config.seed, config.factory_slug);
    Uuid::new_v5(&Uuid::NAMESPACE_URL, name.as_bytes())
}

fn at_utc(date: NaiveDate, hour: u32, minute: u32) -> chrono::DateTime<Utc> {
    Utc.from_utc_datetime(
        &date
            .and_hms_opt(hour, minute, 0)
            .expect("generated time is valid"),
    )
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_LAST_DATE, SimulatorConfig, generate_plan};

    #[test]
    fn same_seed_produces_identical_plan_and_expected_scenarios() {
        let config = SimulatorConfig {
            seed: 42,
            days: 30,
            factory_slug: "demo".to_owned(),
            last_date: DEFAULT_LAST_DATE,
        };
        let first = generate_plan(&config).expect("plan must generate");
        let second = generate_plan(&config).expect("plan must generate");
        assert_eq!(first, second);
        assert_eq!(first.lines.len(), 3);
        assert_eq!(first.machines.len(), 6);
        assert_eq!(first.products.len(), 8);
        assert_eq!(first.work_orders.len(), 120);
        assert_eq!(first.production_records.len(), 180);
        assert_eq!(first.quality_inspections.len(), 90);
        assert_eq!(first.downtime_events.len(), 2);

        let line_a = first.lines[0].id;
        let final_day_total: i64 = first
            .production_records
            .iter()
            .filter(|record| {
                record.line_id == line_a && record.occurred_at.date_naive() == DEFAULT_LAST_DATE
            })
            .map(|record| record.good_qty.get())
            .sum();
        assert_eq!(final_day_total, 600);
    }
}
