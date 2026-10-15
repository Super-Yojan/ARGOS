use crate::types::Operation;
use sqlx::{
    Row, SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
};
use std::{str::FromStr, time::Duration};

#[derive(Clone)]
pub struct Store {
    pub pool: SqlitePool,
}
impl Store {
    pub async fn open(url: &str) -> Result<Self, sqlx::Error> {
        let options = SqliteConnectOptions::from_str(url)?
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .busy_timeout(Duration::from_secs(5));
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(options)
            .await?;
        // Versioned additive schema. Never store live telemetry as authoritative state.
        sqlx::raw_sql(include_str!("schema.sql"))
            .execute(&pool)
            .await?;
        Ok(Self { pool })
    }
    pub async fn record(&self, op: &Operation) -> Result<(), String> {
        let payload = serde_json::to_string(op).map_err(|e| e.to_string())?;
        let mut tx = self.pool.begin().await.map_err(|e| e.to_string())?;
        sqlx::query("INSERT INTO operations(id,request_id,payload) VALUES(?,?,?) ON CONFLICT(id) DO UPDATE SET payload=excluded.payload")
            .bind(&op.id).bind(&op.request_id).bind(&payload).execute(&mut *tx).await.map_err(|e|e.to_string())?;
        sqlx::query("INSERT INTO audit(operation_id,payload) VALUES(?,?)")
            .bind(&op.id)
            .bind(payload)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
        tx.commit().await.map_err(|e| e.to_string())
    }
    pub async fn recover(&self) -> Result<Vec<Operation>, String> {
        let rows = sqlx::query("SELECT payload FROM operations ORDER BY rowid")
            .fetch_all(&self.pool)
            .await
            .map_err(|e| e.to_string())?;
        let mut result = Vec::new();
        for r in rows {
            let mut op: Operation =
                serde_json::from_str(r.get("payload")).map_err(|e| e.to_string())?;
            if !op.terminal() {
                op.phase = "interrupted".into();
                op.message =
                    "Service restarted; outcome may be uncertain. Command not replayed.".into();
                self.record(&op).await?;
            }
            result.push(op);
        }
        Ok(result)
    }
}
