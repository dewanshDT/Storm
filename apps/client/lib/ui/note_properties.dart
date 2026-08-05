import 'package:flutter/material.dart';

import '../editor/frontmatter.dart';

/// A note's frontmatter, rendered as properties instead of raw YAML.
///
/// Obsidian and Notion both show metadata as a small table above the note
/// rather than as text you scroll past, and the raw `---` block in the editor
/// was the single ugliest thing about the app.
///
/// Read-only for now, and honest about it: writing values back means
/// re-serialising the user's YAML, which is exactly what the server refuses to
/// do (it would reorder keys and drop comments). "Edit raw" hands over the
/// real text instead — see [onEditRaw].
class NoteProperties extends StatefulWidget {
  const NoteProperties({super.key, required this.frontmatter, this.onEditRaw});

  final String frontmatter;

  /// Opens the raw block for editing. Null hides the affordance.
  final VoidCallback? onEditRaw;

  @override
  State<NoteProperties> createState() => _NotePropertiesState();
}

class _NotePropertiesState extends State<NoteProperties> {
  bool _showManaged = false;

  @override
  Widget build(BuildContext context) {
    if (widget.frontmatter.isEmpty) return const SizedBox.shrink();

    final theme = Theme.of(context);
    final all = readProperties(widget.frontmatter);
    if (all.isEmpty) return const SizedBox.shrink();

    // `id`, `created` and `modified` are Storm's bookkeeping. They're real,
    // so they stay reachable, but they aren't what the note is about.
    final own = all.where((p) => !p.isManaged).toList();
    final managed = all.where((p) => p.isManaged).toList();
    final tags = readTags(widget.frontmatter);

    return Container(
      width: double.infinity,
      padding: const EdgeInsets.fromLTRB(28, 14, 28, 14),
      decoration: BoxDecoration(
        border: Border(bottom: BorderSide(color: theme.dividerColor)),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          for (final p in own)
            if (p.key == 'tags')
              _Row(
                label: 'tags',
                child: Wrap(
                  spacing: 6,
                  runSpacing: 6,
                  children: [for (final t in tags) _TagChip(tag: t)],
                ),
              )
            else
              _Row(
                label: p.key,
                child: Text(
                  p.value.isEmpty ? '—' : p.value,
                  style: theme.textTheme.bodyMedium,
                ),
              ),
          if (managed.isNotEmpty) ...[
            if (own.isNotEmpty) const SizedBox(height: 2),
            InkWell(
              onTap: () => setState(() => _showManaged = !_showManaged),
              child: Padding(
                padding: const EdgeInsets.symmetric(vertical: 4),
                child: Row(
                  children: [
                    Icon(
                      _showManaged ? Icons.expand_more : Icons.chevron_right,
                      size: 14,
                      color: theme.colorScheme.onSurfaceVariant,
                    ),
                    const SizedBox(width: 4),
                    Text(
                      _showManaged ? 'Hide details' : 'Details',
                      style: theme.textTheme.bodySmall?.copyWith(
                        color: theme.colorScheme.onSurfaceVariant,
                      ),
                    ),
                    const Spacer(),
                    if (widget.onEditRaw != null)
                      TextButton(
                        onPressed: widget.onEditRaw,
                        style: TextButton.styleFrom(
                          visualDensity: VisualDensity.compact,
                          padding: const EdgeInsets.symmetric(horizontal: 8),
                        ),
                        child: const Text('Edit raw'),
                      ),
                  ],
                ),
              ),
            ),
            if (_showManaged)
              for (final p in managed)
                _Row(
                  label: p.key,
                  child: SelectableText(
                    p.value,
                    style: theme.textTheme.bodySmall?.copyWith(
                      fontFamily: 'monospace',
                      color: theme.colorScheme.onSurfaceVariant,
                    ),
                  ),
                ),
          ],
        ],
      ),
    );
  }
}

class _Row extends StatelessWidget {
  const _Row({required this.label, required this.child});

  final String label;
  final Widget child;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 3),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          SizedBox(
            width: 108,
            child: Text(
              label,
              style: theme.textTheme.bodySmall?.copyWith(
                color: theme.colorScheme.onSurfaceVariant,
                fontWeight: FontWeight.w500,
              ),
              overflow: TextOverflow.ellipsis,
            ),
          ),
          const SizedBox(width: 10),
          Expanded(child: child),
        ],
      ),
    );
  }
}

class _TagChip extends StatelessWidget {
  const _TagChip({required this.tag});

  final String tag;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 9, vertical: 3),
      decoration: BoxDecoration(
        color: scheme.primary.withValues(alpha: 0.12),
        borderRadius: BorderRadius.circular(11),
      ),
      child: Text(
        '#$tag',
        style: TextStyle(
          fontSize: 12,
          color: scheme.primary,
          fontWeight: FontWeight.w500,
        ),
      ),
    );
  }
}
