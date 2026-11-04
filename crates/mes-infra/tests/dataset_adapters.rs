use std::fs;

use mes_application::{DatasetAdapter, EvaluationTrack};
use mes_infra::datasets::{Ai4iAdapter, Ai4iInput, SecomAdapter, SecomInput};
use tempfile::tempdir;

#[test]
fn ai4i_preserves_raw_fields_and_exposes_missingness() {
    let directory = tempdir().expect("temporary directory must be created");
    let path = directory.path().join("ai4i2020.csv");
    fs::write(
        &path,
        concat!(
            "UDI,Product ID,Type,Air temperature [K],Process temperature [K],",
            "Rotational speed [rpm],Torque [Nm],Tool wear [min],Machine failure,",
            "TWF,HDF,PWF,OSF,RNF\n",
            "1,M14860,M,298.1,308.6,1551,,0,0,0,0,0,0,0\n"
        ),
    )
    .expect("fixture must be written");

    let imported = Ai4iAdapter
        .import(Ai4iInput {
            csv_path: path,
            version: "test-fixture".to_owned(),
        })
        .expect("fixture must import");

    assert_eq!(imported.manifest.track, EvaluationTrack::PublishedSynthetic);
    assert_eq!(imported.records.len(), 1);
    let record = &imported.records[0];
    assert_eq!(record.raw_payload["Product ID"], "M14860");
    assert_eq!(
        record.normalized_payload["torque_nm"],
        serde_json::Value::Null
    );
    assert_eq!(record.missing_fields, vec!["torque_nm"]);
    assert_eq!(imported.manifest.sha256.len(), 64);
}

#[test]
fn secom_keeps_nan_missing_and_does_not_impute() {
    let directory = tempdir().expect("temporary directory must be created");
    let data_path = directory.path().join("secom.data");
    let labels_path = directory.path().join("secom_labels.data");
    let mut sensor_tokens = vec!["1.0"; 590];
    sensor_tokens[1] = "NaN";
    fs::write(&data_path, format!("{}\n", sensor_tokens.join(" ")))
        .expect("fixture must be written");
    fs::write(&labels_path, "-1 19/07/2008 11:55:00\n").expect("fixture must be written");

    let imported = SecomAdapter
        .import(SecomInput {
            data_path,
            labels_path,
            version: "test-fixture".to_owned(),
        })
        .expect("fixture must import");

    assert_eq!(imported.manifest.track, EvaluationTrack::RealWorld);
    assert_eq!(imported.records.len(), 1);
    let record = &imported.records[0];
    assert_eq!(record.raw_payload["sensor_tokens"][1], "NaN");
    assert_eq!(
        record.normalized_payload["sensors"][1],
        serde_json::Value::Null
    );
    assert_eq!(record.missing_fields, vec!["sensor_1"]);
    assert_eq!(record.normalized_payload["line_test_result"], "pass");
}
