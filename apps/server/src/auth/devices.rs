#![allow(dead_code)] // Used by tests; REST/middleware callers land in a later slice.
//! Client devices — an app installation, never a person.
//!
//! A device belongs to no user. A [`super::sessions::Session`] binds
//! `(user, device)`, which is what lets two people share an iPad, lets "log
//! out" mean one user on one device, and lets "revoke this device" mean all of
//! it at once.
//!
//! **This module does not implement pairing.** The real enrolment path —
//! `pairing_sessions`, the QR, the console nonce, `POST /v1/pair` — is a later
//! slice and nothing here touches it. What exists here is the *synthetic*
//! device the protocol names for operator-minted credentials:
//!
//! > `storm-server token --user dewansh --name "claude-code"`
//! > → creates a synthetic client_device, logs in, prints one access token
//!
//! Synthetic or paired, a device is one row in one table and one entry in the
//! device list, so revoking an agent's access is the same gesture as revoking a
//! lost phone's. That was the point of not giving agents their own entity.

use anyhow::{Result, bail};
use rusqlite::{OptionalExtension, params};

use super::db::AuthDb;
use super::identity::random_id;
use super::token;

pub const EVENT_DEVICE_CREATED: &str = "device_created";
pub const EVENT_DEVICE_REVOKED: &str = "device_revoked";

/// Recorded on devices this server minted for itself rather than paired.
pub const PLATFORM_SYNTHETIC: &str = "synthetic";

/// A device row. **No `secret_hash`**, for the same reason [`super::users::User`]
/// has no password hash: a struct that carries the secret ends up in a log line.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub platform: Option<String>,
    pub client_version: Option<String>,
    pub paired: String,
    pub last_seen: Option<String>,
    pub revoked: Option<String>,
}

impl Device {
    pub fn is_revoked(&self) -> bool {
        self.revoked.is_some()
    }
}

impl AuthDb {
    pub fn find_device(&self, id: &str) -> Result<Option<Device>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, name, platform, client_version, paired, last_seen, revoked
                 FROM client_devices WHERE id = ?1",
                params![id],
                row_to_device,
            )
            .optional()?)
    }

    pub fn list_devices(&self) -> Result<Vec<Device>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, platform, client_version, paired, last_seen, revoked
             FROM client_devices ORDER BY paired",
        )?;
        let rows = stmt.query_map([], row_to_device)?;
        let mut devices = Vec::new();
        for row in rows {
            devices.push(row?);
        }
        Ok(devices)
    }

    /// Verifies a device secret against the stored hash.
    ///
    /// Returns `Ok(None)` for missing or revoked devices — the caller decides
    /// which error to return; the middleware needs the device for the extension
    /// before it can say "revoked".
    pub fn verify_device_secret(&self, id: &str, secret: &str) -> Result<Option<Device>> {
        let stored_hash: Option<Vec<u8>> = self
            .conn
            .query_row(
                "SELECT secret_hash FROM client_devices WHERE id = ?1",
                params![id],
                |r| r.get(0),
            )
            .optional()?;
        let Some(hash) = stored_hash else {
            return Ok(None);
        };
        if hash != super::token::hash(secret) {
            return Ok(None);
        }
        self.find_device(id)
    }

    fn insert_device(&mut self, device: &Device, secret_hash: &[u8]) -> Result<()> {
        self.conn.execute(
            "INSERT INTO client_devices
                 (id, name, platform, client_version, secret_hash, paired)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                device.id,
                device.name,
                device.platform,
                device.client_version,
                secret_hash,
                device.paired,
            ],
        )?;
        Ok(())
    }

    /// Inserts a paired device, recording which pairing session created it.
    fn insert_device_paired(
        &mut self,
        device: &Device,
        secret_hash: &[u8],
        paired_via: &str,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO client_devices
                 (id, name, platform, client_version, secret_hash, paired, paired_via)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                device.id,
                device.name,
                device.platform,
                device.client_version,
                secret_hash,
                device.paired,
                paired_via,
            ],
        )?;
        Ok(())
    }

    fn mark_device_revoked(&mut self, id: &str, reason: &str, now: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE client_devices SET revoked = ?2, revoked_reason = ?3 WHERE id = ?1",
            params![id, now, reason],
        )?;
        Ok(())
    }
}

fn row_to_device(row: &rusqlite::Row<'_>) -> rusqlite::Result<Device> {
    Ok(Device {
        id: row.get(0)?,
        name: row.get(1)?,
        platform: row.get(2)?,
        client_version: row.get(3)?,
        paired: row.get(4)?,
        last_seen: row.get(5)?,
        revoked: row.get(6)?,
    })
}

/// Registers a device this server minted for itself.
///
/// The device secret is generated to keep the row well-formed and is then
/// **discarded on purpose**. A synthetic device's credential is the session
/// token the operator is handed; nothing will ever present this secret, and
/// printing a second secret beside the token would only invite pasting the
/// wrong one. Pairing mints — and hands over — its own.
pub fn create_synthetic(db: &mut AuthDb, name: &str, now: &str) -> Result<Device> {
    let name = name.trim();
    if name.is_empty() {
        bail!("a device needs a name — it is how you recognise what to revoke");
    }

    let device = Device {
        id: random_id("dev_"),
        name: name.to_string(),
        platform: Some(PLATFORM_SYNTHETIC.to_string()),
        client_version: Some(env!("CARGO_PKG_VERSION").to_string()),
        paired: now.to_string(),
        last_seen: None,
        revoked: None,
    };
    let secret = token::mint(token::DEVICE_SECRET_PREFIX);
    db.insert_device(&device, &token::hash(&secret))?;
    db.record_event(
        EVENT_DEVICE_CREATED,
        None,
        Some(&device.id),
        now,
        &format!(r#"{{"name":{:?},"synthetic":true}}"#, device.name),
    )?;
    Ok(device)
}

/// Registers a device that was paired over the network.
///
/// Unlike [`create_synthetic`], the secret is returned to the caller so it can
/// be sent to the client exactly once. The plaintext is never stored or logged.
pub fn create_paired(
    db: &mut AuthDb,
    name: &str,
    platform: Option<&str>,
    client_version: Option<&str>,
    secret: &str,
    paired_via: &str,
    now: &str,
) -> Result<(Device, String)> {
    let name = name.trim();
    if name.is_empty() {
        bail!("a device needs a name — it is how you recognise what to revoke");
    }

    let device_id = random_id("dev_");
    let device = Device {
        id: device_id.clone(),
        name: name.to_string(),
        platform: platform.map(str::to_string),
        client_version: client_version.map(str::to_string),
        paired: now.to_string(),
        last_seen: None,
        revoked: None,
    };
    db.insert_device_paired(&device, &token::hash(secret), paired_via)?;
    db.record_event(
        EVENT_DEVICE_CREATED,
        None,
        Some(&device.id),
        now,
        &format!(
            r#"{{"name":{:?},"paired_via":{}}}"#,
            device.name,
            serde_json::to_string(paired_via).unwrap_or_default()
        ),
    )?;
    Ok((device, device_id))
}

/// Revokes a device **and every session on it**.
///
/// Those are one action, not two. A device whose sessions outlive its
/// revocation is a revocation that did nothing — the tokens already issued keep
/// working, which is exactly the case someone reaches for this in.
pub fn revoke(db: &mut AuthDb, id: &str, reason: &str, now: &str) -> Result<(Device, usize)> {
    let device = match db.find_device(id)? {
        Some(device) => device,
        None => bail!("no device `{id}`"),
    };
    let killed = super::sessions::revoke_for_device(db, &device.id, reason, now)?;
    db.mark_device_revoked(&device.id, reason, now)?;
    db.record_event(
        EVENT_DEVICE_REVOKED,
        None,
        Some(&device.id),
        now,
        &format!(r#"{{"reason":{reason:?},"sessions_revoked":{killed}}}"#),
    )?;
    Ok((device, killed))
}
