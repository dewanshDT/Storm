import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../editor/frontmatter_edit.dart' as fme;
import '../state/app_state.dart';
import '../state/vault_config.dart';
import 'accents.dart';
import 'shell/storm_scaffold.dart' show promptForPath;

/// A note's frontmatter, as an editable key/value list.
///
/// Obsidian and Notion both show metadata as a small table above the note
/// rather than as text you scroll past, and the raw `---` block in the editor
/// was the single ugliest thing about the app.
///
/// It used to be read-only, on the grounds that writing values back means
/// re-serialising the user's YAML. That reasoning had a hole in it: the server
/// doesn't re-serialise either — it splices lines. [fme] does the same, only
/// richer, so this can write.
///
/// **This list is the only way to edit frontmatter.** There is no raw-YAML
/// mode behind it: every key in the block gets a row, in file order, so
/// nothing is reachable only by falling back to editing text. Rows Storm owns
/// (`id`, `created`, `modified`) and values a key/value row cannot represent
/// (a nested map, a block scalar) are shown read-only rather than hidden —
/// visible and honest beats editable and wrong.
class NoteProperties extends ConsumerStatefulWidget {
  const NoteProperties({
    super.key,
    required this.content,
    required this.onChanged,
  });

  /// The note's whole file, frontmatter included.
  final String content;

  /// Called with the whole file after an edit. The panel never mutates
  /// anything itself; the session owns the buffer.
  final ValueChanged<String> onChanged;

  @override
  ConsumerState<NoteProperties> createState() => _NotePropertiesState();
}

class _NotePropertiesState extends ConsumerState<NoteProperties> {
  /// Fields Storm owns. The server re-asserts `id` and `modified` on every
  /// save, so offering them as editable would be a lie.
  static const _managed = {'id', 'created', 'modified'};

  VaultConfig get _config =>
      ref.watch(vaultConfigProvider).value ?? const VaultConfig();

  @override
  Widget build(BuildContext context) {
    // File order, everything in one list. Segregating Storm's own fields
    // behind a disclosure made the panel a place where some metadata lived
    // and some hid; the list should just be what the block says.
    //
    // The one exception is `id`, off by default: it is a UUID nobody reads,
    // and it cost the top row of every note. The setting turns it back on
    // rather than the value being unreachable.
    final showId = ref.watch(settingsProvider).value?.showNoteId ?? false;
    final spans = [
      for (final span in fme.readSpans(widget.content))
        if (showId || span.key != 'id') span,
    ];

    return Padding(
      // No divider. The properties run straight into the prose, which is what
      // makes it read as one document instead of two panes.
      padding: const EdgeInsets.only(bottom: 20),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          for (final span in spans)
            _PropertyRow(
              key: ValueKey(span.key),
              span: span,
              type: _config.typeOf(span.key, span),
              options: _config.optionsFor(span.key),
              readOnly: _managed.contains(span.key) || !span.isEditable,
              onValue: (value, {bool raw = false}) =>
                  _setScalar(span.key, value, raw: raw),
              onItems: (items) => _setList(span.key, items),
              onMenu: (action) => _menu(span, action),
            ),
          _AddProperty(onAdd: _add),
        ],
      ),
    );
  }

  // ---- writes ----------------------------------------------------------

  void _setScalar(String key, String value, {bool raw = false}) {
    // A colour cleared back to "Default" should leave no trace, not a bare
    // `color:` line — the note simply has no colour.
    if (key == kColorKey && value.isEmpty) {
      widget.onChanged(fme.removeProperty(widget.content, key));
      return;
    }
    widget.onChanged(fme.setScalar(widget.content, key, value, raw: raw));
  }

  void _setList(String key, List<String> items) =>
      widget.onChanged(fme.setList(widget.content, key, items));

  Future<void> _add() async {
    final name = await promptForPath(
      context,
      title: 'New property',
      initial: '',
      isFolder: true, // no `.md` suffix — this is a key, not a path
    );
    if (name == null || name.trim().isEmpty || !mounted) return;
    final key = name.trim();
    if (fme.findSpan(widget.content, key) != null) {
      _toast('“$key” is already a property of this note');
      return;
    }
    widget.onChanged(fme.setScalar(widget.content, key, ''));
  }

  Future<void> _menu(fme.PropertySpan span, _RowAction action) async {
    switch (action) {
      case _RowAction.rename:
        final name = await promptForPath(
          context,
          title: 'Rename property',
          initial: span.key,
          isFolder: true,
        );
        if (name == null || name.trim().isEmpty || !mounted) return;
        widget.onChanged(fme.renameKey(widget.content, span.key, name.trim()));
      case _RowAction.delete:
        widget.onChanged(fme.removeProperty(widget.content, span.key));
      case _RowAction.retype:
        await _pickType(span);
    }
  }

  Future<void> _pickType(fme.PropertySpan span) async {
    final current = _config.typeOf(span.key, span);
    final chosen = await showModalBottomSheet<PropertyType>(
      context: context,
      showDragHandle: true,
      builder: (c) => SafeArea(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            for (final type in PropertyType.values)
              ListTile(
                leading: Icon(_iconFor(type)),
                title: Text(type.label),
                trailing: type == current ? const Icon(Icons.check) : null,
                onTap: () => Navigator.pop(c, type),
              ),
          ],
        ),
      ),
    );
    if (chosen == null || chosen == current || !mounted) return;

    // The type lives in the vault's config note, not in this one — it applies
    // to the property everywhere it appears.
    final config = _config;
    final ok = await saveVaultConfig(ref, config.withType(span.key, chosen));
    if (!ok && mounted) _toast('Could not save the property type');
  }

  void _toast(String message) => ScaffoldMessenger.of(
    context,
  ).showSnackBar(SnackBar(content: Text(message)));
}

IconData _iconFor(PropertyType type) => switch (type) {
  PropertyType.text => Icons.notes,
  PropertyType.number => Icons.tag,
  PropertyType.checkbox => Icons.check_box_outlined,
  PropertyType.date => Icons.event,
  PropertyType.datetime => Icons.schedule,
  PropertyType.list => Icons.label_outline,
  PropertyType.select => Icons.arrow_drop_down_circle_outlined,
  PropertyType.url => Icons.link,
  PropertyType.color => Icons.palette_outlined,
};

enum _RowAction { retype, rename, delete }

/// Reports a new scalar value. [raw] means "already a valid YAML scalar" —
/// a checkbox's `true`, a number's `-5` — and skips quoting.
typedef ValueSink = void Function(String value, {bool raw});

/// One property: its key on the left, an input suited to its type on the
/// right.
class _PropertyRow extends StatelessWidget {
  const _PropertyRow({
    super.key,
    required this.span,
    required this.type,
    required this.options,
    required this.readOnly,
    required this.onValue,
    required this.onItems,
    required this.onMenu,
  });

  final fme.PropertySpan span;
  final PropertyType type;
  final List<String> options;

  /// Shown, never written: Storm's own fields, and values a key/value row
  /// cannot represent.
  final bool readOnly;

  final ValueSink onValue;
  final ValueChanged<List<String>> onItems;
  final ValueChanged<_RowAction> onMenu;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 2),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          _KeyChip(
            label: span.key,
            icon: readOnly ? Icons.lock_outline : _iconFor(type),
            enabled: !readOnly,
            onMenu: onMenu,
          ),
          const SizedBox(width: 10),
          Expanded(
            child: Padding(
              padding: const EdgeInsets.only(top: 2),
              child: readOnly
                  ? _ReadOnlyValue(span: span)
                  : _ValueEditor(
                      span: span,
                      type: type,
                      options: options,
                      onValue: onValue,
                      onItems: onItems,
                    ),
            ),
          ),
        ],
      ),
    );
  }
}

/// The rounded key button from the sketch. Tapping it opens the row's menu.
class _KeyChip extends StatelessWidget {
  const _KeyChip({
    required this.label,
    required this.icon,
    required this.enabled,
    required this.onMenu,
  });

  final String label;
  final IconData icon;
  final bool enabled;
  final ValueChanged<_RowAction> onMenu;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    final chip = Container(
      width: 132,
      padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 7),
      decoration: BoxDecoration(
        color: scheme.surfaceContainerHigh,
        borderRadius: BorderRadius.circular(9),
      ),
      child: Row(
        children: [
          Icon(icon, size: 14, color: scheme.onSurfaceVariant),
          const SizedBox(width: 7),
          Expanded(
            child: Text(
              label,
              overflow: TextOverflow.ellipsis,
              style: TextStyle(
                fontSize: 13,
                color: scheme.onSurfaceVariant,
                fontWeight: FontWeight.w500,
              ),
            ),
          ),
        ],
      ),
    );

    if (!enabled) return Opacity(opacity: 0.6, child: chip);

    return PopupMenuButton<_RowAction>(
      tooltip: 'Property actions',
      onSelected: onMenu,
      itemBuilder: (_) => const [
        PopupMenuItem(value: _RowAction.retype, child: Text('Change type')),
        PopupMenuItem(value: _RowAction.rename, child: Text('Rename')),
        PopupMenuItem(value: _RowAction.delete, child: Text('Delete')),
      ],
      child: chip,
    );
  }
}

/// The input for one property, chosen by its type.
class _ValueEditor extends StatelessWidget {
  const _ValueEditor({
    required this.span,
    required this.type,
    required this.options,
    required this.onValue,
    required this.onItems,
  });

  final fme.PropertySpan span;
  final PropertyType type;
  final List<String> options;
  final ValueSink onValue;
  final ValueChanged<List<String>> onItems;

  @override
  Widget build(BuildContext context) {
    switch (type) {
      case PropertyType.list:
        return _ListEditor(items: span.items, onChanged: onItems);
      case PropertyType.checkbox:
        return _CheckboxEditor(
          value: span.displayValue,
          // A boolean, written bare: quoting it would turn it into the
          // *string* "true", which reads back as text on the next open.
          onChanged: (v) => onValue(v, raw: true),
        );
      case PropertyType.date:
        return _DateEditor(
          value: span.displayValue,
          withTime: false,
          onChanged: (v) => onValue(v),
        );
      case PropertyType.datetime:
        return _DateEditor(
          value: span.displayValue,
          withTime: true,
          onChanged: (v) => onValue(v),
        );
      case PropertyType.select:
        return _SelectEditor(
          value: span.displayValue,
          options: options,
          onChanged: (v) => onValue(v),
        );
      case PropertyType.number:
        return _TextEditor(
          value: span.displayValue,
          // Bare, so a negative number is not quoted into a string.
          onChanged: (v) => onValue(v, raw: v.trim().isNotEmpty),
          keyboardType: const TextInputType.numberWithOptions(decimal: true),
          formatters: [FilteringTextInputFormatter.allow(RegExp(r'[-0-9.]'))],
        );
      case PropertyType.color:
        return _AccentEditor(
          value: span.displayValue,
          onChanged: (v) => onValue(v),
        );
      case PropertyType.url:
      case PropertyType.text:
        return _TextEditor(
          value: span.displayValue,
          onChanged: (v) => onValue(v),
        );
    }
  }
}

/// A row of swatches.
///
/// Colour is edited here, in the properties list, rather than through a
/// separate menu — the list is the only way frontmatter changes, and a colour
/// is frontmatter like any other value.
class _AccentEditor extends StatelessWidget {
  const _AccentEditor({required this.value, required this.onChanged});

  final String value;
  final ValueChanged<String> onChanged;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.only(top: 2, bottom: 4),
      child: AccentPicker(
        size: 26,
        selected: Accent.parse(value),
        // "Default" clears the key rather than writing `color: none`, so a
        // note that never had a colour goes back to having no colour line.
        onSelected: (accent) => onChanged(accent.isNone ? '' : accent.name),
      ),
    );
  }
}

/// A borderless field that reports on blur rather than on every keystroke.
///
/// Per-keystroke writes would splice the frontmatter on every character and
/// dirty the note continuously. Committing on blur and on submit keeps one
/// edit per change.
class _TextEditor extends StatefulWidget {
  const _TextEditor({
    required this.value,
    required this.onChanged,
    this.keyboardType,
    this.formatters,
  });

  final String value;
  final ValueChanged<String> onChanged;
  final TextInputType? keyboardType;
  final List<TextInputFormatter>? formatters;

  @override
  State<_TextEditor> createState() => _TextEditorState();
}

class _TextEditorState extends State<_TextEditor> {
  late final TextEditingController _controller = TextEditingController(
    text: widget.value,
  );
  late final FocusNode _focus = FocusNode()..addListener(_onFocus);

  @override
  void didUpdateWidget(_TextEditor old) {
    super.didUpdateWidget(old);
    // Adopt an external change only while the user is not typing into it.
    if (!_focus.hasFocus && widget.value != _controller.text) {
      _controller.text = widget.value;
    }
  }

  void _onFocus() {
    if (!_focus.hasFocus) _commit();
  }

  void _commit() {
    if (_controller.text != widget.value) widget.onChanged(_controller.text);
  }

  @override
  void dispose() {
    _focus.removeListener(_onFocus);
    _focus.dispose();
    _controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return TextField(
      controller: _controller,
      focusNode: _focus,
      keyboardType: widget.keyboardType,
      inputFormatters: widget.formatters,
      style: Theme.of(context).textTheme.bodyMedium,
      decoration: const InputDecoration(
        isDense: true,
        border: InputBorder.none,
        contentPadding: EdgeInsets.zero,
        hintText: 'Empty',
      ),
      onSubmitted: (_) => _commit(),
      onTapOutside: (_) => _focus.unfocus(),
    );
  }
}

class _CheckboxEditor extends StatelessWidget {
  const _CheckboxEditor({required this.value, required this.onChanged});

  final String value;
  final ValueChanged<String> onChanged;

  @override
  Widget build(BuildContext context) {
    final checked = value.trim().toLowerCase() == 'true';
    return SizedBox(
      height: 24,
      child: Align(
        alignment: Alignment.centerLeft,
        child: Checkbox(
          value: checked,
          visualDensity: VisualDensity.compact,
          materialTapTargetSize: MaterialTapTargetSize.shrinkWrap,
          onChanged: (next) => onChanged((next ?? false).toString()),
        ),
      ),
    );
  }
}

/// A date, written as ISO so it sorts and greps.
class _DateEditor extends StatelessWidget {
  const _DateEditor({
    required this.value,
    required this.withTime,
    required this.onChanged,
  });

  final String value;
  final bool withTime;
  final ValueChanged<String> onChanged;

  @override
  Widget build(BuildContext context) {
    final parsed = DateTime.tryParse(value.trim());
    final label = parsed == null
        ? (value.trim().isEmpty ? 'Empty' : value)
        : _format(parsed);

    return Align(
      alignment: Alignment.centerLeft,
      child: InkWell(
        onTap: () => _pick(context, parsed),
        borderRadius: BorderRadius.circular(6),
        child: Padding(
          padding: const EdgeInsets.symmetric(vertical: 2, horizontal: 2),
          child: Text(
            label,
            style: Theme.of(context).textTheme.bodyMedium?.copyWith(
              color: parsed == null && value.trim().isEmpty
                  ? Theme.of(context).hintColor
                  : null,
            ),
          ),
        ),
      ),
    );
  }

  String _format(DateTime d) {
    if (!withTime) return formatDay(d);
    final hh = d.hour.toString().padLeft(2, '0');
    final mm = d.minute.toString().padLeft(2, '0');
    return '${formatDay(d)}, $hh:$mm';
  }

  Future<void> _pick(BuildContext context, DateTime? current) async {
    final now = DateTime.now();
    final day = await showDatePicker(
      context: context,
      initialDate: current ?? now,
      firstDate: DateTime(now.year - 50),
      lastDate: DateTime(now.year + 50),
    );
    if (day == null || !context.mounted) return;

    if (!withTime) {
      onChanged(_isoDate(day));
      return;
    }
    final time = await showTimePicker(
      context: context,
      initialTime: TimeOfDay.fromDateTime(current ?? now),
    );
    final at = time ?? TimeOfDay.fromDateTime(current ?? now);
    final hh = at.hour.toString().padLeft(2, '0');
    final mm = at.minute.toString().padLeft(2, '0');
    onChanged('${_isoDate(day)}T$hh:$mm');
  }

  static String _isoDate(DateTime d) =>
      '${d.year.toString().padLeft(4, '0')}-'
      '${d.month.toString().padLeft(2, '0')}-'
      '${d.day.toString().padLeft(2, '0')}';
}

/// Renders a timestamp for reading, or returns it unchanged.
///
/// Anything that is not a date passes straight through: this runs on every
/// read-only value, and a key whose value merely resembles a date should not
/// be quietly rewritten on screen.
String formatStamp(String value) {
  final parsed = DateTime.tryParse(value.trim());
  if (parsed == null) return value;
  final local = parsed.toLocal();
  final hh = local.hour.toString().padLeft(2, '0');
  final mm = local.minute.toString().padLeft(2, '0');
  return '${formatDay(local)}, $hh:$mm';
}

/// `7 Aug 2026`.
String formatDay(DateTime d) => '${d.day} ${_months[d.month - 1]} ${d.year}';

bool _looksLikeStamp(String value) => DateTime.tryParse(value.trim()) != null;

const _months = [
  'Jan',
  'Feb',
  'Mar',
  'Apr',
  'May',
  'Jun',
  'Jul',
  'Aug',
  'Sep',
  'Oct',
  'Nov',
  'Dec',
];

class _SelectEditor extends StatelessWidget {
  const _SelectEditor({
    required this.value,
    required this.options,
    required this.onChanged,
  });

  final String value;
  final List<String> options;
  final ValueChanged<String> onChanged;

  @override
  Widget build(BuildContext context) {
    // The current value counts as an option even when the vault has not been
    // told about it — otherwise editing a note would silently blank a value
    // somebody typed by hand.
    final choices = <String>{
      ...options,
      if (value.trim().isNotEmpty) value.trim(),
    }.toList();

    if (choices.isEmpty) {
      return _TextEditor(value: value, onChanged: onChanged);
    }

    return Align(
      alignment: Alignment.centerLeft,
      child: DropdownButton<String>(
        value: value.trim().isEmpty ? null : value.trim(),
        hint: const Text('Empty'),
        underline: const SizedBox.shrink(),
        isDense: true,
        style: Theme.of(context).textTheme.bodyMedium,
        items: [
          for (final option in choices)
            DropdownMenuItem(value: option, child: Text(option)),
        ],
        onChanged: (next) => onChanged(next ?? ''),
      ),
    );
  }
}

/// Chips, then an outlined `+` badge that becomes an editable badge in place.
///
/// The add affordance is a badge rather than a bare text field beside the
/// chips: it lines up with what it creates, and an always-present input made
/// the row look unfinished on a phone.
class _ListEditor extends StatefulWidget {
  const _ListEditor({required this.items, required this.onChanged});

  final List<String> items;
  final ValueChanged<List<String>> onChanged;

  @override
  State<_ListEditor> createState() => _ListEditorState();
}

class _ListEditorState extends State<_ListEditor> {
  bool _adding = false;

  void _commit(String value) {
    final trimmed = value.trim();
    if (trimmed.isNotEmpty && !widget.items.contains(trimmed)) {
      widget.onChanged([...widget.items, trimmed]);
    }
    // Stay open after a successful entry: adding tags comes in runs.
    setState(() => _adding = trimmed.isNotEmpty);
  }

  @override
  Widget build(BuildContext context) {
    return Wrap(
      spacing: 6,
      runSpacing: 6,
      crossAxisAlignment: WrapCrossAlignment.center,
      children: [
        for (final item in widget.items)
          ValueChip(
            label: item,
            onRemove: () =>
                widget.onChanged(widget.items.where((i) => i != item).toList()),
          ),
        if (_adding)
          _NewValueChip(
            onSubmit: _commit,
            onCancel: () => setState(() => _adding = false),
          )
        else
          _AddValueChip(onTap: () => setState(() => _adding = true)),
      ],
    );
  }
}

/// An empty badge you type into. Same size and shape as the chip it becomes.
class _NewValueChip extends StatefulWidget {
  const _NewValueChip({required this.onSubmit, required this.onCancel});

  final ValueChanged<String> onSubmit;
  final VoidCallback onCancel;

  @override
  State<_NewValueChip> createState() => _NewValueChipState();
}

class _NewValueChipState extends State<_NewValueChip> {
  final _controller = TextEditingController();
  final _focus = FocusNode();

  @override
  void initState() {
    super.initState();
    _focus.addListener(_onFocus);
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) _focus.requestFocus();
    });
  }

  void _onFocus() {
    // Tapping away from an empty badge cancels rather than leaving a stray
    // outline behind.
    if (!_focus.hasFocus && _controller.text.trim().isEmpty) {
      widget.onCancel();
    }
  }

  @override
  void dispose() {
    _focus.removeListener(_onFocus);
    _focus.dispose();
    _controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    return Container(
      height: _chipHeight,
      constraints: const BoxConstraints(minWidth: 64, maxWidth: 180),
      padding: const EdgeInsets.symmetric(horizontal: 9),
      alignment: Alignment.center,
      decoration: BoxDecoration(
        borderRadius: BorderRadius.circular(_chipRadius),
        border: Border.all(color: scheme.primary.withValues(alpha: 0.6)),
      ),
      child: TextField(
        controller: _controller,
        focusNode: _focus,
        style: TextStyle(fontSize: 12, color: scheme.primary),
        cursorHeight: 14,
        decoration: const InputDecoration(
          isDense: true,
          border: InputBorder.none,
          contentPadding: EdgeInsets.zero,
        ),
        onSubmitted: (value) {
          widget.onSubmit(value);
          _controller.clear();
          _focus.requestFocus();
        },
      ),
    );
  }
}

/// The `+` badge, styled as a secondary action: outline, no fill.
class _AddValueChip extends StatelessWidget {
  const _AddValueChip({required this.onTap});

  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    return Tooltip(
      message: 'Add a value',
      child: InkWell(
        onTap: onTap,
        borderRadius: BorderRadius.circular(_chipRadius),
        child: Container(
          height: _chipHeight,
          width: 34,
          alignment: Alignment.center,
          decoration: BoxDecoration(
            borderRadius: BorderRadius.circular(_chipRadius),
            border: Border.all(color: scheme.outlineVariant),
          ),
          child: Icon(Icons.add, size: 14, color: scheme.onSurfaceVariant),
        ),
      ),
    );
  }
}

/// One value in a list property.
///
/// Deliberately small. The first version used an `IconButton` for the remove
/// affordance, whose 20px minimum plus padding pushed every chip to roughly
/// twice this height — on a phone the list read as a stack of grey slabs.
class ValueChip extends StatelessWidget {
  const ValueChip({super.key, required this.label, this.onRemove});

  final String label;
  final VoidCallback? onRemove;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    return Container(
      height: _chipHeight,
      constraints: const BoxConstraints(maxWidth: 220),
      padding: EdgeInsets.only(left: 9, right: onRemove == null ? 9 : 4),
      decoration: BoxDecoration(
        color: scheme.primary.withValues(alpha: 0.12),
        borderRadius: BorderRadius.circular(_chipRadius),
      ),
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          Flexible(
            child: Text(
              label,
              overflow: TextOverflow.ellipsis,
              style: TextStyle(
                fontSize: 12,
                height: 1.1,
                color: scheme.primary,
                fontWeight: FontWeight.w500,
              ),
            ),
          ),
          if (onRemove != null) ...[
            const SizedBox(width: 3),
            // A plain InkWell, not an IconButton: the latter enforces a
            // minimum tap target that doubles the chip's height.
            InkWell(
              onTap: onRemove,
              borderRadius: BorderRadius.circular(_chipRadius),
              child: Tooltip(
                message: 'Remove $label',
                child: Padding(
                  padding: const EdgeInsets.all(3),
                  child: Icon(Icons.close, size: 12, color: scheme.primary),
                ),
              ),
            ),
          ],
        ],
      ),
    );
  }
}

/// Chip geometry, shared so a value, a new-value field and the `+` all line up.
const double _chipHeight = 26;
const double _chipRadius = 8;

/// A value the list will not write: Storm's own fields, and structures a
/// key/value row cannot represent.
///
/// Shown rather than hidden. There is no raw-YAML mode to fall back to, so a
/// property that did not appear here would be invisible — and metadata you
/// cannot see is worse than metadata you cannot edit.
class _ReadOnlyValue extends StatelessWidget {
  const _ReadOnlyValue({required this.span});

  final fme.PropertySpan span;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final text = switch (span.form) {
      fme.PropertyForm.nested => 'Nested value',
      fme.PropertyForm.blockScalar => 'Multi-line text',
      // `created` and `modified` are RFC3339 with nanoseconds — precise, and
      // unreadable. Shown in local time, which is also the timezone the
      // reader is standing in.
      _ => formatStamp(span.displayValue),
    };
    final structural =
        span.form == fme.PropertyForm.nested ||
        span.form == fme.PropertyForm.blockScalar;

    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 4),
      child: SelectableText(
        text.isEmpty ? '—' : text,
        style: theme.textTheme.bodySmall?.copyWith(
          color: theme.colorScheme.onSurfaceVariant,
          fontStyle: structural ? FontStyle.italic : null,
          // Monospace only for values that are still raw, so a formatted date
          // reads as prose rather than as data.
          fontFamily: structural || _looksLikeStamp(span.displayValue)
              ? null
              : 'monospace',
        ),
      ),
    );
  }
}

/// The `+` from the sketch.
class _AddProperty extends StatelessWidget {
  const _AddProperty({required this.onAdd});

  final VoidCallback onAdd;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    return Padding(
      padding: const EdgeInsets.only(top: 4),
      child: Align(
        alignment: Alignment.centerLeft,
        child: Tooltip(
          message: 'Add property',
          child: InkWell(
            onTap: onAdd,
            borderRadius: BorderRadius.circular(9),
            // Just the `+`, at the same width as a key chip so the column
            // lines up. A label here would have to fit inside a fixed width
            // and cannot, at every font size.
            child: Container(
              width: 132,
              height: 30,
              alignment: Alignment.center,
              decoration: BoxDecoration(
                color: scheme.surfaceContainerHigh.withValues(alpha: 0.55),
                borderRadius: BorderRadius.circular(9),
              ),
              child: Icon(Icons.add, size: 17, color: scheme.onSurfaceVariant),
            ),
          ),
        ),
      ),
    );
  }
}
