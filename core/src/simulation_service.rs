use crate::errors::AppError;
use reqwest::Client;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const DEFAULT_ZSCORE_THRESHOLD: f64 = 2.0;
const DEFAULT_SHIFT_THRESHOLD: f64 = 0.10;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulationMetric {
    pub contract: String,
    pub method: String,
    pub code_hash: String,
    pub cpu_instructions: u64,
    pub ram_bytes: u64,
    pub ledger_footprint: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct HistoricalAverages {
    pub samples: usize,
    pub avg_cpu_instructions: f64,
    pub avg_ram_bytes: f64,
    pub avg_ledger_footprint: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct DriftDetail {
    pub metric: String,
    pub value: u64,
    pub average: f64,
    pub percent_shift: f64,
    pub z_score: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AnalysisResult {
    pub has_historical_baseline: bool,
    pub historical: Option<HistoricalAverages>,
    pub outliers: Vec<DriftDetail>,
    pub alert_triggered: bool,
}

#[derive(Debug, Clone)]
pub struct SimulationService {
    db_path: PathBuf,
    shift_threshold: f64,
    zscore_threshold: f64,
    webhook_url: Option<String>,
}

impl SimulationService {
    pub fn new(db_path: impl AsRef<Path>, webhook_url: Option<String>) -> Result<Self, AppError> {
        let service = Self {
            db_path: db_path.as_ref().to_path_buf(),
            shift_threshold: DEFAULT_SHIFT_THRESHOLD,
            zscore_threshold: DEFAULT_ZSCORE_THRESHOLD,
            webhook_url,
        };
        service.ensure_schema()?;
        Ok(service)
    }

    fn connect(&self) -> Result<Connection, AppError> {
        Connection::open(&self.db_path)
            .map_err(|e| AppError::Internal(format!("Failed to open metrics database: {e}")))
    }

    fn ensure_schema(&self) -> Result<(), AppError> {
        let conn = self.connect()?;
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS simulation_metrics (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                contract TEXT NOT NULL,
                method TEXT NOT NULL,
                code_hash TEXT NOT NULL,
                cpu_instructions INTEGER NOT NULL,
                ram_bytes INTEGER NOT NULL,
                ledger_footprint INTEGER NOT NULL,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );

            CREATE INDEX IF NOT EXISTS idx_simulation_lookup
                ON simulation_metrics(contract, method, code_hash, created_at);

            CREATE TABLE IF NOT EXISTS simulation_alerts (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                simulation_metric_id INTEGER NOT NULL,
                contract TEXT NOT NULL,
                method TEXT NOT NULL,
                code_hash TEXT NOT NULL,
                details_json TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY(simulation_metric_id) REFERENCES simulation_metrics(id)
            );
            ",
        )
        .map_err(|e| AppError::Internal(format!("Failed to create metrics schema: {e}")))?;
        Ok(())
    }

    pub async fn record_and_analyze(
        &self,
        metric: SimulationMetric,
    ) -> Result<AnalysisResult, AppError> {
        let baseline =
            self.load_historical_stats(&metric.contract, &metric.method, &metric.code_hash)?;
        let outliers = if let Some((ref historical, ref rows)) = baseline {
            self.detect_outliers(&metric, historical, rows)
        } else {
            Vec::new()
        };

        let metric_id = self.insert_metric(&metric)?;
        let alert_triggered = !outliers.is_empty();

        if alert_triggered {
            self.store_alert(metric_id, &metric, &outliers)?;
            self.emit_alert(&metric, &outliers).await;
        }

        Ok(AnalysisResult {
            has_historical_baseline: baseline.is_some(),
            historical: baseline.as_ref().map(|(historical, _)| historical.clone()),
            outliers,
            alert_triggered,
        })
    }

    fn insert_metric(&self, metric: &SimulationMetric) -> Result<i64, AppError> {
        let conn = self.connect()?;
        conn.execute(
            "
            INSERT INTO simulation_metrics (contract, method, code_hash, cpu_instructions, ram_bytes, ledger_footprint)
            VALUES (?, ?, ?, ?, ?, ?)
            ",
            params![
                metric.contract,
                metric.method,
                metric.code_hash,
                metric.cpu_instructions as i64,
                metric.ram_bytes as i64,
                metric.ledger_footprint as i64
            ],
        )
        .map_err(|e| AppError::Internal(format!("Failed to insert simulation metric: {e}")))?;
        Ok(conn.last_insert_rowid())
    }

    fn store_alert(
        &self,
        simulation_metric_id: i64,
        metric: &SimulationMetric,
        outliers: &[DriftDetail],
    ) -> Result<(), AppError> {
        let details_json = serde_json::to_string(outliers)
            .map_err(|e| AppError::Internal(format!("Failed to serialize alert details: {e}")))?;
        let conn = self.connect()?;
        conn.execute(
            "
            INSERT INTO simulation_alerts (simulation_metric_id, contract, method, code_hash, details_json)
            VALUES (?, ?, ?, ?, ?)
            ",
            params![
                simulation_metric_id,
                metric.contract,
                metric.method,
                metric.code_hash,
                details_json
            ],
        )
        .map_err(|e| AppError::Internal(format!("Failed to store simulation alert: {e}")))?;
        Ok(())
    }

    fn load_historical_stats(
        &self,
        contract: &str,
        method: &str,
        code_hash: &str,
    ) -> Result<Option<(HistoricalAverages, Vec<(u64, u64, u64)>)>, AppError> {
        let conn = self.connect()?;
        let mut stmt = conn
            .prepare(
                "
                SELECT cpu_instructions, ram_bytes, ledger_footprint
                FROM simulation_metrics
                WHERE contract = ?1 AND method = ?2 AND code_hash = ?3
                ",
            )
            .map_err(|e| AppError::Internal(format!("Failed to prepare history query: {e}")))?;

        let rows = stmt
            .query_map(params![contract, method, code_hash], |row| {
                Ok((
                    row.get::<_, i64>(0)? as u64,
                    row.get::<_, i64>(1)? as u64,
                    row.get::<_, i64>(2)? as u64,
                ))
            })
            .map_err(|e| AppError::Internal(format!("Failed to query historical metrics: {e}")))?;

        let data: Result<Vec<_>, _> = rows.collect();
        let data = data
            .map_err(|e| AppError::Internal(format!("Failed to read historical metrics: {e}")))?;

        if data.is_empty() {
            return Ok(None);
        }

        let n = data.len() as f64;
        let cpu_sum: f64 = data.iter().map(|r| r.0 as f64).sum();
        let ram_sum: f64 = data.iter().map(|r| r.1 as f64).sum();
        let ledger_sum: f64 = data.iter().map(|r| r.2 as f64).sum();

        Ok(Some((
            HistoricalAverages {
                samples: data.len(),
                avg_cpu_instructions: cpu_sum / n,
                avg_ram_bytes: ram_sum / n,
                avg_ledger_footprint: ledger_sum / n,
            },
            data,
        )))
    }

    fn detect_outliers(
        &self,
        current: &SimulationMetric,
        historical: &HistoricalAverages,
        rows: &[(u64, u64, u64)],
    ) -> Vec<DriftDetail> {
        let cpu_values: Vec<f64> = rows.iter().map(|r| r.0 as f64).collect();
        let ram_values: Vec<f64> = rows.iter().map(|r| r.1 as f64).collect();
        let ledger_values: Vec<f64> = rows.iter().map(|r| r.2 as f64).collect();

        let cpu_z = z_score(current.cpu_instructions as f64, &cpu_values);
        let ram_z = z_score(current.ram_bytes as f64, &ram_values);
        let ledger_z = z_score(current.ledger_footprint as f64, &ledger_values);

        let mut outliers = Vec::new();

        if let Some(detail) = assess_metric_shift(
            "cpu_instructions",
            current.cpu_instructions,
            historical.avg_cpu_instructions,
            cpu_z,
            self.shift_threshold,
            self.zscore_threshold,
        ) {
            outliers.push(detail);
        }

        if let Some(detail) = assess_metric_shift(
            "ram_bytes",
            current.ram_bytes,
            historical.avg_ram_bytes,
            ram_z,
            self.shift_threshold,
            self.zscore_threshold,
        ) {
            outliers.push(detail);
        }

        if let Some(detail) = assess_metric_shift(
            "ledger_footprint",
            current.ledger_footprint,
            historical.avg_ledger_footprint,
            ledger_z,
            self.shift_threshold,
            self.zscore_threshold,
        ) {
            outliers.push(detail);
        }

        outliers
    }

    async fn emit_alert(&self, metric: &SimulationMetric, outliers: &[DriftDetail]) {
        eprintln!(
            "[ALERT] Resource shift detected for {}/{} on unchanged code hash {}: {:?}",
            metric.contract, metric.method, metric.code_hash, outliers
        );

        let Some(url) = &self.webhook_url else {
            return;
        };

        let payload = serde_json::json!({
            "event": "simulation_resource_shift",
            "contract": metric.contract,
            "method": metric.method,
            "code_hash": metric.code_hash,
            "cpu_instructions": metric.cpu_instructions,
            "ram_bytes": metric.ram_bytes,
            "ledger_footprint": metric.ledger_footprint,
            "outliers": outliers,
        });

        let client = Client::new();
        if let Err(err) = client.post(url).json(&payload).send().await {
            eprintln!("[ALERT] Failed to send webhook notification: {err}");
        }
    }
}

fn assess_metric_shift(
    metric: &str,
    value: u64,
    average: f64,
    z_score: Option<f64>,
    shift_threshold: f64,
    z_threshold: f64,
) -> Option<DriftDetail> {
    if average <= f64::EPSILON {
        return None;
    }

    let percent_shift = ((value as f64 - average) / average).abs();
    let shifted = percent_shift > shift_threshold;

    let zscore_outlier = z_score.map(|z| z.abs() > z_threshold).unwrap_or(false);

    if shifted || zscore_outlier {
        return Some(DriftDetail {
            metric: metric.to_string(),
            value,
            average,
            percent_shift,
            z_score,
        });
    }

    None
}

fn z_score(current: f64, values: &[f64]) -> Option<f64> {
    if values.len() < 2 {
        return None;
    }

    let mean = values.iter().sum::<f64>() / values.len() as f64;
    let variance = values
        .iter()
        .map(|v| {
            let diff = v - mean;
            diff * diff
        })
        .sum::<f64>()
        / values.len() as f64;

    let std_dev = variance.sqrt();
    if std_dev <= f64::EPSILON {
        return None;
    }

    Some((current - mean) / std_dev)
}
