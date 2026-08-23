use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{BenchmarkArtifacts, GradeOutcome, GradeRecord};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GradingExport {
    pub sheet: PathBuf,
    pub key: PathBuf,
    pub rows: usize,
}

#[derive(Serialize)]
struct PublicRow<'a> {
    opaque_id: &'a str,
    prompt: &'a str,
    rubric: &'a str,
    answer: &'a str,
    automated_test_passed: Option<bool>,
    outcome: &'a str,
    critical_regression: &'a str,
    reason: &'a str,
}

#[derive(Serialize)]
struct KeyRow<'a> {
    opaque_id: &'a str,
    sample_id: &'a str,
}

#[derive(Deserialize)]
struct ImportedRow {
    opaque_id: String,
    outcome: String,
    critical_regression: String,
    reason: String,
}

pub fn export_grading_bundle(
    artifacts: &BenchmarkArtifacts,
    task_text: &BTreeMap<String, (String, String)>,
    seed: u64,
) -> Result<GradingExport> {
    let sheet = artifacts.run_dir().join("grading-sheet.csv");
    let key = artifacts.run_dir().join("grading-key.csv");
    ensure!(
        !sheet.exists() && !key.exists(),
        "grading bundle already exists"
    );
    let samples = artifacts.samples()?;
    let mut sheet_writer = csv::Writer::from_path(&sheet)?;
    let mut key_writer = csv::Writer::from_path(&key)?;
    for sample in &samples {
        let opaque_id = opaque_id(seed, &sample.sample.sample_id);
        let (prompt, rubric) = task_text
            .get(&sample.sample.task_id)
            .with_context(|| format!("missing grading text for {}", sample.sample.task_id))?;
        sheet_writer.serialize(PublicRow {
            opaque_id: &opaque_id,
            prompt,
            rubric,
            answer: &sample.answer,
            automated_test_passed: sample.automated_test_passed,
            outcome: "",
            critical_regression: "",
            reason: "",
        })?;
        key_writer.serialize(KeyRow {
            opaque_id: &opaque_id,
            sample_id: &sample.sample.sample_id,
        })?;
    }
    sheet_writer.flush()?;
    key_writer.flush()?;
    Ok(GradingExport {
        sheet,
        key,
        rows: samples.len(),
    })
}

pub fn import_grades(
    artifacts: &BenchmarkArtifacts,
    csv_path: &std::path::Path,
    grader: &str,
    grader_version: &str,
    graded_at: &str,
) -> Result<usize> {
    ensure!(!grader.trim().is_empty(), "grader identity is required");
    let mut imported = 0usize;
    for row in csv::Reader::from_path(csv_path)?.deserialize::<ImportedRow>() {
        let row = row?;
        let outcome = match row.outcome.trim().to_ascii_lowercase().as_str() {
            "pass" => GradeOutcome::Pass,
            "partial" => GradeOutcome::Partial,
            "fail" => GradeOutcome::Fail,
            other => anyhow::bail!("invalid grade outcome {other:?}"),
        };
        let critical_regression = match row.critical_regression.trim().to_ascii_lowercase().as_str()
        {
            "true" | "yes" | "1" => true,
            "false" | "no" | "0" => false,
            other => anyhow::bail!("invalid critical_regression {other:?}"),
        };
        artifacts.append_grade(&GradeRecord {
            schema_version: 2,
            opaque_id: row.opaque_id,
            outcome,
            critical_regression,
            reason: row.reason,
            grader: grader.to_owned(),
            grader_version: grader_version.to_owned(),
            graded_at: graded_at.to_owned(),
        })?;
        imported += 1;
    }
    Ok(imported)
}

fn opaque_id(seed: u64, sample_id: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(seed.to_le_bytes());
    digest.update(sample_id.as_bytes());
    hex::encode(&digest.finalize()[..12])
}
