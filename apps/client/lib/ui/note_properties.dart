import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../editor/frontmatter_edit.dart' as fme;
import '../state/vault_config.dart';
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
/// What it still refuses to write is anything it cannot represent honestly: a
/// nested map or a block scalar renders read-only, and "Edit raw" hands over
/// the real text — see [onEditRaw].
class NoteProperties extends ConsumerStatefulWidget {
  const NoteProperties({
    super.key,
    required this.content,
    required this.onChanged,
    this.onEditRaw,
  });

  /// The note's whole file, frontmatter included.
  final String content;

  /// Called with the whole file after an edit. The panel never mutates
  /// anything itself; the session owns the buffer.
  final ValueChanged<String> onChanged;

  /// Opens the raw block for editing. Null hides the affordance.
  final VoidCallback? onEditRaw;

  @override
  ConsumerState<NoteProperties> createState() => _NotePropertiesState();
}

class _NotePropertiesState extends ConsumerState<NoteProperties> {
  bool _showManaged = false;

  /// Fields Storm owns. The server re-asserts `id` and `modified` on every
  /// save, so offering them as editable would be a lie.
  static const _managed = {'id', 'created', 'modified'};

  VaultConfig get _config =>
      ref.watch(vaultConfigProvider).value ?? const VaultConfig();

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final spans = fme.readSpans(widget.content);
    final own = spans.where((s) => !_managed.contains(s.key)).toList();
    final managed = spans.where((s) => _managed.contains(s.key)).toList();

    return Padding(
      padding: const EdgeInsets.only(bottom: 18),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          for (final span in own)
            _PropertyRow(
              key: ValueKey(span.key),
              span: span,
              type: _config.typeOf(span.key, span),
              options: _config.optionsFor(span.key),
              onValue: (value, {bool raw = false}) =>
                  _setScalar(span.key, value, raw: raw),
              onItems: (items) => _setList(span.key, items),
              onMenu: (action) => _menu(span, action),
            ),
          _AddProperty(onAdd: _add),
          if (managed.isNotEmpty) ...[
            const SizedBox(height: 4),
            _ManagedToggle(
              expanded: _showManaged,
              onToggle: () => setState(() => _showManaged = !_showManaged),
              onEditRaw: widget.onEditRaw,
            ),
            if (_showManaged)
              for (final span in managed) _ManagedRow(span: span),
          ] else if (widget.onEditRaw != null) ...[
            const SizedBox(height: 4),
            _ManagedToggle(
              expanded: false,
              onToggle: null,
              onEditRaw: widget.onEditRaw,
            ),
          ],
          const SizedBox(height: 10),
          Divider(height: 1, color: theme.dividerColor),
        ],
      ),
    );
  }

  // ---- writes ----------------------------------------------------------

  void _setScalar(String key, String value, {bool raw = false}) =>
      widget.onChanged(fme.setScalar(widget.content, key, value, raw: raw));

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
    required this.onValue,
    required this.onItems,
    required this.onMenu,
  });

  final fme.PropertySpan span;
  final PropertyType type;
  final List<String> options;
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
            icon: _iconFor(type),
            enabled: span.isEditable,
            onMenu: onMenu,
          ),
          const SizedBox(width: 10),
          Expanded(
            child: Padding(
              padding: const EdgeInsets.only(top: 2),
              child: span.isEditable
                  ? _ValueEditor(
                      span: span,
                      type: type,
                      options: options,
                      onValue: onValue,
                      onItems: onItems,
                    )
                  : _Unrepresentable(span: span),
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
      case PropertyType.url:
      case PropertyType.text:
        return _TextEditor(
          value: span.displayValue,
          onChanged: (v) => onValue(v),
        );
    }
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
    final date = '${d.day} ${_months[d.month - 1]} ${d.year}';
    if (!withTime) return date;
    final hh = d.hour.toString().padLeft(2, '0');
    final mm = d.minute.toString().padLeft(2, '0');
    return '$date, $hh:$mm';
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

/// Chips with an inline add field, for list-valued properties.
class _ListEditor extends StatefulWidget {
  const _ListEditor({required this.items, required this.onChanged});

  final List<String> items;
  final ValueChanged<List<String>> onChanged;

  @override
  State<_ListEditor> createState() => _ListEditorState();
}

class _ListEditorState extends State<_ListEditor> {
  final _controller = TextEditingController();
  final _focus = FocusNode();

  @override
  void dispose() {
    _controller.dispose();
    _focus.dispose();
    super.dispose();
  }

  void _add() {
    final value = _controller.text.trim();
    if (value.isEmpty) return;
    if (!widget.items.contains(value)) {
      widget.onChanged([...widget.items, value]);
    }
    _controller.clear();
    _focus.requestFocus();
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
        SizedBox(
          width: 96,
          child: TextField(
            controller: _controller,
            focusNode: _focus,
            style: Theme.of(context).textTheme.bodySmall,
            decoration: const InputDecoration(
              isDense: true,
              border: InputBorder.none,
              contentPadding: EdgeInsets.zero,
              hintText: 'Add…',
            ),
            onSubmitted: (_) => _add(),
            onEditingComplete: _add,
          ),
        ),
      ],
    );
  }
}

/// One value in a list property.
class ValueChip extends StatelessWidget {
  const ValueChip({super.key, required this.label, this.onRemove});

  final String label;
  final VoidCallback? onRemove;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    return Container(
      padding: EdgeInsets.only(
        left: 9,
        right: onRemove == null ? 9 : 3,
        top: 3,
        bottom: 3,
      ),
      decoration: BoxDecoration(
        color: scheme.primary.withValues(alpha: 0.12),
        borderRadius: BorderRadius.circular(11),
      ),
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          Text(
            label,
            style: TextStyle(
              fontSize: 12,
              color: scheme.primary,
              fontWeight: FontWeight.w500,
            ),
          ),
          if (onRemove != null)
            IconButton(
              icon: const Icon(Icons.close, size: 13),
              color: scheme.primary,
              visualDensity: VisualDensity.compact,
              padding: EdgeInsets.zero,
              constraints: const BoxConstraints(minWidth: 20, minHeight: 20),
              tooltip: 'Remove $label',
              onPressed: onRemove,
            ),
        ],
      ),
    );
  }
}

/// A nested map or block scalar: shown, never written.
class _Unrepresentable extends StatelessWidget {
  const _Unrepresentable({required this.span});

  final fme.PropertySpan span;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 4),
      child: Row(
        children: [
          Icon(Icons.lock_outline, size: 13, color: scheme.onSurfaceVariant),
          const SizedBox(width: 6),
          Expanded(
            child: Text(
              span.form == fme.PropertyForm.nested
                  ? 'Nested value — use Edit raw'
                  : 'Multi-line text — use Edit raw',
              style: TextStyle(fontSize: 12, color: scheme.onSurfaceVariant),
            ),
          ),
        ],
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

class _ManagedToggle extends StatelessWidget {
  const _ManagedToggle({
    required this.expanded,
    required this.onToggle,
    required this.onEditRaw,
  });

  final bool expanded;
  final VoidCallback? onToggle;
  final VoidCallback? onEditRaw;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    return Row(
      children: [
        if (onToggle != null)
          Flexible(
            child: InkWell(
              onTap: onToggle,
              borderRadius: BorderRadius.circular(6),
              child: Padding(
                padding: const EdgeInsets.symmetric(horizontal: 4, vertical: 4),
                child: Row(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    Icon(
                      expanded ? Icons.expand_less : Icons.expand_more,
                      size: 16,
                      color: scheme.onSurfaceVariant,
                    ),
                    const SizedBox(width: 4),
                    Flexible(
                      child: Text(
                        expanded ? 'Hide details' : 'Details',
                        overflow: TextOverflow.ellipsis,
                        style: TextStyle(
                          fontSize: 12,
                          color: scheme.onSurfaceVariant,
                        ),
                      ),
                    ),
                  ],
                ),
              ),
            ),
          ),
        const Spacer(),
        if (onEditRaw != null)
          TextButton(
            onPressed: onEditRaw,
            style: TextButton.styleFrom(
              visualDensity: VisualDensity.compact,
              padding: const EdgeInsets.symmetric(horizontal: 8),
            ),
            child: const Text('Edit raw', style: TextStyle(fontSize: 12)),
          ),
      ],
    );
  }
}

class _ManagedRow extends StatelessWidget {
  const _ManagedRow({required this.span});

  final fme.PropertySpan span;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 3),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          SizedBox(
            width: 132,
            child: Text(
              span.key,
              style: theme.textTheme.bodySmall?.copyWith(
                color: theme.colorScheme.onSurfaceVariant,
                fontWeight: FontWeight.w500,
              ),
              overflow: TextOverflow.ellipsis,
            ),
          ),
          const SizedBox(width: 10),
          Expanded(
            child: SelectableText(
              span.displayValue,
              style: theme.textTheme.bodySmall?.copyWith(
                fontFamily: 'monospace',
                color: theme.colorScheme.onSurfaceVariant,
              ),
            ),
          ),
        ],
      ),
    );
  }
}
