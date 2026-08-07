# Storm typed properties — implementation doc (v0.1)

Scope: turn a note's YAML frontmatter from a read-only strip into an editable, typed key/value list — a key on the left, an input suited to its type on the right, and `+` to add one. Values live in the note's frontmatter; a property's *type* lives in a hidden per-vault note.

Status: **built.** M11 in `PLAN.md`, which records the decisions and what would justify revisiting them.

---

## 1. Why this is not just "make the panel editable"

The panel was read-only on purpose, and its own doc comment said why: writing values back means re-serialising the user's YAML, which reorders keys and drops comments. That is a real constraint and it has not gone away.

What changed is the conclusion. The server doesn't re-serialise either — `frontmatter.rs` splices *lines* and passes every other byte through. The client can do the same. So `lib/editor/frontmatter_edit.dart` is a second writer built on the same principle, and the panel writes through it.

**It is deliberately not a port of `set_scalars`.** That function exists to stamp `id`, `created` and `modified`, and takes four shortcuts that are correct for those and wrong for a user editing their own metadata:

| `set_scalars` | Why it fails for a properties editor |
|---|---|
| Replaces the key's single line | A `tags:` block list loses its `- item` children — invalid YAML |
| Rebuilds the block as `---\n…\n---\n` | Normalises CRLF and fence whitespace on every write |
| `format!("{key}: {value}")` | Destroys a trailing `# comment` on that line |
| No quoting at all | `title: My Note: a study` written bare breaks the document |

Each has a test in `test/frontmatter_edit_test.dart`.

## 2. The writer

`readSpans(content)` returns a `PropertySpan` per top-level key, carrying enough to rewrite it in place:

```dart
enum PropertyForm { scalar, inlineList, blockList, nested, blockScalar }

class PropertySpan {
  final String key;
  final int firstLine;          // index into the block's inner lines
  final int lastLine;           // inclusive — a block list spans several
  final PropertyForm form;
  final String rawValue;        // exactly as written, quotes included
  final List<String> items;     // list forms only
  final String? trailingComment;
  bool get isEditable;          // false for nested and blockScalar
}
```

Operations, each splicing a **range** of lines: `setScalar`, `setList`, `addProperty`, `renameKey`, `removeProperty`, `ensureBlock`.

Three rules the tests pin down:

- **Quoting is decided, not assumed.** `needsQuoting` covers `: `, a leading `#`, leading/trailing space, an empty value, YAML's booleans and nulls, and the punctuation that starts a flow collection. Inside an inline list it also covers `,` and `]`, which would otherwise end the item — the context has to be passed in, because a comma in a scalar is perfectly fine.
- **`raw: true` skips quoting** for callers that produce a valid YAML scalar by construction: a checkbox emitting `true`, a number field emitting `-5`. Without it a checkbox would write the *string* `"true"`, which reads back as text.
- **Line endings are preserved.** A CRLF note stays CRLF. The server's writer normalises, which is fine once at import and wrong on every keystroke.

**A list keeps the form it was found in.** Block stays block, inline stays inline, and block indentation is copied from the existing items. A new list is written inline.

**Nested maps and block scalars are never written.** They render read-only with a pointer to "Edit raw", which is the honest answer for YAML a key/value row cannot represent.

## 3. Types — `_storm/vault.md`

An ordinary note at a reserved path, hidden from the UI, read with the frontmatter reader that already exists:

```markdown
---
id: 8f3a…                                 ← server-stamped, as for any note
storm.description: Personal notes
storm.type.due: date
storm.type.priority: number
storm.type.status: select
storm.options.status: [todo, doing, done]
storm.type.tags: list
---

# Personal

Freeform notes about this vault.
```

A note, not a server table, so it syncs, merges, versions and backs up with everything else — no new endpoint, no wire change — and it stays greppable and hand-editable. `storm.options.<key>` also accepts a hand-written `a, b, c`, because a person is expected to edit this file.

**Types:** `text`, `number`, `checkbox`, `date`, `datetime`, `list`, `select`, `url`.

**Resolution:** a declared type wins; otherwise the type is inferred from how the value is written (ISO date → date, `[a, b]` → list, `true` → checkbox, bare number → number, `http(s)://` → url, else text). Inference is what makes a vault that has never been configured still look right.

**Hiding it:** `isVaultConfigPath` excludes `_storm/` from browse, search, recents and wikilink resolution. Server-side, `Db::count_notes` and `Indexer::all_folders` exclude it too, so a vault card does not read one note too high. The `_` in the SQL `LIKE` pattern is escaped — unescaped it is a wildcard and would also swallow a real `astorm/` folder.

## 4. Where it lives

Inside the note's scroll view, in the same 820px column as the prose, above the body. Pinned above it — where the old panel sat — it was full-width while the text was centred and capped, and it cost a phone's first screen permanently.

`NoteSession.editProperties(wholeFile)` is the mirror of `editBody`. It goes through `edit()`, so debounce, base version, merge, conflict handling and the offline outbox all work unchanged — and it **does not bump `revision`**. Only `_adopt` does that, and the body editor reacts by replacing its entire text value; a property keystroke that bumped it would reset the caret in the prose on every character. There is a test that fails if it ever does.

## 5. Non-goals

- **Property-based views.** Notion-style tables, or sorting a folder by `due`. Types are the prerequisite; the views are their own milestone.
- **A server-side schema.** The server keeps knowing nothing about property types. Values are frontmatter, types are a note.
- **Writing nested maps or block scalars.** "Edit raw" remains the escape hatch.
- **Renaming a property across every note.** Rename affects the note you are looking at.
