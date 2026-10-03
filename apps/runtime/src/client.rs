//! Talking to the Storm Server: enrollment and key authentication (freeze
//! §5.3–5.4; decision 77b).
//!
//! **The server is verified before it is trusted, on every connect.** The host
//! sends `POST /v1/server/challenge` with a fresh nonce and checks the answer
//! against the key pinned at enrollment. Until that passes it sends nothing
//! else, so an enrollment token or a key signature never goes to a server
//! that cannot prove it is the one the owner meant.

use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use data_encoding::BASE64URL_NOPAD;
use ed25519_dalek::VerifyingKey;
use rand::Rng;
use serde::Deserialize;
use serde_json::json;

use crate::identity::{
    Enrollment, HostConfig, HostKey, challenge_message, connect_message, decode_public_key,
    enroll_message, token_id, validate_host_id, verify,
};

/// A server this host has pinned.
pub struct ServerClient {
    http: reqwest::Client,
    base: String,
    server_id: String,
    server_key: VerifyingKey,
}

#[derive(Deserialize)]
struct ChallengeAnswer {
    server_id: String,
    signature: String,
}

#[derive(Deserialize)]
struct Enrolled {
    host_id: String,
    server_id: String,
}

#[derive(Deserialize)]
struct Nonce {
    nonce: String,
}

/// A host token, minted by proving the key.
#[derive(Debug, Clone, Deserialize)]
pub struct HostToken {
    pub token: String,
    pub expires: String,
}

impl ServerClient {
    pub fn new(base: &str, server_id: &str, server_pubkey: &str) -> Result<Self> {
        let server_key = decode_public_key(server_pubkey).map_err(|e| anyhow!("{e}"))?;
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()
            .context("building the HTTP client")?;
        Ok(Self {
            http,
            base: base.trim_end_matches('/').to_string(),
            server_id: server_id.to_string(),
            server_key,
        })
    }

    pub fn for_host(config: &HostConfig) -> Result<Self> {
        Self::new(&config.server_url, &config.server_id, &config.server_pubkey)
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    pub fn http(&self) -> &reqwest::Client {
        &self.http
    }

    /// Proves the server holds the pinned key. Called before anything else.
    pub async fn verify_server(&self) -> Result<()> {
        let mut raw = [0u8; 24];
        rand::rng().fill_bytes(&mut raw);
        let nonce = BASE64URL_NOPAD.encode(&raw);
        let answer: ChallengeAnswer = self
            .http
            .post(format!("{}/v1/server/challenge", self.base))
            .json(&json!({ "nonce": nonce }))
            .send()
            .await
            .with_context(|| format!("reaching {}", self.base))?
            .error_for_status()
            .context("the server refused its identity challenge")?
            .json()
            .await
            .context("reading the server's identity answer")?;
        if answer.server_id != self.server_id {
            bail!(
                "the server at {} is {}, not the pinned {} — refusing to talk to it",
                self.base,
                answer.server_id,
                self.server_id
            );
        }
        if !verify(
            &self.server_key,
            &challenge_message(&self.server_id, &nonce),
            &answer.signature,
        ) {
            bail!(
                "the server at {} could not prove it holds the pinned key — refusing to talk to it",
                self.base
            );
        }
        Ok(())
    }

    /// Consumes the enrollment token, registering `key` as this host's.
    pub async fn enroll(&self, token: &str, key: &HostKey, name: &str) -> Result<String> {
        let tid = token_id(token).context("malformed enrollment token")?;
        let response = self
            .http
            .post(format!("{}/v1/runtime/enroll", self.base))
            .json(&json!({
                "token": token,
                "public_key": key.public_key_b64(),
                "key_id": key.key_id,
                "name": name,
                "signature": key.sign(&enroll_message(&self.server_id, &tid)),
            }))
            .send()
            .await
            .context("sending the enrollment")?;
        match response.status() {
            s if s.is_success() => {}
            reqwest::StatusCode::UNAUTHORIZED => bail!(
                "the server refused the enrollment: it may have expired (10 minutes) or \
                 already been used. Issue a new one from the app."
            ),
            reqwest::StatusCode::TOO_MANY_REQUESTS => {
                bail!("the server is rate-limiting this address; wait a minute and retry")
            }
            s => bail!("the server answered the enrollment with {s}"),
        }
        let enrolled: Enrolled = response.json().await.context("reading the enrollment")?;
        if enrolled.server_id != self.server_id {
            bail!("the enrollment answer names another server");
        }
        validate_host_id(&enrolled.host_id).map_err(|e| anyhow!("{e}"))?;
        Ok(enrolled.host_id)
    }

    /// Challenge, sign, and receive a host token (freeze §5.4).
    pub async fn authenticate(&self, host_id: &str, key: &HostKey) -> Result<HostToken> {
        let nonce: Nonce = self
            .http
            .post(format!("{}/v1/runtime/auth/challenge", self.base))
            .json(&json!({ "host_id": host_id }))
            .send()
            .await
            .context("asking for a challenge")?
            .error_for_status()
            .map_err(refusal)?
            .json()
            .await?;
        let signature = key.sign(&connect_message(&self.server_id, host_id, &nonce.nonce));
        self.http
            .post(format!("{}/v1/runtime/auth", self.base))
            .json(&json!({ "host_id": host_id, "nonce": nonce.nonce, "signature": signature }))
            .send()
            .await
            .context("proving the host key")?
            .error_for_status()
            .map_err(refusal)?
            .json()
            .await
            .context("reading the host token")
    }
}

/// Distinguishes a refusal — revoked, or the server forgot this host — from a
/// transient failure, because the caller must stop retrying on the first.
fn refusal(e: reqwest::Error) -> anyhow::Error {
    if e.status() == Some(reqwest::StatusCode::UNAUTHORIZED) {
        anyhow::Error::new(Refused)
    } else {
        e.into()
    }
}

/// The server refused this host's key: it was revoked, or never existed.
/// Not retryable — a revoked host ends its sessions (freeze §5.6).
#[derive(Debug)]
pub struct Refused;

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the server refused this host's key (revoked, or re-enroll it)")
    }
}

impl std::error::Error for Refused {}

/// The whole of `storm-runtime enroll`, after the string has been read.
///
/// Order matters:
/// 1. Verify the server.
/// 2. Write the key.
/// 3. Enroll.
/// 4. Write `host.json`.
///
/// `host.json` existing therefore implies a key on disk that the server knows.
pub async fn enroll(
    state_dir: &Path,
    enrollment: &str,
    name: &str,
    force: bool,
) -> Result<HostConfig> {
    let enrollment = Enrollment::parse(enrollment).map_err(|e| anyhow!("{e}"))?;
    if HostConfig::path(state_dir).exists() && !force {
        bail!(
            "{} already exists: this host is enrolled. Pass --force to enroll it again \
             (the old enrollment should be revoked in the app).",
            HostConfig::path(state_dir).display()
        );
    }
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 64 {
        bail!("a host name is 1–64 characters");
    }

    let client = ServerClient::new(
        &enrollment.server_url,
        &enrollment.server_id,
        &enrollment.server_pubkey,
    )?;
    client.verify_server().await?;

    let key = HostKey::generate();
    key.save(state_dir)?;
    let host_id = client.enroll(&enrollment.token, &key, name).await?;
    let config = HostConfig {
        server_url: enrollment.server_url,
        server_id: enrollment.server_id,
        server_pubkey: enrollment.server_pubkey,
        host_id,
        key_id: key.key_id.clone(),
    };
    config.save(state_dir)?;
    Ok(config)
}
