use std::{collections::BTreeMap, fs, path::PathBuf};

use mes_application::{
    ApplicationError, DatasetAdapter, DatasetImport, DatasetManifest, EvaluationTrack, SourceRecord,
};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

const SOURCE_URL: &str =
    "https://archive.ics.uci.edu/dataset/601/ai4i+2020+predictive+maintenance+dataset";

#[derive(Debug, Clone)]
pub struct Ai4iInput {
    pub csv_path: PathBuf,
    pub version: String,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Ai4iAdapter;

impl DatasetAdapter for Ai4iAdapter {
    type Input = Ai4iInput;

    fn import(&self, input: Self::Input) -> Result<DatasetImport, ApplicationError> {
        let bytes = fs::read(&input.csv_path).map_err(dataset_error)?;
        let checksum = hex::encode(Sha256::digest(&bytes));
        let mut reader = csv::ReaderBuilder::new()
            .trim(csv::Trim::None)
            .from_reader(bytes.as_slice());
        let headers = reader.headers().map_err(dataset_error)?.clone();
        let mut records = Vec::new();

        for (index, row) in reader.records().enumerate() {
            let row = row.map_err(dataset_error)?;
            let values: BTreeMap<String, String> = headers
                .iter()
                .zip(row.iter())
                .map(|(header, value)| (header.to_owned(), value.to_owned()))
                .collect();
            let source_id = values
                .get("UDI")
                .or_else(|| values.get("UID"))
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    ApplicationError::Dataset(format!("AI4I row {} has no UDI/UID", index + 2))
                })?
                .clone();
            let (normalized_payload, missing_fields) = normalize(&values);
            let raw_payload = serde_json::to_value(&values).map_err(dataset_error)?;

            records.push(SourceRecord {
                source_record_id: source_id,
                source_locator: format!("{}:row:{}", input.csv_path.display(), index + 2),
                raw_payload,
                normalized_payload,
                missing_fields,
            });
        }

        Ok(DatasetImport {
            manifest: DatasetManifest {
                dataset_key: "uci-ai4i-2020".to_owned(),
                track: EvaluationTrack::PublishedSynthetic,
                name: "AI4I 2020 Predictive Maintenance Dataset".to_owned(),
                version: input.version,
                license: "CC BY 4.0".to_owned(),
                source_url: SOURCE_URL.to_owned(),
                source_files: vec![file_name(&input.csv_path)?],
                sha256: checksum,
            },
            records,
        })
    }
}

fn normalize(values: &BTreeMap<String, String>) -> (Value, Vec<String>) {
    let mut missing = Vec::new();
    let mut normalized = Map::new();
    copy_text(
        values,
        &mut normalized,
        &mut missing,
        "Product ID",
        "product_id",
    );
    copy_text(
        values,
        &mut normalized,
        &mut missing,
        "Type",
        "product_type",
    );
    copy_number(
        values,
        &mut normalized,
        &mut missing,
        "Air temperature [K]",
        "air_temperature_k",
    );
    copy_number(
        values,
        &mut normalized,
        &mut missing,
        "Process temperature [K]",
        "process_temperature_k",
    );
    copy_number(
        values,
        &mut normalized,
        &mut missing,
        "Rotational speed [rpm]",
        "rotational_speed_rpm",
    );
    copy_number(
        values,
        &mut normalized,
        &mut missing,
        "Torque [Nm]",
        "torque_nm",
    );
    copy_number(
        values,
        &mut normalized,
        &mut missing,
        "Tool wear [min]",
        "tool_wear_minutes",
    );
    for field in ["Machine failure", "TWF", "HDF", "PWF", "OSF", "RNF"] {
        copy_integer(
            values,
            &mut normalized,
            &mut missing,
            field,
            snake_case(field),
        );
    }
    normalized.insert(
        "dataset_scope".to_owned(),
        json!("predictive_maintenance_observation"),
    );
    (Value::Object(normalized), missing)
}

fn copy_text(
    source: &BTreeMap<String, String>,
    target: &mut Map<String, Value>,
    missing: &mut Vec<String>,
    source_key: &str,
    target_key: &str,
) {
    match source.get(source_key).filter(|value| !value.is_empty()) {
        Some(value) => {
            target.insert(target_key.to_owned(), json!(value));
        }
        None => mark_missing(target, missing, target_key),
    }
}

fn copy_number(
    source: &BTreeMap<String, String>,
    target: &mut Map<String, Value>,
    missing: &mut Vec<String>,
    source_key: &str,
    target_key: &str,
) {
    match source
        .get(source_key)
        .filter(|value| !value.is_empty())
        .and_then(|value| value.parse::<f64>().ok())
    {
        Some(value) => {
            target.insert(target_key.to_owned(), json!(value));
        }
        None => mark_missing(target, missing, target_key),
    }
}

fn copy_integer(
    source: &BTreeMap<String, String>,
    target: &mut Map<String, Value>,
    missing: &mut Vec<String>,
    source_key: &str,
    target_key: String,
) {
    match source
        .get(source_key)
        .filter(|value| !value.is_empty())
        .and_then(|value| value.parse::<i64>().ok())
    {
        Some(value) => {
            target.insert(target_key, json!(value));
        }
        None => mark_missing(target, missing, &target_key),
    }
}

fn mark_missing(target: &mut Map<String, Value>, missing: &mut Vec<String>, key: &str) {
    target.insert(key.to_owned(), Value::Null);
    missing.push(key.to_owned());
}

fn snake_case(value: &str) -> String {
    value.to_ascii_lowercase().replace(' ', "_")
}

fn file_name(path: &std::path::Path) -> Result<String, ApplicationError> {
    path.file_name()
        .and_then(|value| value.to_str())
        .map(ToOwned::to_owned)
        .ok_or_else(|| ApplicationError::Dataset("dataset path has no UTF-8 filename".to_owned()))
}

fn dataset_error(error: impl std::fmt::Display) -> ApplicationError {
    ApplicationError::Dataset(error.to_string())
}
