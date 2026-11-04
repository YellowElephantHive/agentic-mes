use std::{env, path::PathBuf};

use chrono::NaiveDate;
use clap::{Parser, Subcommand};
use factory_simulator::{SimulatorConfig, generate_plan};
use mes_application::{DatasetAdapter, SeedRepository};
use mes_infra::{
    PostgresMesRepository,
    datasets::{Ai4iAdapter, Ai4iInput, SecomAdapter, SecomInput},
};

#[derive(Debug, Parser)]
#[command(about = "Deterministic MES data and source-dataset importer")]
struct Cli {
    #[arg(long, env = "DATABASE_URL")]
    database_url: Option<String>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Seed {
        #[arg(long, default_value_t = 42)]
        seed: u64,
        #[arg(long, default_value_t = 30)]
        days: u32,
        #[arg(long, default_value = "demo")]
        factory: String,
        #[arg(long, default_value = "2026-01-30")]
        last_date: NaiveDate,
    },
    ImportAi4i {
        #[arg(long)]
        csv: PathBuf,
        #[arg(long, default_value = "UCI snapshot retrieved 2026-11-03")]
        version: String,
    },
    ImportSecom {
        #[arg(long)]
        data: PathBuf,
        #[arg(long)]
        labels: PathBuf,
        #[arg(long, default_value = "UCI snapshot retrieved 2026-11-03")]
        version: String,
    },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let database_url = cli.database_url.or_else(|| env::var("DATABASE_URL").ok());
    let repository = match database_url {
        Some(url) => Some(PostgresMesRepository::connect(&url).await?),
        None => None,
    };

    match cli.command {
        Command::Seed {
            seed,
            days,
            factory,
            last_date,
        } => {
            let repository = require_repository(repository)?;
            repository.migrate().await?;
            let plan = generate_plan(&SimulatorConfig {
                seed,
                days,
                factory_slug: factory,
                last_date,
            })?;
            let summary = repository.replace_seed_plan(&plan).await?;
            println!("{}", serde_json::to_string_pretty(&summary)?);
        }
        Command::ImportAi4i { csv, version } => {
            let import = Ai4iAdapter.import(Ai4iInput {
                csv_path: csv,
                version,
            })?;
            persist_or_preview(repository, &import).await?;
        }
        Command::ImportSecom {
            data,
            labels,
            version,
        } => {
            let import = SecomAdapter.import(SecomInput {
                data_path: data,
                labels_path: labels,
                version,
            })?;
            persist_or_preview(repository, &import).await?;
        }
    }
    Ok(())
}

fn require_repository(
    repository: Option<PostgresMesRepository>,
) -> Result<PostgresMesRepository, Box<dyn std::error::Error>> {
    repository.ok_or_else(|| "DATABASE_URL is required for this command".into())
}

async fn persist_or_preview(
    repository: Option<PostgresMesRepository>,
    import: &mes_application::DatasetImport,
) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(repository) = repository {
        repository.migrate().await?;
        let summary = repository.persist_dataset_import(None, import).await?;
        println!(
            "{}",
            serde_json::json!({
                "dataset": import.manifest,
                "persistence": {
                    "import_id": summary.import_id,
                    "record_count": summary.record_count,
                }
            })
        );
    } else {
        println!(
            "{}",
            serde_json::json!({
                "dataset": import.manifest,
                "record_count": import.records.len(),
                "persistence": "not requested; set DATABASE_URL to persist"
            })
        );
    }
    Ok(())
}
