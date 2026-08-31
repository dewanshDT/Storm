//! Seeds the `kit` vault on a server's first run.
//!
//! `kit` holds the reusable agent tooling — the project layout spec and the
//! agent role definitions in `kit/vault/` at the repository root. It is core to
//! using Storm with a coding agent rather than an optional extra, so the server
//! creates it rather than leaving it as a setup step nobody performs.
//!
//! **Only on a true first run**, keyed off the registry file being absent
//! rather than off `kit` being missing. Someone who deletes the vault has said
//! something; recreating it every boot would be arguing with them. Once seeded
//! the copy is theirs — the [`seed`] early return, not any per-file check, is
//! what protects it.
//!
//! **Known limitation.** Because the gate is the registry file, a seed that
//! fails partway leaves an incomplete vault that no later boot repairs — it is
//! indistinguishable from a vault the user pruned deliberately. Rare (it needs
//! a write to fail mid-run) and non-corrupting, and the warning the serve path
//! logs says how to retry. Distinguishing the two cases would need a "seeded"
//! marker in the registry, which is not worth a schema field for this.
//!
//! Templates are embedded at compile time, so the canonical copy is the one
//! people read on GitHub and there is no runtime path to get wrong. Adding a
//! note means adding a line to [`TEMPLATES`]; a typo in a path is a build
//! error rather than a server that boots with a vault half full.

use std::path::Path;

use anyhow::{Context, Result};

use crate::registry::Registry;

/// The vault's name, and so also its directory under the storage root.
pub const VAULT_NAME: &str = "kit";

/// `(path within the vault, contents)`. Folders are created as needed.
const TEMPLATES: &[(&str, &str)] = &[
    ("README.md", include_str!("../../../kit/vault/README.md")),
    (
        "Project Architecture Guidelines.md",
        include_str!("../../../kit/vault/Project Architecture Guidelines.md"),
    ),
    (
        "agents/Storm Architect.md",
        include_str!("../../../kit/vault/agents/Storm Architect.md"),
    ),
    (
        "agents/Storm Lead.md",
        include_str!("../../../kit/vault/agents/Storm Lead.md"),
    ),
    (
        "agents/Storm Coder.md",
        include_str!("../../../kit/vault/agents/Storm Coder.md"),
    ),
    (
        "agents/Storm Researcher.md",
        include_str!("../../../kit/vault/agents/Storm Researcher.md"),
    ),
    (
        "agents/Storm Reviewer.md",
        include_str!("../../../kit/vault/agents/Storm Reviewer.md"),
    ),
];

/// Creates and fills the `kit` vault, registering it.
///
/// Returns `false` without touching anything when a vault already occupies the
/// `kit` directory — so calling this twice is harmless, and a server upgraded
/// into this behaviour does not clobber a vault the user made themselves.
///
/// The caller decides *when* to call: [`seed`] is safe on any boot, but the
/// serve path only calls it on a first run so that a deliberate deletion
/// sticks.
pub fn seed(registry: &mut Registry, now: &str) -> Result<bool> {
    if registry.by_dir(VAULT_NAME).is_some() {
        return Ok(false);
    }

    let entry = registry
        .create(VAULT_NAME, now)
        .context("creating the kit vault")?;
    let dir = registry.path_of(&entry);

    for (rel, body) in TEMPLATES {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        std::fs::write(&path, body).with_context(|| format!("writing {}", path.display()))?;
    }

    Ok(true)
}

/// Whether this is a first run — no registry file has been written yet.
///
/// Distinct from "the registry has no vaults": a server whose vaults were all
/// removed still has a registry, and has already had its chance to be seeded.
pub fn is_first_run(state_dir: &Path) -> bool {
    !state_dir.join(crate::registry::REGISTRY_FILE).exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry_at(root: &Path) -> Registry {
        let state = root.join("state");
        std::fs::create_dir_all(&state).unwrap();
        Registry::load(&state, root).unwrap()
    }

    #[test]
    fn seeds_every_template_into_a_named_vault() {
        let dir = tempdir::TempDir::new("storm-kit").unwrap();
        let root = dir.path().join("vaults");
        std::fs::create_dir_all(&root).unwrap();
        let mut registry = registry_at(&root);

        assert!(seed(&mut registry, "now").unwrap());

        let entry = registry.by_dir(VAULT_NAME).expect("kit registered").clone();
        assert_eq!(entry.name, VAULT_NAME);

        let vault_dir = registry.path_of(&entry);
        for (rel, body) in TEMPLATES {
            let path = vault_dir.join(rel);
            assert!(path.exists(), "{rel} was not written");
            assert_eq!(
                std::fs::read_to_string(&path).unwrap(),
                *body,
                "{rel} does not match its embedded template"
            );
        }
    }

    #[test]
    fn the_five_roles_are_all_present() {
        // Guards against a role being added to the repo and forgotten here —
        // the vault would seed with a set the docs do not describe.
        for role in [
            "Storm Architect",
            "Storm Lead",
            "Storm Coder",
            "Storm Researcher",
            "Storm Reviewer",
        ] {
            let want = format!("agents/{role}.md");
            assert!(
                TEMPLATES.iter().any(|(rel, _)| *rel == want),
                "{role} is missing from TEMPLATES"
            );
        }
    }

    #[test]
    fn a_second_call_changes_nothing() {
        let dir = tempdir::TempDir::new("storm-kit").unwrap();
        let root = dir.path().join("vaults");
        std::fs::create_dir_all(&root).unwrap();
        let mut registry = registry_at(&root);

        assert!(seed(&mut registry, "now").unwrap());
        let before = registry.vaults.len();

        // Edited by the user — seeding again must not restore the original.
        let entry = registry.by_dir(VAULT_NAME).unwrap().clone();
        let readme = registry.path_of(&entry).join("README.md");
        std::fs::write(&readme, "mine now").unwrap();

        assert!(!seed(&mut registry, "later").unwrap());
        assert_eq!(registry.vaults.len(), before);
        assert_eq!(std::fs::read_to_string(&readme).unwrap(), "mine now");
    }

    #[test]
    fn first_run_is_the_registry_file_not_an_empty_vault_list() {
        let dir = tempdir::TempDir::new("storm-kit-first-run").unwrap();
        let state = dir.path();
        assert!(is_first_run(state));

        std::fs::write(
            state.join(crate::registry::REGISTRY_FILE),
            r#"{"root":"","vaults":[]}"#,
        )
        .unwrap();

        // No vaults, but the server has booted before: it had its chance, and a
        // deleted kit stays deleted.
        assert!(!is_first_run(state));
    }
}
