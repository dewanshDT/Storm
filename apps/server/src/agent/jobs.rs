//! Agent install jobs (decision 133): installing, updating or removing an
//! agent CLI on a Runtime Host. In memory only: a restart forgets them, and
//! the host's next inventory says what is installed.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// The agents a host can install (decision 133).
pub const INSTALLABLE: [&str; 5] = ["claude-code", "opencode", "codex", "cursor", "command-code"];
pub const OPS: [&str; 3] = ["install", "update", "remove"];
pub const STEPS: [&str; 3] = ["Downloading", "Verifying", "Installing"];
/// Why a host says it cannot install an agent.
pub const BLOCKED: [&str; 4] = [
    "disabled",
    "unsupported_platform",
    "no_node",
    "node_too_old",
];
/// How a host's job may fail. Anything else is stored as `unknown`, so a
/// host's text never reaches a client or the audit row.
pub const FAILURES: [&str; 15] = [
    "unknown_agent",
    "disabled",
    "unsupported_platform",
    "no_node",
    "node_too_old",
    "download_failed",
    "checksum_mismatch",
    "unpack_failed",
    "npm_failed",
    "version_probe_failed",
    "disk_full",
    "not_managed",
    "in_use",
    "busy",
    "timeout",
];

/// At most this many unfinished jobs per host.
pub const PER_HOST: usize = 2;
const MAX_JOBS: usize = 200;
const KEEP_FINISHED: Duration = Duration::from_secs(60 * 60);
/// A job its host has said nothing about for this long has been lost, most
/// likely to a host restart, and would otherwise hold its agent `busy`.
const STALE: Duration = Duration::from_secs(30 * 60);

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum JobState {
    Queued,
    Running,
    Done,
    Failed,
}

impl JobState {
    pub fn finished(self) -> bool {
        matches!(self, Self::Done | Self::Failed)
    }
}

/// A job as the client sees it.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct AgentJob {
    pub id: String,
    pub host_id: String,
    pub agent: String,
    pub op: String,
    pub state: JobState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Started by the host's own update policy, not from the app (133-5).
    pub auto: bool,
    pub created: String,
    pub updated: String,
}

/// What a host posts about a job: exactly one of the three.
#[derive(Debug, Clone, Deserialize)]
pub struct ReportBody {
    #[serde(default)]
    pub step: Option<String>,
    #[serde(default)]
    pub done: Option<DoneBody>,
    #[serde(default)]
    pub failed: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DoneBody {
    #[serde(default)]
    pub version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Report {
    Step(String),
    Done(Option<String>),
    Failed(String),
}

impl ReportBody {
    /// The report, or the reason it is malformed.
    pub fn parse(self) -> Result<Report, &'static str> {
        match (self.step, self.done, self.failed) {
            (Some(step), None, None) => STEPS
                .contains(&step.as_str())
                .then_some(Report::Step(step))
                .ok_or("unknown_step"),
            (None, Some(done), None) => match done.version {
                Some(v) if !valid_version(&v) => Err("bad_version"),
                v => Ok(Report::Done(v)),
            },
            (None, None, Some(code)) => Ok(Report::Failed(if FAILURES.contains(&code.as_str()) {
                code
            } else {
                "unknown".into()
            })),
            _ => Err("one_of_step_done_failed"),
        }
    }
}

fn valid_version(v: &str) -> bool {
    !v.is_empty()
        && v.len() <= 64
        && v.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

struct Job {
    view: AgentJob,
    /// The account that started it, for the audit row; `None` for `auto`.
    user_id: Option<String>,
    touched: Instant,
}

/// A finished job and who started it, for its audit row.
#[derive(Debug, Clone)]
pub struct Finished {
    pub job: AgentJob,
    pub user_id: Option<String>,
}

pub struct NewJob<'a> {
    pub host_id: &'a str,
    pub agent: &'a str,
    pub op: &'a str,
    pub auto: bool,
    pub user_id: Option<&'a str>,
    pub state: JobState,
}

#[derive(Default)]
pub struct Jobs {
    jobs: HashMap<String, Job>,
}

fn new_id() -> String {
    let random = crate::auth::identity::random_id("");
    format!("agj_{}", &random[..24])
}

impl Jobs {
    /// Fails the jobs that went stale and forgets old finished ones. Returns
    /// the jobs it failed, to be audited.
    pub fn prune(&mut self, now: Instant, stamp: &str) -> Vec<Finished> {
        let mut failed = Vec::new();
        for job in self.jobs.values_mut() {
            if !job.view.state.finished() && now.duration_since(job.touched) > STALE {
                job.view.state = JobState::Failed;
                job.view.error = Some("timeout".into());
                job.view.updated = stamp.to_string();
                job.touched = now;
                failed.push(Finished {
                    job: job.view.clone(),
                    user_id: job.user_id.clone(),
                });
            }
        }
        self.jobs.retain(|_, j| {
            !j.view.state.finished() || now.duration_since(j.touched) <= KEEP_FINISHED
        });
        if self.jobs.len() > MAX_JOBS {
            let mut finished: Vec<(Instant, String)> = self
                .jobs
                .iter()
                .filter(|(_, j)| j.view.state.finished())
                .map(|(id, j)| (j.touched, id.clone()))
                .collect();
            finished.sort();
            let excess = self.jobs.len() - MAX_JOBS;
            for (_, id) in finished.into_iter().take(excess) {
                self.jobs.remove(&id);
            }
        }
        failed
    }

    /// Admits a job unless its agent or its host is already busy.
    pub fn admit(&mut self, new: NewJob<'_>, stamp: &str) -> Result<AgentJob, &'static str> {
        let unfinished = self
            .jobs
            .values()
            .filter(|j| j.view.host_id == new.host_id && !j.view.state.finished());
        let mut on_host = 0;
        for j in unfinished {
            if j.view.agent == new.agent {
                return Err("busy");
            }
            on_host += 1;
        }
        if on_host >= PER_HOST {
            return Err("busy");
        }
        let view = AgentJob {
            id: new_id(),
            host_id: new.host_id.to_string(),
            agent: new.agent.to_string(),
            op: new.op.to_string(),
            state: new.state,
            step: None,
            error: None,
            version: None,
            auto: new.auto,
            created: stamp.to_string(),
            updated: stamp.to_string(),
        };
        self.jobs.insert(
            view.id.clone(),
            Job {
                view: view.clone(),
                user_id: new.user_id.map(str::to_string),
                touched: Instant::now(),
            },
        );
        Ok(view)
    }

    pub fn remove(&mut self, id: &str) {
        self.jobs.remove(id);
    }

    /// The job, if it is `host_id`'s.
    pub fn get(&self, host_id: &str, id: &str) -> Option<AgentJob> {
        self.jobs
            .get(id)
            .filter(|j| j.view.host_id == host_id)
            .map(|j| j.view.clone())
    }

    /// Applies a host's report. `None` when the job is not that host's;
    /// `Some(Some(_))` when this report finished it. A finished job ignores
    /// further reports.
    pub fn report(
        &mut self,
        host_id: &str,
        id: &str,
        report: Report,
        stamp: &str,
    ) -> Option<Option<Finished>> {
        let job = self
            .jobs
            .get_mut(id)
            .filter(|j| j.view.host_id == host_id)?;
        if job.view.state.finished() {
            return Some(None);
        }
        let view = &mut job.view;
        match report {
            Report::Step(step) => {
                view.state = JobState::Running;
                view.step = Some(step);
            }
            Report::Done(version) => {
                view.state = JobState::Done;
                view.version = version;
            }
            Report::Failed(code) => {
                view.state = JobState::Failed;
                view.error = Some(code);
            }
        }
        view.updated = stamp.to_string();
        job.touched = Instant::now();
        Some(view.state.finished().then(|| Finished {
            job: view.clone(),
            user_id: job.user_id.clone(),
        }))
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.jobs.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new<'a>(host: &'a str, agent: &'a str) -> NewJob<'a> {
        NewJob {
            host_id: host,
            agent,
            op: "install",
            auto: false,
            user_id: Some("usr_1"),
            state: JobState::Queued,
        }
    }

    fn body(v: serde_json::Value) -> Result<Report, &'static str> {
        serde_json::from_value::<ReportBody>(v).unwrap().parse()
    }

    #[test]
    fn a_job_id_is_agj_and_24_alphanumerics() {
        let id = new_id();
        let rest = id.strip_prefix("agj_").unwrap();
        assert_eq!(rest.len(), 24);
        assert!(rest.bytes().all(|b| b.is_ascii_alphanumeric()));
    }

    #[test]
    fn reports_are_exactly_one_known_thing() {
        use serde_json::json;
        assert_eq!(
            body(json!({"step": "Verifying"})),
            Ok(Report::Step("Verifying".into()))
        );
        assert_eq!(body(json!({"step": "Hacking"})), Err("unknown_step"));
        assert_eq!(
            body(json!({"done": {"version": "2.1.0-beta_1"}})),
            Ok(Report::Done(Some("2.1.0-beta_1".into())))
        );
        assert_eq!(
            body(json!({"done": {"version": null}})),
            Ok(Report::Done(None))
        );
        assert_eq!(body(json!({"done": {}})), Ok(Report::Done(None)));
        assert_eq!(
            body(json!({"done": {"version": "1.0 /tmp/x"}})),
            Err("bad_version")
        );
        assert_eq!(
            body(json!({"done": {"version": "1".repeat(65)}})),
            Err("bad_version")
        );
        assert_eq!(
            body(json!({"failed": "npm_failed"})),
            Ok(Report::Failed("npm_failed".into()))
        );
        assert_eq!(
            body(json!({"failed": "see https://x/y token=abc"})),
            Ok(Report::Failed("unknown".into()))
        );
        assert!(body(json!({})).is_err());
        assert!(body(json!({"step": "Verifying", "failed": "busy"})).is_err());
    }

    #[test]
    fn one_job_per_agent_and_two_per_host() {
        let mut jobs = Jobs::default();
        let a = jobs.admit(new("h1", "codex"), "t").unwrap();
        assert_eq!(jobs.admit(new("h1", "codex"), "t"), Err("busy"));
        jobs.admit(new("h1", "cursor"), "t").unwrap();
        assert_eq!(jobs.admit(new("h1", "opencode"), "t"), Err("busy"));
        jobs.admit(new("h2", "codex"), "t").unwrap();
        jobs.report("h1", &a.id, Report::Done(None), "t2").unwrap();
        jobs.admit(new("h1", "opencode"), "t").unwrap();
    }

    #[test]
    fn a_host_reports_only_its_own_jobs_and_a_finished_job_stays_finished() {
        let mut jobs = Jobs::default();
        let a = jobs.admit(new("h1", "codex"), "t").unwrap();
        assert!(jobs.report("h2", &a.id, Report::Done(None), "t").is_none());
        assert!(jobs.get("h2", &a.id).is_none());
        let step = jobs.report("h1", &a.id, Report::Step("Downloading".into()), "t1");
        assert!(matches!(step, Some(None)));
        assert_eq!(jobs.get("h1", &a.id).unwrap().state, JobState::Running);
        let done = jobs
            .report("h1", &a.id, Report::Failed("disk_full".into()), "t2")
            .unwrap()
            .unwrap();
        assert_eq!(done.job.error.as_deref(), Some("disk_full"));
        assert!(matches!(
            jobs.report("h1", &a.id, Report::Done(Some("1".into())), "t3"),
            Some(None)
        ));
        let job = jobs.get("h1", &a.id).unwrap();
        assert_eq!(
            (job.state, job.version, job.updated.as_str()),
            (JobState::Failed, None, "t2")
        );
    }

    #[test]
    fn stale_jobs_time_out_and_old_finished_jobs_are_forgotten() {
        let mut jobs = Jobs::default();
        let stale = jobs.admit(new("h1", "codex"), "t").unwrap();
        let old = jobs.admit(new("h2", "codex"), "t").unwrap();
        jobs.report("h2", &old.id, Report::Done(None), "t").unwrap();
        assert!(jobs.prune(Instant::now(), "t").is_empty());
        let later = Instant::now() + KEEP_FINISHED + Duration::from_secs(1);
        let failed = jobs.prune(later, "t9");
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].job.id, stale.id);
        assert_eq!(failed[0].job.error.as_deref(), Some("timeout"));
        assert_eq!(jobs.get("h1", &stale.id).unwrap().state, JobState::Failed);
        assert!(jobs.get("h2", &old.id).is_none());

        let fresh = jobs.admit(new("h3", "codex"), "t").unwrap();
        for i in 0..MAX_JOBS + 10 {
            let host = format!("x{i}");
            let j = jobs.admit(new(&host, "codex"), "t").unwrap();
            jobs.report(&host, &j.id, Report::Done(None), "t").unwrap();
        }
        jobs.prune(Instant::now(), "t");
        assert_eq!(jobs.len(), MAX_JOBS);
        assert!(jobs.get("h3", &fresh.id).is_some());
    }
}
