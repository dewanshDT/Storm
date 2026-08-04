//! Storm's conflict policy: optimistic concurrency with a 3-way text merge.
//!
//! Because the server is the single source of record, reconciliation is only
//! ever "the client's edits" against "the server's current text", with the
//! version the client started from as the merge base. That is a plain diff3 —
//! no CRDT needed, and the markdown file stays the only content store.
//!
//! When a merge genuinely conflicts we write the conflict-marked text into the
//! note rather than rejecting the write or spawning a sibling file. Nothing is
//! lost: the pre-merge server text is already in `note_versions`. The user
//! fixes it by deleting four lines, which beats hunting down Syncthing's
//! `.sync-conflict-*` copies.

/// What happened when a client's write met the server's current state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The client was up to date; its text is taken verbatim.
    FastForward(String),
    /// The client was behind but the edits didn't overlap.
    Merged(String),
    /// Both sides changed the same region; text carries conflict markers.
    Conflicted(String),
}

impl Outcome {
    pub fn text(&self) -> &str {
        match self {
            Outcome::FastForward(s) | Outcome::Merged(s) | Outcome::Conflicted(s) => s,
        }
    }

    pub fn is_conflicted(&self) -> bool {
        matches!(self, Outcome::Conflicted(_))
    }

    /// Whether the client must adopt the server's text rather than keep its own.
    pub fn rewrites_client(&self) -> bool {
        !matches!(self, Outcome::FastForward(_))
    }
}

/// Reconciles `ours` (the client's text, based on `base`) with `theirs`
/// (whatever the server holds now).
pub fn reconcile(base: &str, theirs: &str, ours: &str) -> Outcome {
    // Nobody else moved: take the client's text as-is.
    if base == theirs {
        return Outcome::FastForward(ours.to_string());
    }
    // The client changed nothing; keep the server's newer text.
    if base == ours {
        return Outcome::Merged(theirs.to_string());
    }
    // Both arrived at the same text independently.
    if theirs == ours {
        return Outcome::Merged(ours.to_string());
    }

    // Argument order sets the conflict-marker labels: diffy calls the second
    // argument "ours" and the third "theirs". The client's own text goes
    // second so that whoever opens the conflicted note sees their own edit
    // under `<<<<<<< ours` and the other device's under `>>>>>>> theirs`.
    match diffy::merge(base, ours, theirs) {
        Ok(merged) => Outcome::Merged(merged),
        Err(marked) => Outcome::Conflicted(marked),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = "# Note\n\nAlpha line.\n\nBeta line.\n\nGamma line.\n";

    #[test]
    fn fast_forwards_when_server_has_not_moved() {
        let ours = BASE.replace("Alpha", "ALPHA");
        let out = reconcile(BASE, BASE, &ours);
        assert_eq!(out, Outcome::FastForward(ours.clone()));
        assert!(!out.rewrites_client());
    }

    #[test]
    fn takes_server_text_when_client_did_not_edit() {
        let theirs = BASE.replace("Beta", "BETA");
        let out = reconcile(BASE, &theirs, BASE);
        assert_eq!(out.text(), theirs);
        assert!(!out.is_conflicted());
    }

    #[test]
    fn merges_edits_to_different_regions() {
        // Scenario 4 of the sync matrix: offline edit to one paragraph, online
        // edit to another. Both must survive with no markers.
        let theirs = BASE.replace("Alpha line.", "Alpha line, edited on desktop.");
        let ours = BASE.replace("Gamma line.", "Gamma line, edited on phone.");

        let out = reconcile(BASE, &theirs, &ours);
        assert!(!out.is_conflicted(), "got: {}", out.text());
        assert!(out.text().contains("edited on desktop"));
        assert!(out.text().contains("edited on phone"));
        assert!(!out.text().contains("<<<<<<<"));
    }

    #[test]
    fn conflicts_when_both_edit_the_same_line() {
        // Scenario 5: genuine overlap. Both versions must appear in the output.
        let theirs = BASE.replace("Beta line.", "Beta line from desktop.");
        let ours = BASE.replace("Beta line.", "Beta line from phone.");

        let out = reconcile(BASE, &theirs, &ours);
        assert!(out.is_conflicted());
        let text = out.text();
        assert!(text.contains("<<<<<<<"));
        assert!(text.contains(">>>>>>>"));
        assert!(text.contains("from desktop"));
        assert!(text.contains("from phone"));
    }

    #[test]
    fn conflict_labels_put_the_clients_own_edit_under_ours() {
        // Orientation matters for readability: the person resolving the note
        // is the one whose write just conflicted, so their text must be the
        // `ours` side, not the other device's.
        let theirs = BASE.replace("Beta line.", "OTHER DEVICE.");
        let ours = BASE.replace("Beta line.", "MY EDIT.");
        let text = reconcile(BASE, &theirs, &ours).text().to_string();

        let mine = text.find("MY EDIT.").expect("client text missing");
        let sep = text.find("=======").expect("no separator");
        let other = text.find("OTHER DEVICE.").expect("server text missing");
        assert!(mine < sep, "client's edit should appear above the separator");
        assert!(other > sep, "other device's edit should appear below");
    }

    #[test]
    fn identical_independent_edits_do_not_conflict() {
        let same = BASE.replace("Beta", "BETA");
        let out = reconcile(BASE, &same, &same);
        assert!(!out.is_conflicted());
        assert_eq!(out.text(), same);
    }

    #[test]
    fn merges_an_append_against_a_prepend() {
        let theirs = format!("Prepended.\n{BASE}");
        let ours = format!("{BASE}Appended.\n");
        let out = reconcile(BASE, &theirs, &ours);
        assert!(!out.is_conflicted(), "got: {}", out.text());
        assert!(out.text().starts_with("Prepended."));
        assert!(out.text().ends_with("Appended.\n"));
    }

    /// A longer document, so "far apart" really is far apart.
    const LONG: &str = "# Note\n\nAlpha.\n\nBeta.\n\nGamma.\n\nDelta.\n\n\
        Epsilon.\n\nZeta.\n\nEta.\n\nTheta.\n";

    #[test]
    fn handles_deletion_far_from_an_edit() {
        let theirs = LONG.replace("Beta.\n\n", "");
        let ours = LONG.replace("Theta.", "Theta edited.");
        let out = reconcile(LONG, &theirs, &ours);
        assert!(!out.is_conflicted(), "got: {}", out.text());
        assert!(!out.text().contains("Beta."));
        assert!(out.text().contains("Theta edited."));
    }

    #[test]
    fn adjacent_edits_conflict_even_though_they_do_not_overlap() {
        // Pins a real limitation of diff3, and the main behavioural cost of
        // choosing it over a CRDT: hunks are line-based with context, so
        // deleting a paragraph while another device edits the *next* one is
        // reported as a conflict. Nothing is lost — both sides appear between
        // the markers — but the user has to resolve it by hand.
        //
        // Acceptable for a single-user vault, where concurrent edits to
        // neighbouring lines of the same note are rare. If this ever becomes
        // common in practice, that is the signal to revisit `yrs`.
        let theirs = BASE.replace("Beta line.\n\n", "");
        let ours = BASE.replace("Gamma line.", "Gamma edited.");
        let out = reconcile(BASE, &theirs, &ours);

        assert!(out.is_conflicted());
        assert!(out.text().contains("Gamma edited."));
        assert!(out.text().contains("Beta line."));
    }

    #[test]
    fn never_loses_content_on_conflict() {
        // The safety property that lets us write markers straight into the file.
        let theirs = "totally different desktop text\n";
        let ours = "totally different phone text\n";
        let out = reconcile(BASE, theirs, ours);
        assert!(out.text().contains("desktop text"));
        assert!(out.text().contains("phone text"));
    }

    #[test]
    fn empty_base_is_survivable() {
        let out = reconcile("", "server\n", "client\n");
        assert!(out.text().contains("server") || out.text().contains("client"));
    }
}
