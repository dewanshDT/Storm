//! Decision 161, slice 161-6: closed-app push registrations (`push_endpoints`)
//! and the delivery queue (`push_deliveries`). Both tables are 161-6's own —
//! see `db.rs` for the schema and `mark_device_revoked` for the revoke
//! cascade.
//!
//! Only the storage lives here. Policy (which devices qualify for a given
//! notice, the 15s grace, preview truncation, the actual HTTP send and its
//! retry/backoff) is `crate::push` — kept a thin, separate seam on purpose,
//! so it can be the one thing a second delivery sink (161-9, ntfy) need not
//! touch.

use anyhow::Result;
use rusqlite::{OptionalExtension, params};

use super::db::AuthDb;

pub const KINDS: &[&str] = &["webpush", "unifiedpush"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushEndpoint {
    pub id: String,
    pub device_id: String,
    pub kind: String,
    pub endpoint: String,
    pub p256dh: String,
    /// Sealed at rest (`crate::push::crypto::seal_secret`); opened only to
    /// send. Never logged — `Debug` is derived, but nothing in this crate
    /// logs a `PushEndpoint` wholesale (log the origin host instead).
    pub auth_secret_enc: Vec<u8>,
    pub created: String,
    pub last_ok: Option<String>,
    pub failures: i64,
}

const EP_COLUMNS: &str =
    "id, device_id, kind, endpoint, p256dh, auth_secret_enc, created, last_ok, failures";

fn row_to_endpoint(row: &rusqlite::Row<'_>) -> rusqlite::Result<PushEndpoint> {
    Ok(PushEndpoint {
        id: row.get(0)?,
        device_id: row.get(1)?,
        kind: row.get(2)?,
        endpoint: row.get(3)?,
        p256dh: row.get(4)?,
        auth_secret_enc: row.get(5)?,
        created: row.get(6)?,
        last_ok: row.get(7)?,
        failures: row.get(8)?,
    })
}

/// A delivery queue row: one (notice, endpoint) pair still owed a send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushDeliveryRow {
    pub id: String,
    pub endpoint_id: String,
    pub notice_id: String,
    pub collapse_key: String,
    pub urgency: String,
    pub created: String,
    pub not_before: String,
    pub attempts: i64,
    pub last_error: Option<String>,
}

const DEL_COLUMNS: &str =
    "id, endpoint_id, notice_id, collapse_key, urgency, created, not_before, attempts, last_error";

fn row_to_delivery(row: &rusqlite::Row<'_>) -> rusqlite::Result<PushDeliveryRow> {
    Ok(PushDeliveryRow {
        id: row.get(0)?,
        endpoint_id: row.get(1)?,
        notice_id: row.get(2)?,
        collapse_key: row.get(3)?,
        urgency: row.get(4)?,
        created: row.get(5)?,
        not_before: row.get(6)?,
        attempts: row.get(7)?,
        last_error: row.get(8)?,
    })
}

impl AuthDb {
    /// `POST /v1/push/endpoints`: registers or (on a re-subscribe of the
    /// same device+endpoint) replaces the keys and resets `failures`.
    pub fn upsert_push_endpoint(
        &mut self,
        device_id: &str,
        kind: &str,
        endpoint: &str,
        p256dh: &str,
        auth_secret_enc: &[u8],
        now: &str,
    ) -> Result<PushEndpoint> {
        self.conn.execute(
            "INSERT INTO push_endpoints
                 (id, device_id, kind, endpoint, p256dh, auth_secret_enc, created)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(device_id, endpoint) DO UPDATE SET
                 kind = excluded.kind,
                 p256dh = excluded.p256dh,
                 auth_secret_enc = excluded.auth_secret_enc,
                 failures = 0",
            params![
                super::identity::random_id("pep_"),
                device_id,
                kind,
                endpoint,
                p256dh,
                auth_secret_enc,
                now,
            ],
        )?;
        self.conn
            .query_row(
                &format!(
                    "SELECT {EP_COLUMNS} FROM push_endpoints WHERE device_id = ?1 AND endpoint = ?2"
                ),
                params![device_id, endpoint],
                row_to_endpoint,
            )
            .map_err(anyhow::Error::from)
    }

    pub fn push_endpoint(&self, id: &str) -> Result<Option<PushEndpoint>> {
        Ok(self
            .conn
            .query_row(
                &format!("SELECT {EP_COLUMNS} FROM push_endpoints WHERE id = ?1"),
                params![id],
                row_to_endpoint,
            )
            .optional()?)
    }

    /// `DELETE /v1/push/endpoints/{id}`: the session tier, the calling
    /// device's own only — the caller checks `device_id` matches before
    /// this is reached; `false` means no such row for that device (never a
    /// different device's row, by construction of the query).
    pub fn delete_push_endpoint(&mut self, id: &str, device_id: &str) -> Result<bool> {
        let changed = self.conn.execute(
            "DELETE FROM push_endpoints WHERE id = ?1 AND device_id = ?2",
            params![id, device_id],
        )?;
        Ok(changed > 0)
    }

    /// 404/410 from the push service: the subscription is gone. Deletes the
    /// endpoint (and, via `ON DELETE CASCADE`, its queued deliveries).
    pub fn delete_push_endpoint_unconditionally(&mut self, id: &str) -> Result<bool> {
        let changed = self
            .conn
            .execute("DELETE FROM push_endpoints WHERE id = ?1", params![id])?;
        Ok(changed > 0)
    }

    /// Every endpoint registered for a device, for the push-delivery
    /// policy's "who to reach" step. Small per device in practice (a
    /// handful at most), so no pagination.
    pub fn push_endpoints_for_device(&self, device_id: &str) -> Result<Vec<PushEndpoint>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {EP_COLUMNS} FROM push_endpoints WHERE device_id = ?1"
        ))?;
        let rows = stmt.query_map(params![device_id], row_to_endpoint)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// `GET /v1/notifications/device`: this device's own endpoint health,
    /// cheaply — one row at most per device in the common case, so the
    /// worst (oldest `last_ok`) stands in for "is push working here".
    pub fn push_endpoint_health(&self, device_id: &str) -> Result<Option<PushEndpointHealth>> {
        Ok(self
            .conn
            .query_row(
                "SELECT last_ok, failures FROM push_endpoints
                 WHERE device_id = ?1
                 ORDER BY failures DESC, last_ok ASC
                 LIMIT 1",
                params![device_id],
                |r| {
                    Ok(PushEndpointHealth {
                        last_ok: r.get(0)?,
                        failures: r.get(1)?,
                    })
                },
            )
            .optional()?)
    }

    pub fn record_push_ok(&mut self, endpoint_id: &str, now: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE push_endpoints SET last_ok = ?2, failures = 0 WHERE id = ?1",
            params![endpoint_id, now],
        )?;
        Ok(())
    }

    pub fn record_push_failure(&mut self, endpoint_id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE push_endpoints SET failures = failures + 1 WHERE id = ?1",
            params![endpoint_id],
        )?;
        Ok(())
    }

    // ---- delivery queue ---------------------------------------------------

    /// Enqueues one (notice, endpoint) delivery, ready immediately
    /// (`not_before = now`) unless `delay` pushes it out (the 15s grace is
    /// applied by the caller before this is reached — this is storage, not
    /// policy).
    pub fn enqueue_push_delivery(
        &mut self,
        endpoint_id: &str,
        notice_id: &str,
        collapse_key: &str,
        urgency: &str,
        now: &str,
        not_before: &str,
    ) -> Result<PushDeliveryRow> {
        let id = super::identity::random_id("pdl_");
        self.conn.execute(
            "INSERT INTO push_deliveries
                 (id, endpoint_id, notice_id, collapse_key, urgency, created, not_before)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                id,
                endpoint_id,
                notice_id,
                collapse_key,
                urgency,
                now,
                not_before
            ],
        )?;
        self.conn
            .query_row(
                &format!("SELECT {DEL_COLUMNS} FROM push_deliveries WHERE id = ?1"),
                params![id],
                row_to_delivery,
            )
            .map_err(anyhow::Error::from)
    }

    /// Whether a notice already has at least one queued delivery — the
    /// push-delivery policy's one-shot-per-notice guard, so a restart (or a
    /// second poll before the first finishes) never double-enqueues.
    pub fn push_delivery_exists_for_notice(&self, notice_id: &str) -> Result<bool> {
        Ok(self
            .conn
            .query_row(
                "SELECT 1 FROM push_deliveries WHERE notice_id = ?1 LIMIT 1",
                params![notice_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    /// Rows due now, oldest first, for the sender worker. `limit` bounds one
    /// pass so a large backlog cannot starve other `auth.db` callers of the
    /// lock for long.
    pub fn due_push_deliveries(&self, now: &str, limit: i64) -> Result<Vec<PushDeliveryRow>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {DEL_COLUMNS} FROM push_deliveries
             WHERE not_before <= ?1
             ORDER BY created ASC
             LIMIT ?2"
        ))?;
        let rows = stmt.query_map(params![now, limit], row_to_delivery)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// A successful send (push service accepted it): the row's done.
    pub fn delete_push_delivery(&mut self, id: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM push_deliveries WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// 429/5xx: try again later, honouring the backoff the caller computed
    /// (`Retry-After` when the push service sent one, else exponential).
    pub fn reschedule_push_delivery(
        &mut self,
        id: &str,
        not_before: &str,
        error: &str,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE push_deliveries
             SET attempts = attempts + 1, not_before = ?2, last_error = ?3
             WHERE id = ?1",
            params![id, not_before, error],
        )?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct PushEndpointHealth {
    pub last_ok: Option<String>,
    pub failures: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: &str = "2026-11-21T00:00:00Z";

    fn db() -> AuthDb {
        AuthDb::open_in_memory().unwrap()
    }

    fn device(db: &mut AuthDb) -> String {
        super::super::devices::create_synthetic(db, "test-device", NOW)
            .unwrap()
            .id
    }

    #[test]
    fn upsert_creates_then_replaces_keys_and_resets_failures() {
        let mut db = db();
        let device_id = device(&mut db);
        let ep = db
            .upsert_push_endpoint(
                &device_id,
                "webpush",
                "https://push.example.net/sub/1",
                "p256dh-a",
                b"secret-a",
                NOW,
            )
            .unwrap();
        db.record_push_failure(&ep.id).unwrap();
        db.record_push_failure(&ep.id).unwrap();

        let again = db
            .upsert_push_endpoint(
                &device_id,
                "webpush",
                "https://push.example.net/sub/1",
                "p256dh-b",
                b"secret-b",
                "2026-11-21T01:00:00Z",
            )
            .unwrap();
        assert_eq!(again.id, ep.id, "same device+endpoint upserts in place");
        assert_eq!(again.p256dh, "p256dh-b");
        assert_eq!(again.auth_secret_enc, b"secret-b");
        assert_eq!(again.failures, 0, "a re-subscribe resets failure count");
    }

    #[test]
    fn delete_is_scoped_to_the_calling_device() {
        let mut db = db();
        let owner = device(&mut db);
        let other = device(&mut db);
        let ep = db
            .upsert_push_endpoint(
                &owner,
                "webpush",
                "https://push.example.net/sub/2",
                "k",
                b"s",
                NOW,
            )
            .unwrap();

        assert!(!db.delete_push_endpoint(&ep.id, &other).unwrap());
        assert!(db.push_endpoint(&ep.id).unwrap().is_some());
        assert!(db.delete_push_endpoint(&ep.id, &owner).unwrap());
        assert!(db.push_endpoint(&ep.id).unwrap().is_none());
    }

    #[test]
    fn revoking_a_device_deletes_its_endpoints_and_their_queued_deliveries() {
        let mut db = db();
        let device_id = device(&mut db);
        let ep = db
            .upsert_push_endpoint(
                &device_id,
                "webpush",
                "https://push.example.net/sub/3",
                "k",
                b"s",
                NOW,
            )
            .unwrap();
        db.enqueue_push_delivery(&ep.id, "ntf_x", "key", "normal", NOW, NOW)
            .unwrap();

        super::super::devices::revoke(&mut db, &device_id, "test", NOW).unwrap();

        assert!(db.push_endpoint(&ep.id).unwrap().is_none());
        assert!(db.due_push_deliveries(NOW, 10).unwrap().is_empty());
    }

    #[test]
    fn deleting_an_endpoint_cascades_its_deliveries() {
        let mut db = db();
        let device_id = device(&mut db);
        let ep = db
            .upsert_push_endpoint(
                &device_id,
                "webpush",
                "https://push.example.net/sub/4",
                "k",
                b"s",
                NOW,
            )
            .unwrap();
        db.enqueue_push_delivery(&ep.id, "ntf_y", "key", "high", NOW, NOW)
            .unwrap();

        db.delete_push_endpoint_unconditionally(&ep.id).unwrap();

        assert!(db.due_push_deliveries(NOW, 10).unwrap().is_empty());
    }

    #[test]
    fn one_shot_guard_sees_an_existing_queued_delivery() {
        let mut db = db();
        let device_id = device(&mut db);
        let ep = db
            .upsert_push_endpoint(
                &device_id,
                "webpush",
                "https://push.example.net/sub/5",
                "k",
                b"s",
                NOW,
            )
            .unwrap();
        assert!(!db.push_delivery_exists_for_notice("ntf_z").unwrap());
        db.enqueue_push_delivery(&ep.id, "ntf_z", "key", "normal", NOW, NOW)
            .unwrap();
        assert!(db.push_delivery_exists_for_notice("ntf_z").unwrap());
    }

    #[test]
    fn due_deliveries_respect_not_before() {
        let mut db = db();
        let device_id = device(&mut db);
        let ep = db
            .upsert_push_endpoint(
                &device_id,
                "webpush",
                "https://push.example.net/sub/6",
                "k",
                b"s",
                NOW,
            )
            .unwrap();
        db.enqueue_push_delivery(
            &ep.id,
            "ntf_later",
            "key",
            "low",
            NOW,
            "2026-11-22T00:00:00Z",
        )
        .unwrap();
        assert!(db.due_push_deliveries(NOW, 10).unwrap().is_empty());
        assert_eq!(
            db.due_push_deliveries("2026-11-22T00:00:00Z", 10)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn reschedule_bumps_attempts_and_records_the_error() {
        let mut db = db();
        let device_id = device(&mut db);
        let ep = db
            .upsert_push_endpoint(
                &device_id,
                "webpush",
                "https://push.example.net/sub/7",
                "k",
                b"s",
                NOW,
            )
            .unwrap();
        let row = db
            .enqueue_push_delivery(&ep.id, "ntf_retry", "key", "normal", NOW, NOW)
            .unwrap();
        db.reschedule_push_delivery(&row.id, "2026-11-21T00:01:00Z", "429")
            .unwrap();
        let due = db.due_push_deliveries("2026-11-21T00:01:00Z", 10).unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].attempts, 1);
        assert_eq!(due[0].last_error.as_deref(), Some("429"));
    }
}
