//! Extracts the structure Storm indexes: wikilinks, inline tags, and a title.
//!
//! Runs on every write, so it is a single linear scan with no regex engine and
//! no allocation beyond the results.
//!
//! Code is excluded deliberately. A note explaining a shell script should not
//! put `#!/bin/sh` in the tag index, and a Rust snippet with `[[a]]` is not a
//! wikilink. Both fenced blocks and inline spans are skipped.

use std::collections::BTreeSet;

use crate::frontmatter;

/// Everything the index needs from a note's content.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Extracted {
    /// Wikilink targets, `[[Target]]` or `[[Target|alias]]`, deduped and sorted.
    pub links: Vec<String>,
    /// Tags from both `#inline` and frontmatter `tags:`, deduped and sorted.
    pub tags: Vec<String>,
    /// First `# Heading` in the body, if any.
    pub heading: Option<String>,
}

/// Parses a full note (frontmatter included).
pub fn extract(content: &str) -> Extracted {
    let parsed = frontmatter::parse(content);

    let mut tags: BTreeSet<String> = parsed
        .frontmatter
        .as_ref()
        .map(|fm| frontmatter::get_tags(&fm.raw))
        .unwrap_or_default()
        .into_iter()
        .collect();

    let body = &content[parsed.body_start..];
    let mut links = BTreeSet::new();
    let mut heading = None;

    let mut in_fence = false;
    for line in body.lines() {
        let trimmed = line.trim_start();

        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        // Indented code blocks.
        if line.starts_with("    ") || line.starts_with('\t') {
            continue;
        }

        if heading.is_none() {
            heading = trimmed.strip_prefix("# ").map(|r| r.trim().to_string());
        }

        scan_line(line, &mut links, &mut tags);
    }

    Extracted {
        links: links.into_iter().collect(),
        tags: tags.into_iter().collect(),
        heading,
    }
}

/// Scans one line, skipping inline code spans.
fn scan_line(line: &str, links: &mut BTreeSet<String>, tags: &mut BTreeSet<String>) {
    let bytes = line.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        match bytes[i] {
            // Inline code: skip to the closing backtick.
            b'`' => {
                i += 1;
                while i < bytes.len() && bytes[i] != b'`' {
                    i += 1;
                }
                i += 1;
            }

            // Wikilink.
            b'[' if bytes.get(i + 1) == Some(&b'[') => {
                let start = i + 2;
                let Some(close) = find(bytes, start, b"]]") else {
                    i += 2;
                    continue;
                };
                let inner = &line[start..close];
                // `[[Target|alias]]` and `[[Target#heading]]` both index Target.
                let target = inner
                    .split('|')
                    .next()
                    .unwrap_or(inner)
                    .split('#')
                    .next()
                    .unwrap_or(inner)
                    .trim();
                if !target.is_empty() {
                    links.insert(target.to_string());
                }
                i = close + 2;
            }

            // Inline tag. Must follow a boundary, so `foo#bar` and URL
            // fragments don't register.
            b'#' => {
                let preceded_ok = i == 0
                    || matches!(
                        bytes[i - 1],
                        b' ' | b'\t' | b'(' | b'[' | b'{' | b'>' | b',' | b';' | b'"' | b'\''
                    );
                if !preceded_ok {
                    i += 1;
                    continue;
                }
                let start = i + 1;
                let mut end = start;
                while end < bytes.len() && is_tag_byte(bytes[end]) {
                    end += 1;
                }
                // Require at least one non-digit, so `#1` stays an issue number.
                let tag = &line[start..end];
                if !tag.is_empty() && !tag.bytes().all(|b| b.is_ascii_digit()) {
                    tags.insert(tag.trim_end_matches('/').to_string());
                }
                i = end.max(i + 1);
            }

            _ => i += 1,
        }
    }
}

fn is_tag_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'/') || b >= 0x80
}

fn find(haystack: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    (from..haystack.len().saturating_sub(needle.len() - 1))
        .find(|&i| &haystack[i..i + needle.len()] == needle)
}

/// Derives a display title: the note's first `# Heading`, else its file stem.
pub fn title_for(content: &str, path: &str) -> String {
    if let Some(h) = extract(content).heading {
        return h;
    }
    std::path::Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "Untitled".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_links_tags_and_heading() {
        let note = "---\ntags: [from-frontmatter]\n---\n\n\
            # The Title\n\n\
            See [[Other Note]] and [[Folder/Deep Note|an alias]]. #homelab #proj/sub\n";
        let e = extract(note);
        assert_eq!(e.heading.as_deref(), Some("The Title"));
        assert_eq!(e.links, vec!["Folder/Deep Note", "Other Note"]);
        assert_eq!(e.tags, vec!["from-frontmatter", "homelab", "proj/sub"]);
    }

    #[test]
    fn ignores_fenced_code() {
        let note = "# T\n\n```sh\n#!/bin/sh\n# not a tag\necho '[[not a link]]'\n```\n\n#real\n";
        let e = extract(note);
        assert_eq!(e.tags, vec!["real"]);
        assert!(e.links.is_empty());
    }

    #[test]
    fn ignores_inline_code() {
        let e = extract("Use `#define` and `[[array]]` carefully. #actual\n");
        assert_eq!(e.tags, vec!["actual"]);
        assert!(e.links.is_empty());
    }

    #[test]
    fn ignores_indented_code() {
        let e = extract("Text\n\n    #indented-not-a-tag\n\n#yes\n");
        assert_eq!(e.tags, vec!["yes"]);
    }

    #[test]
    fn does_not_treat_url_fragments_as_tags() {
        let e = extract("See https://example.com/page#section for detail.\n");
        assert!(e.tags.is_empty());
    }

    #[test]
    fn does_not_treat_issue_numbers_as_tags() {
        let e = extract("Fixes #1234 and #x1\n");
        assert_eq!(e.tags, vec!["x1"]);
    }

    #[test]
    fn headings_are_not_tags() {
        // `# Title` is a heading; only `#word` with no space is a tag.
        let e = extract("# Title\n## Subtitle\n");
        assert!(e.tags.is_empty());
        assert_eq!(e.heading.as_deref(), Some("Title"));
    }

    #[test]
    fn strips_wikilink_headings_and_aliases() {
        let e = extract("[[Note#Section]] and [[Note2|alias]]\n");
        assert_eq!(e.links, vec!["Note", "Note2"]);
    }

    #[test]
    fn unterminated_wikilink_is_ignored() {
        let e = extract("text [[unclosed and more\n");
        assert!(e.links.is_empty());
    }

    #[test]
    fn dedupes() {
        let e = extract("[[A]] [[A]] #t #t\n");
        assert_eq!(e.links, vec!["A"]);
        assert_eq!(e.tags, vec!["t"]);
    }

    #[test]
    fn unicode_tags_and_links() {
        let e = extract("[[日本語ノート]] #ünïcode\n");
        assert_eq!(e.links, vec!["日本語ノート"]);
        assert_eq!(e.tags, vec!["ünïcode"]);
    }

    #[test]
    fn title_falls_back_to_the_filename() {
        assert_eq!(
            title_for("no heading here\n", "Daily/2026-08-05.md"),
            "2026-08-05"
        );
        assert_eq!(title_for("# Real Title\n", "x/y.md"), "Real Title");
    }

    #[test]
    fn frontmatter_hash_is_not_a_tag() {
        // A `#` inside frontmatter is a YAML comment, not an inline tag.
        let e = extract("---\ntitle: x\n# yaml comment\n---\n\nbody\n");
        assert!(e.tags.is_empty());
    }
}
