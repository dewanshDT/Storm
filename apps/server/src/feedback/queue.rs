//! The durable send queue (decision 162b/162-3): a report is in this table
//! before `POST /v1/feedback` answers `202`, and stays until the forwarder
//! delivers it, drops it permanently, or gives up after 7 days.
//!
//! **`body` is the exact bytes forwarded to the intake Worker.** Never
//! logged — a row holds a user's report text, their email and the
//! diagnostics bundle, and decision 162c's whole point is that none of that
//! leaves this server except to the intake it was sent to.

use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{Connection, params};

pub struct Queue {
    conn: tokio::sync::Mutex<Connection>,
}

#[derive(Debug, Clone)]
pub struct QueueRow {
    pub report_id: String,
    pub body: String,
    pub attempts: i64,
    pub created_at: String,
}

impl Queue {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        let conn = Connection::open(path)
            .with_context(|| format!("opening the feedback queue {}", path.display()))?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             CREATE TABLE IF NOT EXISTS feedback_queue (
                 report_id    TEXT PRIMARY KEY,
                 body         TEXT NOT NULL,
                 attempts     INTEGER NOT NULL DEFAULT 0,
                 next_attempt TEXT NOT NULL,
                 created_at   TEXT NOT NULL,
                 last_error   TEXT
             );
             CREATE INDEX IF NOT EXISTS feedback_queue_by_next
                 ON feedback_queue(next_attempt);",
        )?;
        Ok(Self {
            conn: tokio::sync::Mutex::new(conn),
        })
    }

    /// Persists a send before its `202` — the durability guarantee a client
    /// reconnecting later relies on.
    pub async fn enqueue(&self, report_id: &str, body: &str, now: &str) -> Result<()> {
        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT INTO feedback_queue (report_id, body, attempts, next_attempt, created_at)
             VALUES (?1, ?2, 0, ?3, ?3)",
            params![report_id, body, now],
        )?;
        Ok(())
    }

    /// Rows ready to try now, oldest first, at most `limit`.
    pub async fn due(&self, now: &str, limit: usize) -> Result<Vec<QueueRow>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare(
            "SELECT report_id, body, attempts, created_at FROM feedback_queue
             WHERE next_attempt <= ?1 ORDER BY next_attempt ASC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![now, limit as i64], |r| {
            Ok(QueueRow {
                report_id: r.get(0)?,
                body: r.get(1)?,
                attempts: r.get(2)?,
                created_at: r.get(3)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("reading due feedback sends")
    }

    /// A send failed and will be tried again: one more attempt, at
    /// `next_attempt`, with `reason` for the operator (never the body).
    pub async fn reschedule(
        &self,
        report_id: &str,
        attempts: i64,
        next_attempt: &str,
        reason: &str,
    ) -> Result<()> {
        let conn = self.conn.lock().await;
        conn.execute(
            "UPDATE feedback_queue SET attempts = ?1, next_attempt = ?2, last_error = ?3
             WHERE report_id = ?4",
            params![attempts, next_attempt, reason, report_id],
        )?;
        Ok(())
    }

    /// Delivered, dropped permanently, or given up on — removed either way.
    pub async fn remove(&self, report_id: &str) -> Result<()> {
        let conn = self.conn.lock().await;
        conn.execute(
            "DELETE FROM feedback_queue WHERE report_id = ?1",
            params![report_id],
        )?;
        Ok(())
    }

    #[cfg(test)]
    pub async fn len(&self) -> Result<i64> {
        let conn = self.conn.lock().await;
        conn.query_row("SELECT COUNT(*) FROM feedback_queue", [], |r| r.get(0))
            .context("counting the feedback queue")
    }

    /// Backdates a row's `created_at`, for the "gives up after 7 days" test
    /// — real time never has to pass for it.
    #[cfg(test)]
    pub async fn backdate_for_tests(&self, report_id: &str, created_at: &str) -> Result<()> {
        let conn = self.conn.lock().await;
        conn.execute(
            "UPDATE feedback_queue SET created_at = ?1 WHERE report_id = ?2",
            params![created_at, report_id],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn enqueued_rows_are_due_at_once_and_gone_once_removed() {
        let dir = tempdir::TempDir::new("storm-feedback-queue").unwrap();
        let q = Queue::open(&dir.path().join("feedback.db")).unwrap();
        q.enqueue("rep_1", "{}", "2026-11-19T00:00:00Z")
            .await
            .unwrap();
        let due = q.due("2026-11-19T00:00:00Z", 10).await.unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].report_id, "rep_1");
        assert_eq!(due[0].attempts, 0);

        q.remove("rep_1").await.unwrap();
        assert_eq!(q.due("2026-11-19T00:00:00Z", 10).await.unwrap().len(), 0);
    }

    #[tokio::test]
    async fn a_rescheduled_row_is_not_due_before_its_time() {
        let dir = tempdir::TempDir::new("storm-feedback-queue-resched").unwrap();
        let q = Queue::open(&dir.path().join("feedback.db")).unwrap();
        q.enqueue("rep_1", "{}", "2026-11-19T00:00:00Z")
            .await
            .unwrap();
        q.reschedule("rep_1", 1, "2026-11-19T01:00:00Z", "network")
            .await
            .unwrap();
        assert_eq!(q.due("2026-11-19T00:30:00Z", 10).await.unwrap().len(), 0);
        assert_eq!(q.due("2026-11-19T01:00:00Z", 10).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_queue_reopened_keeps_its_pending_rows() {
        let dir = tempdir::TempDir::new("storm-feedback-queue-reopen").unwrap();
        let path = dir.path().join("feedback.db");
        {
            let q = Queue::open(&path).unwrap();
            q.enqueue("rep_1", "{\"text\":\"hello\"}", "2026-11-19T00:00:00Z")
                .await
                .unwrap();
        }
        let q = Queue::open(&path).unwrap();
        let due = q.due("2026-11-19T00:00:00Z", 10).await.unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].body, "{\"text\":\"hello\"}");
    }
}
