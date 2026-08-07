/// Per-vault configuration, stored as an ordinary markdown note.
///
/// `_storm/vault.md` carries what the UI needs to know about a vault that
/// isn't derivable from its notes: the type of each property, the options a
/// select offers, and a description.
///
/// It is a *note*, not a server table, on purpose. It syncs, merges, versions
/// and backs up with everything else — no new endpoint, no new wire format —
/// and it stays greppable and hand-editable, which is the property the whole
/// vault is built around. The cost is that it must be hidden from the places
/// that list notes; see [isVaultConfigPath].
library;

import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../api/models.dart';
import '../sync/sync_engine.dart';
import '../editor/frontmatter_edit.dart' as fme;
import 'app_state.dart';

/// Where the config note lives, vault-relative.
const kVaultConfigPath = '_storm/vault.md';

/// Everything under `_storm/` is Storm's own, and never shown as a note.
///
/// A prefix rather than a dotfile: a dotted directory is skipped by the
/// server's scanner entirely, so the file would never be indexed and the
/// client could not read it through the note API at all.
bool isVaultConfigPath(String path) =>
    path == kVaultConfigPath || path.startsWith('_storm/');

/// The kinds of value a property can hold.
///
/// Each maps to one input in the properties panel and one way of writing the
/// value back into frontmatter.
enum PropertyType {
  text('text'),
  number('number'),
  checkbox('checkbox'),
  date('date'),
  datetime('datetime'),
  list('list'),
  select('select'),
  url('url');

  const PropertyType(this.wire);

  /// How the type is spelled in `_storm/vault.md`.
  final String wire;

  static PropertyType? fromWire(String s) {
    for (final t in PropertyType.values) {
      if (t.wire == s.trim().toLowerCase()) return t;
    }
    return null;
  }

  String get label => switch (this) {
    PropertyType.text => 'Text',
    PropertyType.number => 'Number',
    PropertyType.checkbox => 'Checkbox',
    PropertyType.date => 'Date',
    PropertyType.datetime => 'Date and time',
    PropertyType.list => 'List',
    PropertyType.select => 'Select',
    PropertyType.url => 'Link',
  };
}

/// Frontmatter key prefixes inside the config note.
const _kTypePrefix = 'storm.type.';
const _kOptionsPrefix = 'storm.options.';
const _kDescription = 'storm.description';

/// A vault's property types and description, read from its config note.
class VaultConfig {
  const VaultConfig({
    this.types = const {},
    this.options = const {},
    this.description = '',
    this.raw = '',
  });

  /// Property key → declared type.
  final Map<String, PropertyType> types;

  /// Property key → the choices a `select` offers.
  final Map<String, List<String>> options;

  final String description;

  /// The config note's full content, so a write can splice it.
  final String raw;

  /// The type of [key], declared or inferred.
  ///
  /// Declared wins. Inference is what makes a vault that has never been
  /// configured still show a date picker on a date — nobody should have to
  /// set up types before the panel is useful.
  PropertyType typeOf(String key, fme.PropertySpan? span) =>
      types[key] ?? inferType(span);

  List<String> optionsFor(String key) => options[key] ?? const [];

  static VaultConfig parse(String content) {
    final types = <String, PropertyType>{};
    final options = <String, List<String>>{};
    var description = '';

    for (final span in fme.readSpans(content)) {
      final key = span.key;
      if (key == _kDescription) {
        description = span.displayValue;
      } else if (key.startsWith(_kTypePrefix)) {
        final type = PropertyType.fromWire(span.displayValue);
        if (type != null) types[key.substring(_kTypePrefix.length)] = type;
      } else if (key.startsWith(_kOptionsPrefix)) {
        // Written as a list normally, but a hand-edited `a, b, c` should work
        // too — this file is meant to be editable by a person.
        final choices = span.items.isNotEmpty
            ? span.items
            : span.displayValue.split(',').map((s) => s.trim());
        options[key.substring(_kOptionsPrefix.length)] = [
          for (final c in choices)
            if (c.isNotEmpty) c,
        ];
      }
    }
    return VaultConfig(
      types: types,
      options: options,
      description: description,
      raw: content,
    );
  }

  /// The config note's content with [key] declared as [type].
  String withType(String key, PropertyType type) =>
      fme.setScalar(raw, '$_kTypePrefix$key', type.wire);

  /// The config note's content with [key]'s select options replaced.
  String withOptions(String key, List<String> choices) =>
      fme.setList(raw, '$_kOptionsPrefix$key', choices);

  String withDescription(String text) =>
      fme.setScalar(raw, _kDescription, text);
}

/// Guesses a property's type from how its value is written.
///
/// Only ever a fallback — a declared type always wins. Deliberately
/// conservative: anything it cannot recognise is text, because text is the
/// one type that can represent every other type's value without losing it.
PropertyType inferType(fme.PropertySpan? span) {
  if (span == null) return PropertyType.text;

  switch (span.form) {
    case fme.PropertyForm.inlineList:
    case fme.PropertyForm.blockList:
      return PropertyType.list;
    case fme.PropertyForm.nested:
    case fme.PropertyForm.blockScalar:
      return PropertyType.text;
    case fme.PropertyForm.scalar:
      break;
  }

  final value = span.displayValue.trim();
  if (value.isEmpty) return PropertyType.text;

  final lower = value.toLowerCase();
  if (lower == 'true' || lower == 'false') return PropertyType.checkbox;
  if (_datetime.hasMatch(value)) return PropertyType.datetime;
  if (_date.hasMatch(value)) return PropertyType.date;
  if (lower.startsWith('http://') || lower.startsWith('https://')) {
    return PropertyType.url;
  }
  if (num.tryParse(value) != null) return PropertyType.number;
  return PropertyType.text;
}

final _date = RegExp(r'^\d{4}-\d{2}-\d{2}$');
final _datetime = RegExp(r'^\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}');

/// The active vault's configuration.
///
/// Absent config is not an error: a vault with no `_storm/vault.md` yields an
/// empty [VaultConfig] and everything falls back to inference.
final vaultConfigProvider = FutureProvider<VaultConfig>((ref) async {
  ref.watch(activeVaultProvider);
  ref.watch(vaultRevisionProvider);

  final notes = ref.watch(treeProvider).value ?? const <NoteMeta>[];
  final meta = notes.where((n) => n.path == kVaultConfigPath).firstOrNull;
  if (meta == null) return const VaultConfig();

  final cached = await ref.read(syncEngineProvider).openNote(meta.id);
  return VaultConfig.parse(cached?.content ?? '');
});

/// Writes the config note, creating it on first use.
///
/// Saved through the ordinary note path, so it merges and queues offline like
/// anything else. Returns false when the server refused — a type that could
/// not be recorded should not be shown as recorded.
Future<bool> saveVaultConfig(WidgetRef ref, String content) async {
  final engine = ref.read(syncEngineProvider);
  final notes = ref.read(treeProvider).value ?? const <NoteMeta>[];
  final existing = notes.where((n) => n.path == kVaultConfigPath).firstOrNull;

  if (existing == null) {
    final created = await engine.create(
      path: kVaultConfigPath,
      content: content.isEmpty ? _seed : content,
    );
    if (created.meta == null) return false;
  } else {
    final held = await engine.openNote(existing.id);
    final outcome = await engine.save(
      id: existing.id,
      baseVersion: held?.version ?? existing.version,
      content: content,
    );
    if (outcome.status == SaveStatus.failed) return false;
  }

  ref.read(vaultRevisionProvider.notifier).state++;
  ref.invalidate(treeProvider);
  ref.invalidate(vaultConfigProvider);
  return true;
}

/// What a brand-new config note starts as.
const _seed = '---\n---\n\n# Vault\n';
