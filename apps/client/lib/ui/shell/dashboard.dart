import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../../api/models.dart';
import '../../router.dart';
import '../../state/app_state.dart';
import 'corner_bubbles.dart';
import 'nav_bubble.dart';
import 'storm_scaffold.dart';

/// The home screen: what the vault holds, and what you touched last.
class DashboardScreen extends ConsumerWidget {
  const DashboardScreen({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    ref.watch(syncListenerProvider);
    final notes = ref.watch(treeProvider);

    return NewNoteRequest(
      onRequest: () => _create(context, ref),
      child: Scaffold(
        appBar: const DashboardHeader(),
        body: Stack(
          children: [
            Positioned.fill(
              child: RefreshIndicator(
                onRefresh: () async {
                  await ref.read(syncEngineProvider).sync();
                  ref.invalidate(treeProvider);
                },
                child: notes.when(
                  loading: () =>
                      const Center(child: CircularProgressIndicator()),
                  error: (e, _) => _Message(
                    icon: Icons.cloud_off,
                    text: '$e',
                    action: 'Retry',
                    onAction: () => ref.invalidate(treeProvider),
                  ),
                  data: (list) => _Body(notes: list),
                ),
              ),
            ),
            const Positioned(left: 0, right: 0, bottom: 0, child: NavBubble()),
          ],
        ),
      ),
    );
  }

  Future<void> _create(BuildContext context, WidgetRef ref) async {
    final path = await promptForPath(
      context,
      title: 'New note',
      initial: 'Untitled.md',
    );
    if (path == null || !context.mounted) return;

    final created = await ref.read(syncEngineProvider).create(path: path);
    if (!context.mounted) return;
    if (created.meta == null) {
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(content: Text(created.error ?? 'Could not create the note')),
      );
      return;
    }
    ref.invalidate(treeProvider);
    context.go(Routes.note(created.meta!.id));
  }
}

class _Body extends ConsumerWidget {
  const _Body({required this.notes});

  final List<NoteMeta> notes;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final engine = ref.watch(syncEngineProvider);

    // "Recent" means recently *edited*, from the `modified` the server already
    // returns — tracking a second "opened" timestamp would be real plumbing
    // for a low-effort screen.
    final recent = [...notes]
      ..sort((a, b) => b.modified.compareTo(a.modified));

    return ListView(
      padding: const EdgeInsets.fromLTRB(16, 8, 16, 110),
      children: [
        Row(
          children: [
            _Stat(
              value: '${notes.length}',
              label: notes.length == 1 ? 'note' : 'notes',
            ),
            const SizedBox(width: 12),
            _Stat(
              value: relativeTime(engine.lastSyncedAt),
              label: 'last synced',
            ),
          ],
        ),
        const SizedBox(height: 24),
        Padding(
          padding: const EdgeInsets.only(left: 4, bottom: 6),
          child: Text(
            'Recent',
            style: Theme.of(context).textTheme.labelLarge?.copyWith(
                  color: Theme.of(context).colorScheme.onSurfaceVariant,
                ),
          ),
        ),
        if (recent.isEmpty)
          const _Message(
            icon: Icons.note_add_outlined,
            text: 'No notes yet.\nUse the bubble below to make one.',
          )
        else
          for (final note in recent.take(20)) _RecentTile(note: note),
      ],
    );
  }
}

class _Stat extends StatelessWidget {
  const _Stat({required this.value, required this.label});

  final String value;
  final String label;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    return Expanded(
      child: Container(
        padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 14),
        decoration: BoxDecoration(
          color: scheme.surfaceContainerHigh,
          borderRadius: BorderRadius.circular(14),
        ),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text(
              value,
              style: Theme.of(context).textTheme.titleLarge?.copyWith(
                    fontWeight: FontWeight.w600,
                  ),
            ),
            const SizedBox(height: 2),
            Text(
              label,
              style: Theme.of(context).textTheme.bodySmall?.copyWith(
                    color: scheme.onSurfaceVariant,
                  ),
            ),
          ],
        ),
      ),
    );
  }
}

class _RecentTile extends ConsumerWidget {
  const _RecentTile({required this.note});

  final NoteMeta note;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final scheme = Theme.of(context).colorScheme;
    final pinned = ref.watch(pinnedNotesProvider).value ?? const <String>{};

    return InkWell(
      onTap: () => context.go(Routes.note(note.id)),
      borderRadius: BorderRadius.circular(12),
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 11),
        child: Row(
          children: [
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Row(
                    children: [
                      Flexible(
                        child: Text(
                          note.title.isEmpty ? note.fileName : note.title,
                          style: Theme.of(context).textTheme.bodyLarge,
                          overflow: TextOverflow.ellipsis,
                        ),
                      ),
                      if (pinned.contains(note.id))
                        Padding(
                          padding: const EdgeInsets.only(left: 6),
                          child: Icon(
                            Icons.push_pin,
                            size: 12,
                            color: scheme.primary,
                          ),
                        ),
                    ],
                  ),
                  const SizedBox(height: 2),
                  Text(
                    note.folder.isEmpty ? 'in the vault root' : note.folder,
                    style: Theme.of(context).textTheme.bodySmall?.copyWith(
                          color: scheme.onSurfaceVariant,
                        ),
                    overflow: TextOverflow.ellipsis,
                  ),
                ],
              ),
            ),
            const SizedBox(width: 10),
            Text(
              relativeTime(DateTime.tryParse(note.modified)?.toLocal()),
              style: Theme.of(context).textTheme.bodySmall?.copyWith(
                    color: scheme.onSurfaceVariant,
                  ),
            ),
          ],
        ),
      ),
    );
  }
}

class _Message extends StatelessWidget {
  const _Message({
    required this.icon,
    required this.text,
    this.action,
    this.onAction,
  });

  final IconData icon;
  final String text;
  final String? action;
  final VoidCallback? onAction;

  @override
  Widget build(BuildContext context) {
    final muted = Theme.of(context).colorScheme.onSurfaceVariant;
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 48, horizontal: 24),
      child: Column(
        children: [
          Icon(icon, size: 30, color: muted),
          const SizedBox(height: 12),
          Text(
            text,
            textAlign: TextAlign.center,
            style: Theme.of(context).textTheme.bodyMedium?.copyWith(color: muted),
          ),
          if (action != null) ...[
            const SizedBox(height: 14),
            OutlinedButton(onPressed: onAction, child: Text(action!)),
          ],
        ],
      ),
    );
  }
}
