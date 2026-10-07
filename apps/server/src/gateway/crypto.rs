//! The gateway's data key and its AEAD (spec §8, decision 81b).
//!
//! # What this encryption buys, plainly
//!
//! It protects an upstream credential against **a leak of `gateway.db`
//! alone**: a copied database file, a stray `VACUUM INTO`, a backup of the
//! database without the key directory. It does **not** protect against root on
//! the server, against the service user, or against anyone holding a full
//! backup — all of those can read the key file sitting next to the database.
//! That is the accepted limit (spec risk 3), and the reason the key is a file:
//! who can read it is `ls -l`-auditable, exactly like the server's identity
//! key (A2).
//!
//! # The shape
//!
//! - **XChaCha20-Poly1305**, from the pure-Rust `chacha20poly1305` crate. A
//!   random 24-byte nonce per seal; XChaCha's nonce is long enough that
//!   random nonces need no counter and no coordination.
//! - **The AAD is `<id> 0x00 <kind>`** — the spec's `connection_id ‖ kind`,
//!   with a separator so the split is unambiguous. A ciphertext copied into
//!   another connection's row, or from a `static` row into an `oauth_tokens`
//!   row, fails to open rather than decrypting as someone else's secret.
//! - **Keys live in `state/gateway/keys/<key_id>.key`**: 32 raw bytes, `0600`,
//!   in a `0700` directory, each created *with* its mode (no window at a wider
//!   one). `gateway.db`'s `meta.active_key_id` says which one seals; every
//!   key file that loads can open, because a row records the key that sealed
//!   it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use rand::Rng;

pub const KEY_LEN: usize = 32;
pub const NONCE_LEN: usize = 24;

/// A sealed value as it sits in a row.
#[derive(Clone, PartialEq, Eq)]
pub struct Sealed {
    pub key_id: String,
    pub nonce: Vec<u8>,
    pub ciphertext: Vec<u8>,
}

impl std::fmt::Debug for Sealed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The ciphertext is not secret, but there is no reason for it to reach
        // a log either; the length is what a reader debugging this wants.
        f.debug_struct("Sealed")
            .field("key_id", &self.key_id)
            .field("ciphertext_len", &self.ciphertext.len())
            .finish()
    }
}

/// An opened secret. **Never `Debug`-printed, never serialized**: the
/// redacting impl below is what keeps `?value` in a `tracing` call from being
/// a credential disclosure, the rule `ServerIdentity` follows for its key.
pub struct Plaintext(Vec<u8>);

impl Plaintext {
    pub fn expose(&self) -> &[u8] {
        &self.0
    }
}

impl std::fmt::Debug for Plaintext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Plaintext(<redacted>)")
    }
}

impl Drop for Plaintext {
    fn drop(&mut self) {
        // Best effort: overwrite before the allocation is freed. The compiler
        // may still have copied it elsewhere; this only shortens its life.
        self.0.iter_mut().for_each(|b| *b = 0);
    }
}

/// Every data key that could be loaded, and which one seals.
pub struct Keyring {
    active: String,
    keys: HashMap<String, XChaCha20Poly1305>,
}

impl std::fmt::Debug for Keyring {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut ids: Vec<&String> = self.keys.keys().collect();
        ids.sort();
        f.debug_struct("Keyring")
            .field("active", &self.active)
            .field("loaded", &ids)
            .finish()
    }
}

/// The AAD: `id 0x00 kind`. Neither half may contain the separator, so no two
/// distinct pairs share an AAD.
fn aad(id: &str, kind: &str) -> Result<Vec<u8>> {
    if id.is_empty() || kind.is_empty() || id.contains('\0') || kind.contains('\0') {
        bail!("an AAD half is empty or contains NUL");
    }
    let mut out = Vec::with_capacity(id.len() + 1 + kind.len());
    out.extend_from_slice(id.as_bytes());
    out.push(0);
    out.extend_from_slice(kind.as_bytes());
    Ok(out)
}

impl Keyring {
    pub fn active_key_id(&self) -> &str {
        &self.active
    }

    pub fn has_key(&self, key_id: &str) -> bool {
        self.keys.contains_key(key_id)
    }

    /// Seals `plaintext` for the row `(id, kind)` under the active key.
    pub fn seal(&self, id: &str, kind: &str, plaintext: &[u8]) -> Result<Sealed> {
        let cipher = self
            .keys
            .get(&self.active)
            .context("the active data key is not loaded")?;
        let mut nonce = [0u8; NONCE_LEN];
        rand::rng().fill_bytes(&mut nonce);
        let ciphertext = cipher
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: plaintext,
                    aad: &aad(id, kind)?,
                },
            )
            .map_err(|_| anyhow::anyhow!("sealing failed"))?;
        Ok(Sealed {
            key_id: self.active.clone(),
            nonce: nonce.to_vec(),
            ciphertext,
        })
    }

    /// Opens a row sealed for `(id, kind)`. Fails — never returns garbage —
    /// if the key is missing, the row was tampered with, or it belongs to
    /// another `(id, kind)`.
    pub fn open(&self, id: &str, kind: &str, sealed: &Sealed) -> Result<Plaintext> {
        let cipher = self
            .keys
            .get(&sealed.key_id)
            .with_context(|| format!("data key {} is not loaded", sealed.key_id))?;
        if sealed.nonce.len() != NONCE_LEN {
            bail!("a sealed value has a {}-byte nonce", sealed.nonce.len());
        }
        let plaintext = cipher
            .decrypt(
                XNonce::from_slice(&sealed.nonce),
                Payload {
                    msg: &sealed.ciphertext,
                    aad: &aad(id, kind)?,
                },
            )
            .map_err(|_| anyhow::anyhow!("a sealed value did not authenticate"))?;
        Ok(Plaintext(plaintext))
    }
}

pub fn keys_dir(state_dir: &Path) -> PathBuf {
    state_dir.join(super::GATEWAY_DIR).join(super::KEYS_DIR)
}

pub fn key_path(state_dir: &Path, key_id: &str) -> PathBuf {
    keys_dir(state_dir).join(format!("{key_id}.key"))
}

/// What loading found, for the caller to act on and log.
pub struct Loaded {
    pub keyring: Keyring,
    /// The active key named in the database could not be loaded, so a new one
    /// was made active. Rows sealed under the lost key cannot be opened.
    pub replaced_lost_key: Option<String>,
}

/// Loads every key file and settles which key is active.
///
/// `recorded_active` is `gateway.db`'s `meta.active_key_id`, if it has one.
/// A missing or unreadable active key is **not** a refusal to boot: the spec
/// says losing the key means reconnecting, not a lockout (§8). A fresh key is
/// made active, the caller marks the affected connections `needs_reauth`, and
/// the journal says why. Other key files are left exactly where they are.
pub fn load_or_create(state_dir: &Path, recorded_active: Option<&str>) -> Result<Loaded> {
    let dir = keys_dir(state_dir);
    create_private_dir(&dir)?;

    let mut keys = HashMap::new();
    for entry in std::fs::read_dir(&dir).with_context(|| format!("reading {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        let Some(key_id) = path
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.strip_suffix(".key"))
            .map(str::to_string)
        else {
            continue;
        };
        if !entry.file_type()?.is_file() {
            continue;
        }
        match read_key_file(&path) {
            Ok(bytes) => {
                warn_if_readable_by_others(&path);
                keys.insert(key_id, XChaCha20Poly1305::new_from_slice(&bytes)?);
            }
            Err(e) => tracing::warn!(
                path = %path.display(),
                error = %e,
                "a gateway data key could not be loaded; credentials sealed under it need reconnecting"
            ),
        }
    }

    if let Some(active) = recorded_active
        && keys.contains_key(active)
    {
        return Ok(Loaded {
            keyring: Keyring {
                active: active.to_string(),
                keys,
            },
            replaced_lost_key: None,
        });
    }

    let key_id = crate::auth::identity::random_id("gwk_");
    let mut secret = [0u8; KEY_LEN];
    rand::rng().fill_bytes(&mut secret);
    write_key_file(&key_path(state_dir, &key_id), &secret)?;
    keys.insert(key_id.clone(), XChaCha20Poly1305::new_from_slice(&secret)?);
    secret.iter_mut().for_each(|b| *b = 0);

    Ok(Loaded {
        keyring: Keyring {
            active: key_id,
            keys,
        },
        replaced_lost_key: recorded_active.map(str::to_string),
    })
}

fn read_key_file(path: &Path) -> Result<[u8; KEY_LEN]> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    bytes
        .as_slice()
        .try_into()
        .map_err(|_| anyhow::anyhow!("{} is {} bytes, not {KEY_LEN}", path.display(), bytes.len()))
}

/// Creates a directory at `0700`, and tightens it if it already existed wider.
/// Created *with* the mode, so there is no window where it is listable.
pub(crate) fn create_private_dir(dir: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        if let Some(parent) = dir.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        if !dir.exists() {
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(dir)
                .with_context(|| format!("creating {}", dir.display()))?;
        }
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
            .with_context(|| format!("tightening {}", dir.display()))?;
    }
    #[cfg(not(unix))]
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    Ok(())
}

/// Writes key bytes at `0600` with no window at a wider mode — the identity
/// key's rule (`auth::identity::write_key_file`).
fn write_key_file(path: &Path, secret: &[u8; KEY_LEN]) -> Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    use std::io::Write;
    let mut file = options
        .open(path)
        .with_context(|| format!("creating {}", path.display()))?;
    file.write_all(secret)
        .with_context(|| format!("writing {}", path.display()))?;
    file.sync_all()
        .with_context(|| format!("flushing {}", path.display()))?;
    Ok(())
}

/// A warning, not a refusal, for the identity key's reason: a restore that
/// lost permissions should produce a journal line, not a server that will not
/// start.
fn warn_if_readable_by_others(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(path) {
            let mode = meta.permissions().mode() & 0o777;
            if mode & 0o077 != 0 {
                tracing::warn!(
                    path = %path.display(),
                    mode = format!("{mode:o}"),
                    "a gateway data key is readable beyond its owner; chmod 600 it"
                );
            }
        }
    }
    #[cfg(not(unix))]
    let _ = path;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ring(dir: &Path) -> Keyring {
        load_or_create(dir, None).unwrap().keyring
    }

    #[test]
    fn a_sealed_value_opens_for_its_own_row_only() {
        let dir = tempdir::TempDir::new("storm-gw-crypto").unwrap();
        let k = ring(dir.path());
        let sealed = k.seal("mcc_A", "static", b"Bearer secret-pat").unwrap();
        assert_eq!(
            k.open("mcc_A", "static", &sealed).unwrap().expose(),
            b"Bearer secret-pat"
        );
        // The swap the AAD exists to stop: the same bytes in another
        // connection's row, or under another kind, do not open.
        assert!(k.open("mcc_B", "static", &sealed).is_err());
        assert!(k.open("mcc_A", "oauth_tokens", &sealed).is_err());
        // And the separator is what makes the split unambiguous.
        assert_ne!(aad("ab", "c").unwrap(), aad("a", "bc").unwrap());
        assert!(aad("a\0b", "c").is_err());
    }

    #[test]
    fn a_tampered_ciphertext_does_not_open() {
        let dir = tempdir::TempDir::new("storm-gw-tamper").unwrap();
        let k = ring(dir.path());
        let mut sealed = k.seal("mcc_A", "static", b"value").unwrap();
        sealed.ciphertext[0] ^= 1;
        assert!(k.open("mcc_A", "static", &sealed).is_err());
    }

    #[test]
    fn the_ciphertext_does_not_contain_the_plaintext() {
        let dir = tempdir::TempDir::new("storm-gw-ct").unwrap();
        let k = ring(dir.path());
        let secret = b"upstream-canary-0123456789abcdef";
        let a = k.seal("mcc_A", "static", secret).unwrap();
        let b = k.seal("mcc_A", "static", secret).unwrap();
        assert!(!a.ciphertext.windows(secret.len()).any(|w| w == secret));
        // A fresh nonce per seal: the same secret twice is two ciphertexts.
        assert_ne!(a.nonce, b.nonce);
        assert_ne!(a.ciphertext, b.ciphertext);
    }

    #[test]
    fn the_key_is_0600_in_a_0700_directory() {
        let dir = tempdir::TempDir::new("storm-gw-mode").unwrap();
        let k = ring(dir.path());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let key = key_path(dir.path(), k.active_key_id());
            let mode = std::fs::metadata(&key).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "the data key is {mode:o}");
            let dmode = std::fs::metadata(keys_dir(dir.path()))
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(dmode, 0o700, "the key directory is {dmode:o}");
        }
        assert_eq!(
            std::fs::read(key_path(dir.path(), k.active_key_id()))
                .unwrap()
                .len(),
            KEY_LEN
        );
    }

    #[test]
    fn the_recorded_key_is_reloaded_and_still_opens() {
        let dir = tempdir::TempDir::new("storm-gw-reload").unwrap();
        let first = ring(dir.path());
        let sealed = first.seal("mcc_A", "static", b"value").unwrap();
        let again = load_or_create(dir.path(), Some(first.active_key_id())).unwrap();
        assert!(again.replaced_lost_key.is_none());
        assert_eq!(again.keyring.active_key_id(), first.active_key_id());
        assert_eq!(
            again
                .keyring
                .open("mcc_A", "static", &sealed)
                .unwrap()
                .expose(),
            b"value"
        );
    }

    #[test]
    fn a_lost_key_is_replaced_not_a_lockout() {
        let dir = tempdir::TempDir::new("storm-gw-lost").unwrap();
        let first = ring(dir.path());
        let sealed = first.seal("mcc_A", "static", b"value").unwrap();
        std::fs::remove_file(key_path(dir.path(), first.active_key_id())).unwrap();

        let loaded = load_or_create(dir.path(), Some(first.active_key_id())).unwrap();
        assert_eq!(
            loaded.replaced_lost_key.as_deref(),
            Some(first.active_key_id())
        );
        assert_ne!(loaded.keyring.active_key_id(), first.active_key_id());
        assert!(!loaded.keyring.has_key(first.active_key_id()));
        // The old row cannot be opened; it is not silently misread.
        assert!(loaded.keyring.open("mcc_A", "static", &sealed).is_err());
    }

    #[test]
    fn nothing_secret_is_debug_printed() {
        let dir = tempdir::TempDir::new("storm-gw-debug").unwrap();
        let k = ring(dir.path());
        let sealed = k.seal("mcc_A", "static", b"hunter2-secret").unwrap();
        let opened = k.open("mcc_A", "static", &sealed).unwrap();
        let printed = format!("{k:?} {sealed:?} {opened:?}");
        assert!(!printed.contains("hunter2"));
        assert!(printed.contains("redacted"));
    }
}
