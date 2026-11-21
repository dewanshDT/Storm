//! Decision 161, slice 161-4: per-device notification settings — visible
//! from every device but writable only by its own (AM125). The table itself
//! is 161-1's (`db.rs`); this is the first code to read or write it.

use anyhow::Result;
use rusqlite::{OptionalExtension, params};

use super::db::AuthDb;

/// The known device-prefs classes (161-4 brief). `classes: []` on a row
/// means every one of these, not none.
pub const CLASSES: &[&str] = &[
    "needs_you",
    "delete_requests",
    "finished",
    "errors",
    "agent_messages",
    "hosts",
];

pub const PREVIEWS: &[&str] = &["full", "session", "storm"];

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct DevicePrefs {
    pub device_id: String,
    pub enabled: bool,
    pub classes: Vec<String>,
    pub preview: String,
    pub updated_at: String,
    /// This device's push endpoint health (decision 161, slice 161-6):
    /// `None` here from the database row alone — `ops::get_device_prefs`
    /// fills it in with one extra, indexed `push_endpoints` query, cheap
    /// enough to do on every `GET /v1/notifications/device`. `None` means
    /// no endpoint registered, not "unhealthy".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub push: Option<super::push_endpoints::PushEndpointHealth>,
}

/// One row of `GET /v1/notifications/devices` — this device's prefs beside
/// its name, for a future "Other devices" list. `None` fields mean the
/// device has never had a row created (it has never opened the app since
/// this slice shipped).
#[derive(Debug, Clone, serde::Serialize)]
pub struct DeviceRow {
    pub device_id: String,
    pub device_name: String,
    pub platform: Option<String>,
    pub revoked: bool,
    pub enabled: Option<bool>,
    pub classes: Option<Vec<String>>,
    pub preview: Option<String>,
}

/// Q19: desktop defaults on; everything else (web, Android, iOS, or a
/// platform this server has never heard named) stays off until the person
/// taps "Enable notifications".
fn default_enabled(platform: Option<&str>) -> bool {
    matches!(platform, Some("macos" | "windows" | "linux"))
}

fn encode_classes(classes: &[String]) -> String {
    serde_json::to_string(classes).unwrap_or_else(|_| "[]".into())
}

fn decode_classes(raw: &str) -> Vec<String> {
    serde_json::from_str(raw).unwrap_or_default()
}

const COLUMNS: &str = "device_id, enabled, classes, preview, updated_at";

fn row_to_prefs(row: &rusqlite::Row<'_>) -> rusqlite::Result<DevicePrefs> {
    let classes_raw: String = row.get(2)?;
    Ok(DevicePrefs {
        device_id: row.get(0)?,
        enabled: row.get(1)?,
        classes: decode_classes(&classes_raw),
        preview: row.get(3)?,
        updated_at: row.get(4)?,
        push: None,
    })
}

impl AuthDb {
    pub fn device_prefs(&self, device_id: &str) -> Result<Option<DevicePrefs>> {
        Ok(self
            .conn
            .query_row(
                &format!("SELECT {COLUMNS} FROM device_notification_prefs WHERE device_id = ?1"),
                params![device_id],
                row_to_prefs,
            )
            .optional()?)
    }

    /// Returns this device's row, creating it first if this is the first
    /// time — with the default for `platform`, or for the device's own
    /// recorded platform when the caller does not know it (an older client
    /// calling `PUT` with no `GET` first).
    pub fn ensure_device_prefs(
        &mut self,
        device_id: &str,
        platform: Option<&str>,
        now: &str,
    ) -> Result<DevicePrefs> {
        if let Some(existing) = self.device_prefs(device_id)? {
            return Ok(existing);
        }
        let platform = match platform {
            Some(p) => Some(p.to_string()),
            None => self
                .conn
                .query_row(
                    "SELECT platform FROM client_devices WHERE id = ?1",
                    params![device_id],
                    |r| r.get(0),
                )
                .optional()?
                .flatten(),
        };
        let enabled = default_enabled(platform.as_deref());
        self.conn.execute(
            "INSERT INTO device_notification_prefs (device_id, enabled, classes, preview, updated_at)
             VALUES (?1, ?2, '[]', 'full', ?3)
             ON CONFLICT(device_id) DO NOTHING",
            params![device_id, enabled, now],
        )?;
        self.device_prefs(device_id)?
            .ok_or_else(|| anyhow::anyhow!("device prefs row missing right after it was inserted"))
    }

    /// Patches only the fields given. The caller validates `classes` and
    /// `preview` against [`CLASSES`]/[`PREVIEWS`] first — this is storage,
    /// not policy.
    pub fn update_device_prefs(
        &mut self,
        device_id: &str,
        enabled: Option<bool>,
        classes: Option<&[String]>,
        preview: Option<&str>,
        now: &str,
    ) -> Result<DevicePrefs> {
        if let Some(enabled) = enabled {
            self.conn.execute(
                "UPDATE device_notification_prefs SET enabled = ?2, updated_at = ?3 \
                 WHERE device_id = ?1",
                params![device_id, enabled, now],
            )?;
        }
        if let Some(classes) = classes {
            self.conn.execute(
                "UPDATE device_notification_prefs SET classes = ?2, updated_at = ?3 \
                 WHERE device_id = ?1",
                params![device_id, encode_classes(classes), now],
            )?;
        }
        if let Some(preview) = preview {
            self.conn.execute(
                "UPDATE device_notification_prefs SET preview = ?2, updated_at = ?3 \
                 WHERE device_id = ?1",
                params![device_id, preview, now],
            )?;
        }
        self.device_prefs(device_id)?
            .ok_or_else(|| anyhow::anyhow!("device prefs row missing after an update"))
    }

    /// `GET /v1/notifications/devices`: every paired device, its prefs row
    /// if it has one. Read-only — never creates a row, unlike
    /// [`AuthDb::ensure_device_prefs`].
    pub fn list_device_prefs(&self) -> Result<Vec<DeviceRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT d.id, d.name, d.platform, d.revoked IS NOT NULL, \
                    p.enabled, p.classes, p.preview \
             FROM client_devices d \
             LEFT JOIN device_notification_prefs p ON p.device_id = d.id \
             ORDER BY d.paired",
        )?;
        let rows = stmt.query_map([], |row| {
            let classes_raw: Option<String> = row.get(5)?;
            Ok(DeviceRow {
                device_id: row.get(0)?,
                device_name: row.get(1)?,
                platform: row.get(2)?,
                revoked: row.get(3)?,
                enabled: row.get(4)?,
                classes: classes_raw.map(|s| decode_classes(&s)),
                preview: row.get(6)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> AuthDb {
        AuthDb::open_in_memory().unwrap()
    }

    const NOW: &str = "2026-11-18T00:00:00Z";

    fn device(db: &mut AuthDb, platform: &str) -> String {
        super::super::devices::create_synthetic(db, &format!("test-{platform}"), NOW)
            .unwrap()
            .id
    }

    #[test]
    fn desktop_platforms_default_on_others_default_off() {
        for (platform, expect_on) in [
            ("macos", true),
            ("windows", true),
            ("linux", true),
            ("web", false),
            ("android", false),
            ("ios", false),
        ] {
            let mut db = db();
            let id = device(&mut db, platform);
            let prefs = db.ensure_device_prefs(&id, Some(platform), NOW).unwrap();
            assert_eq!(prefs.enabled, expect_on, "platform {platform}");
            assert_eq!(prefs.classes, Vec::<String>::new());
            assert_eq!(prefs.preview, "full");
        }
    }

    #[test]
    fn a_missing_platform_falls_back_to_the_devices_own_recorded_one() {
        let mut db = db();
        let id = device(&mut db, "synthetic"); // not a desktop platform
        let prefs = db.ensure_device_prefs(&id, None, NOW).unwrap();
        assert!(!prefs.enabled);
    }

    #[test]
    fn ensure_is_idempotent_and_never_resets_a_later_change() {
        let mut db = db();
        let id = device(&mut db, "macos");
        db.ensure_device_prefs(&id, Some("macos"), NOW).unwrap();
        db.update_device_prefs(&id, Some(false), None, None, NOW)
            .unwrap();
        let prefs = db.ensure_device_prefs(&id, Some("macos"), NOW).unwrap();
        assert!(
            !prefs.enabled,
            "a second ensure must not re-apply the default"
        );
    }

    #[test]
    fn update_patches_only_the_given_fields() {
        let mut db = db();
        let id = device(&mut db, "linux");
        db.ensure_device_prefs(&id, Some("linux"), NOW).unwrap();

        let after_classes = db
            .update_device_prefs(
                &id,
                None,
                Some(&["needs_you".to_string(), "errors".to_string()]),
                None,
                NOW,
            )
            .unwrap();
        assert!(after_classes.enabled, "enabled must be untouched");
        assert_eq!(after_classes.preview, "full", "preview must be untouched");
        assert_eq!(after_classes.classes, vec!["needs_you", "errors"]);

        let after_preview = db
            .update_device_prefs(&id, None, None, Some("storm"), NOW)
            .unwrap();
        assert_eq!(after_preview.classes, vec!["needs_you", "errors"]);
        assert_eq!(after_preview.preview, "storm");
    }

    #[test]
    fn two_devices_prefs_are_independent() {
        let mut db = db();
        let a = device(&mut db, "macos");
        let b = device(&mut db, "macos");
        db.ensure_device_prefs(&a, Some("macos"), NOW).unwrap();
        db.ensure_device_prefs(&b, Some("macos"), NOW).unwrap();
        db.update_device_prefs(&a, Some(false), None, None, NOW)
            .unwrap();

        assert!(!db.device_prefs(&a).unwrap().unwrap().enabled);
        assert!(
            db.device_prefs(&b).unwrap().unwrap().enabled,
            "b is untouched by a's write"
        );
    }

    #[test]
    fn list_device_prefs_left_joins_devices_with_no_row_yet() {
        let mut db = db();
        let with_row = device(&mut db, "macos");
        let without_row = device(&mut db, "web");
        db.ensure_device_prefs(&with_row, Some("macos"), NOW)
            .unwrap();

        let rows = db.list_device_prefs().unwrap();
        let a = rows.iter().find(|r| r.device_id == with_row).unwrap();
        assert_eq!(a.enabled, Some(true));
        let b = rows.iter().find(|r| r.device_id == without_row).unwrap();
        assert_eq!(b.enabled, None);
        assert_eq!(b.classes, None);
    }
}
