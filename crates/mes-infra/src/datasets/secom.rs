use std::{fs, path::PathBuf};

use chrono::{NaiveDateTime, TimeZone, Utc};
use mes_application::{
    ApplicationError, DatasetAdapter, DatasetImport, DatasetManifest, EvaluationTrack, SourceRecord,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const SOURCE_URL: &str = "https://archive.ics.uci.edu/dataset/179/secom";

#[derive(Debug, Clone)]
pub struct SecomInput {
    pub data_path: PathBuf,
    pub labels_path: PathBuf,
    pub version: String,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SecomAdapter;

impl DatasetAdapter for SecomAdapter {
    type Input = SecomInput;

    fn import(&self, input: Self::Input) -> Result<DatasetImport, ApplicationError> {
        let data = fs::read(&input.data_path).map_err(dataset_error)?;
        let labels = fs::read(&input.labels_path).map_err(dataset_error)?;
        let data_text = std::str::from_utf8(&data).map_err(dataset_error)?;
        let labels_text = std::str::from_utf8(&labels).map_err(dataset_error)?;
        let data_lines: Vec<_> = data_text.lines().collect();
        let label_lines: Vec<_> = labels_text.lines().collect();
        if data_lines.len() != label_lines.len() {
            return Err(ApplicationError::Dataset(format!(
                "SECOM data/label row mismatch: {} versus {}",
                data_lines.len(),
                label_lines.len()
            )));
        }

        let mut hasher = Sha256::new();
        hasher.update(&data);
        hasher.update([0]);
        hasher.update(&labels);
        let checksum = hex::encode(hasher.finalize());
        let mut records = Vec::with_capacity(data_lines.len());

        for (index, (data_line, label_line)) in
            data_lines.iter().zip(label_lines.iter()).enumerate()
        {
            records.push(parse_record(index, data_line, label_line, &input)?);
        }

        Ok(DatasetImport {
            manifest: DatasetManifest {
                dataset_key: "uci-secom".to_owned(),
                track: EvaluationTrack::RealWorld,
                name: "SECOM".to_owned(),
                version: input.version,
                license: "CC BY 4.0".to_owned(),
                source_url: SOURCE_URL.to_owned(),
                source_files: vec![file_name(&input.data_path)?, file_name(&input.labels_path)?],
                sha256: checksum,
            },
            records,
        })
    }
}

fn parse_record(
    index: usize,
    data_line: &str,
    label_line: &str,
    input: &SecomInput,
) -> Result<SourceRecord, ApplicationError> {
    let tokens: Vec<_> = data_line.split_whitespace().collect();
    if tokens.len() != 590 {
        return Err(ApplicationError::Dataset(format!(
            "SECOM row {} has {} sensor fields; expected 590",
            index + 1,
            tokens.len()
        )));
    }
    let label_parts: Vec<_> = label_line.split_whitespace().collect();
    if label_parts.len() != 3 {
        return Err(ApplicationError::Dataset(format!(
            "SECOM label row {} is malformed",
            index + 1
        )));
    }
    let label = label_parts[0].parse::<i32>().map_err(dataset_error)?;
    if !matches!(label, -1 | 1) {
        return Err(ApplicationError::Dataset(format!(
            "SECOM label row {} is not -1 or 1",
            index + 1
        )));
    }
    let timestamp_source = format!("{} {}", label_parts[1], label_parts[2]);
    let timestamp_value = timestamp_source.trim_matches('"');
    let naive =
        NaiveDateTime::parse_from_str(timestamp_value, "%d/%m/%Y %H:%M:%S").map_err(|error| {
            ApplicationError::Dataset(format!(
                "SECOM timestamp row {} ({timestamp_source}) is invalid: {error}",
                index + 1
            ))
        })?;
    let timestamp = Utc.from_utc_datetime(&naive);

    let mut missing_fields = Vec::new();
    let sensors: Vec<Value> = tokens
        .iter()
        .enumerate()
        .map(|(sensor_index, token)| -> Result<Value, ApplicationError> {
            if *token == "NaN" {
                missing_fields.push(format!("sensor_{sensor_index}"));
                Ok(Value::Null)
            } else {
                token
                    .parse::<f64>()
                    .map(|value| json!(value))
                    .map_err(|error| {
                        ApplicationError::Dataset(format!(
                            "SECOM row {}, sensor {} has invalid token {token}: {error}",
                            index + 1,
                            sensor_index
                        ))
                    })
            }
        })
        .collect::<Result<_, _>>()?;

    Ok(SourceRecord {
        source_record_id: format!("secom:{}", index + 1),
        source_locator: format!(
            "{}:row:{} + {}:row:{}",
            input.data_path.display(),
            index + 1,
            input.labels_path.display(),
            index + 1
        ),
        raw_payload: json!({
            "data_line": data_line,
            "label_line": label_line,
            "sensor_tokens": tokens,
            "label": label_parts[0],
            "timestamp": timestamp_source,
        }),
        normalized_payload: json!({
            "dataset_scope": "semiconductor_line_test_observation",
            "sensors": sensors,
            "line_test_result": if label == -1 { "pass" } else { "fail" },
            "observed_at": timestamp.to_rfc3339(),
        }),
        missing_fields,
    })
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
