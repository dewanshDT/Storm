//! Every request the helper sees is audited (136c): one line on stderr,
//! which the unit sends to the journal. A line names the operation, the
//! outcome, the refusal code and the peer, never a value from the request.

use crate::protocol::{Op, Refusal};

pub trait Audit {
    fn line(&mut self, line: String);
}

pub struct Journal;

impl Audit for Journal {
    fn line(&mut self, line: String) {
        eprintln!("{line}");
    }
}

impl Audit for Vec<String> {
    fn line(&mut self, line: String) {
        self.push(line);
    }
}

fn peer(uid: Option<u32>) -> String {
    uid.map_or("-".into(), |u| u.to_string())
}

pub fn refused(audit: &mut dyn Audit, op: Option<Op>, why: &Refusal, peer_uid: Option<u32>) {
    let mut line = format!(
        "audit: refused op={} reason={} peer_uid={}",
        op.map_or("-", Op::name),
        why.code(),
        peer(peer_uid)
    );
    if let Some(detail) = why.detail() {
        line.push_str(&format!(" field={detail}"));
    }
    audit.line(line);
}

pub fn accepted(audit: &mut dyn Audit, op: Op, peer_uid: Option<u32>) {
    audit.line(format!(
        "audit: accepted op={} peer_uid={}",
        op.name(),
        peer(peer_uid)
    ));
}
