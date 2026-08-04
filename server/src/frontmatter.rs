//! YAML frontmatter reading and *surgical* rewriting.
//!
//! The hard requirement here is that we never reformat a user's frontmatter.
//! A real Obsidian vault carries arbitrary keys, hand-chosen key order,
//! comments, and inconsistent quoting. Round-tripping through a YAML
//! serializer would normalize all of that, rewriting effectively every file in
//! the vault on first scan and producing a colossal, untrustworthy diff.
//!
//! So this module never serializes YAML. It reads the few keys Storm cares
//! about with a deliberately small scalar reader, and writes by replacing or
//! inserting individual *lines*. Every byte we don't own is passed through
//! untouched.

/// The frontmatter block of a note, if it has one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frontmatter {
    /// Bytes between the opening and closing `---` fences, fences excluded.
    pub raw: String,
    /// Byte offset in the source where the block starts (the opening `-`).
    pub start: usize,
    /// Byte offset in the source just past the closing fence's newline.
    pub end: usize,
}

/// A note split into its frontmatter and body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedNote {
    pub frontmatter: Option<Frontmatter>,
    /// Byte offset where the body begins.
    pub body_start: usize,
}

/// Splits [content] into frontmatter and body.
///
/// A block only counts when `---` is the very first line, matching Obsidian.
/// An unterminated block is not frontmatter — treating it as such would
/// swallow the whole note.
pub fn parse(content: &str) -> ParsedNote {
    let Some(rest) = strip_fence_line(content) else {
        return ParsedNote { frontmatter: None, body_start: 0 };
    };
    let open_len = content.len() - rest.len();

    // Find a closing `---` on a line of its own.
    let mut offset = open_len;
    for line in rest.split_inclusive('\n') {
        if is_fence(line) {
            let end = offset + line.len();
            return ParsedNote {
                frontmatter: Some(Frontmatter {
                    raw: content[open_len..offset].to_string(),
                    start: 0,
                    end,
                }),
                body_start: end,
            };
        }
        offset += line.len();
    }

    ParsedNote { frontmatter: None, body_start: 0 }
}

/// Returns the remainder after an opening fence line, or `None`.
fn strip_fence_line(content: &str) -> Option<&str> {
    let line = content.split_inclusive('\n').next()?;
    is_fence(line).then(|| &content[line.len()..])
}

/// True when a line is exactly `---`, ignoring trailing whitespace and EOL.
fn is_fence(line: &str) -> bool {
    line.trim_end_matches(['\n', '\r']).trim_end() == "---"
}

/// Reads a top-level scalar value, e.g. `id: abc` -> `Some("abc")`.
///
/// Only looks at column-0 keys and skips over block scalars, so a `key:`-
/// looking line inside a `description: |` block can't be mistaken for a real
/// key. Quotes are stripped; anything more exotic is out of scope because
/// Storm only ever reads keys it wrote itself.
pub fn get_scalar(raw: &str, key: &str) -> Option<String> {
    line_of_key(raw, key).map(|(_, line)| {
        let value = line[key.len() + 1..].trim();
        unquote(value).to_string()
    })
}

/// Reads `tags:` in either inline (`[a, b]`) or block (`- a`) form.
pub fn get_tags(raw: &str) -> Vec<String> {
    let Some((idx, line)) = line_of_key(raw, "tags") else {
        return Vec::new();
    };
    let inline = line["tags:".len()..].trim();

    if inline.starts_with('[') {
        return inline
            .trim_start_matches('[')
            .trim_end_matches(']')
            .split(',')
            .map(|t| unquote(t.trim()).to_string())
            .filter(|t| !t.is_empty())
            .collect();
    }

    if !inline.is_empty() {
        // `tags: single` — a bare scalar.
        return vec![unquote(inline).to_string()];
    }

    // Block form: indented `- item` lines until the indent ends.
    raw.lines()
        .skip(idx + 1)
        .take_while(|l| l.starts_with(char::is_whitespace) || l.trim_start().starts_with('-'))
        .filter_map(|l| {
            let t = l.trim();
            t.strip_prefix('-').map(|v| unquote(v.trim()).to_string())
        })
        .filter(|t| !t.is_empty())
        .collect()
}

/// Finds the (line index, line) of a top-level `key:` in a frontmatter block.
fn line_of_key<'a>(raw: &'a str, key: &str) -> Option<(usize, &'a str)> {
    let mut block_scalar_indent: Option<usize> = None;

    for (idx, line) in raw.lines().enumerate() {
        let indent = line.len() - line.trim_start().len();

        // Inside a block scalar (`key: |`), skip everything more indented.
        if let Some(base) = block_scalar_indent {
            if line.trim().is_empty() || indent > base {
                continue;
            }
            block_scalar_indent = None;
        }

        if indent != 0 {
            continue;
        }
        let trimmed = line.trim_end();
        if trimmed.ends_with('|') || trimmed.ends_with('>') {
            block_scalar_indent = Some(indent);
        }
        if line.strip_prefix(key).is_some_and(|rest| rest.starts_with(':')) {
            return Some((idx, line));
        }
    }
    None
}

fn unquote(s: &str) -> &str {
    let bytes = s.as_bytes();
    if bytes.len() >= 2
        && ((bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[bytes.len() - 1] == b'\''))
    {
        &s[1..s.len() - 1]
    } else {
        s
    }
}

/// Sets top-level scalar keys, preserving every other byte of the note.
///
/// Existing keys are replaced in place so key order is retained; missing keys
/// are inserted at the top of the block. A note with no frontmatter gets one.
///
/// This is the only function that writes frontmatter, and it never runs the
/// user's YAML through a serializer.
pub fn set_scalars(content: &str, pairs: &[(&str, &str)]) -> String {
    let parsed = parse(content);

    let Some(fm) = parsed.frontmatter else {
        // No block at all — create one and keep the body verbatim.
        let mut out = String::from("---\n");
        for (k, v) in pairs {
            out.push_str(&format!("{k}: {v}\n"));
        }
        out.push_str("---\n");
        if !content.is_empty() && !content.starts_with('\n') {
            out.push('\n');
        }
        out.push_str(content);
        return out;
    };

    let mut lines: Vec<String> = fm.raw.lines().map(str::to_string).collect();
    // Where the next missing key gets inserted. Advancing this keeps inserted
    // keys in the order they were passed rather than reversing them.
    let mut insert_at = 0usize;

    for (key, value) in pairs {
        // Recomputed against the *current* lines, not the original text: an
        // earlier insert shifts every index after it, and using a stale index
        // here would overwrite the wrong line.
        let current = lines.join("\n");
        match line_of_key(&current, key) {
            Some((idx, _)) => lines[idx] = format!("{key}: {value}"),
            None => {
                lines.insert(insert_at.min(lines.len()), format!("{key}: {value}"));
                insert_at += 1;
            }
        }
    }

    let mut out = String::with_capacity(content.len() + 64);
    out.push_str("---\n");
    for line in &lines {
        out.push_str(line);
        out.push('\n');
    }
    out.push_str("---\n");
    out.push_str(&content[fm.end..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const MESSY: &str = "---\n\
        title: \"My Note: a study\"\n\
        aliases:\n\
        \x20 - one\n\
        \x20 - two\n\
        # a comment the user wrote\n\
        cssclass: wide\n\
        tags: [homelab, project]\n\
        publish: false\n\
        ---\n\
        \n\
        # Heading\n\
        \n\
        Body text.\n";

    #[test]
    fn parses_a_block() {
        let p = parse(MESSY);
        let fm = p.frontmatter.unwrap();
        assert!(fm.raw.contains("cssclass: wide"));
        assert_eq!(&MESSY[p.body_start..], "\n# Heading\n\nBody text.\n");
    }

    #[test]
    fn no_frontmatter() {
        let p = parse("# Just a heading\n");
        assert!(p.frontmatter.is_none());
        assert_eq!(p.body_start, 0);
    }

    #[test]
    fn unterminated_block_is_not_frontmatter() {
        // Otherwise we would swallow the entire note as metadata.
        let p = parse("---\nid: abc\n\n# Heading\n");
        assert!(p.frontmatter.is_none());
    }

    #[test]
    fn fence_must_be_the_first_line() {
        let p = parse("\n---\nid: abc\n---\n");
        assert!(p.frontmatter.is_none());
    }

    #[test]
    fn horizontal_rule_in_body_is_not_a_fence() {
        let c = "# Title\n\nsome text\n\n---\n\nmore text\n";
        assert!(parse(c).frontmatter.is_none());
    }

    #[test]
    fn reads_scalars_and_tags() {
        let fm = parse(MESSY).frontmatter.unwrap();
        assert_eq!(get_scalar(&fm.raw, "title").unwrap(), "My Note: a study");
        assert_eq!(get_scalar(&fm.raw, "cssclass").unwrap(), "wide");
        assert_eq!(get_tags(&fm.raw), vec!["homelab", "project"]);
    }

    #[test]
    fn reads_block_form_tags() {
        let raw = "tags:\n  - alpha\n  - beta\nother: x\n";
        assert_eq!(get_tags(raw), vec!["alpha", "beta"]);
    }

    #[test]
    fn ignores_keys_inside_block_scalars() {
        // `id:` here is prose, not a key. Mistaking it would corrupt the note.
        let raw = "description: |\n  id: not-a-real-key\n  more text\nreal: yes\n";
        assert_eq!(get_scalar(raw, "id"), None);
        assert_eq!(get_scalar(raw, "real").unwrap(), "yes");
    }

    #[test]
    fn inserting_id_preserves_every_other_byte() {
        let out = set_scalars(MESSY, &[("id", "abc-123")]);
        assert!(out.starts_with("---\nid: abc-123\ntitle:"));
        // Everything the user wrote survives verbatim, comment included.
        assert!(out.contains("# a comment the user wrote"));
        assert!(out.contains("title: \"My Note: a study\""));
        assert!(out.contains("aliases:\n  - one\n  - two"));
        assert!(out.contains("tags: [homelab, project]"));
        assert!(out.ends_with("\n# Heading\n\nBody text.\n"));
    }

    #[test]
    fn replacing_a_key_keeps_its_position() {
        let src = "---\na: 1\nid: old\nz: 26\n---\nbody\n";
        let out = set_scalars(src, &[("id", "new")]);
        assert_eq!(out, "---\na: 1\nid: new\nz: 26\n---\nbody\n");
    }

    #[test]
    fn creates_a_block_when_absent() {
        let out = set_scalars("# Heading\n\ntext\n", &[("id", "x")]);
        assert_eq!(out, "---\nid: x\n---\n\n# Heading\n\ntext\n");
    }

    #[test]
    fn creates_a_block_for_an_empty_note() {
        assert_eq!(set_scalars("", &[("id", "x")]), "---\nid: x\n---\n");
    }

    #[test]
    fn round_trips_when_nothing_changes() {
        // Rewriting a key to its current value must be a byte-identical no-op,
        // or every scan would dirty every file.
        let src = "---\nid: keep\nmodified: t1\nother: v\n---\nbody\n";
        let out = set_scalars(src, &[("id", "keep"), ("modified", "t1")]);
        assert_eq!(out, src);
    }

    #[test]
    fn inserted_keys_keep_the_order_they_were_given() {
        let out = set_scalars(
            "---\nexisting: v\n---\nbody\n",
            &[("id", "i"), ("created", "c"), ("modified", "m")],
        );
        assert_eq!(out, "---\nid: i\ncreated: c\nmodified: m\nexisting: v\n---\nbody\n");
    }

    #[test]
    fn inserting_one_key_does_not_clobber_another() {
        // Regression: indices were computed against the original text while the
        // line vector was being mutated, so inserting `id` shifted `modified`
        // down and the next write landed on the `id` line, destroying it.
        let src = "---\nmodified: old\ntitle: keep me\n---\nbody\n";
        let out = set_scalars(src, &[("id", "new-id"), ("modified", "new-time")]);

        assert_eq!(
            out,
            "---\nid: new-id\nmodified: new-time\ntitle: keep me\n---\nbody\n"
        );
        assert!(out.contains("title: keep me"));
    }

    #[test]
    fn is_idempotent() {
        let once = set_scalars(MESSY, &[("id", "abc"), ("modified", "t")]);
        let twice = set_scalars(&once, &[("id", "abc"), ("modified", "t")]);
        assert_eq!(once, twice);
    }
}
