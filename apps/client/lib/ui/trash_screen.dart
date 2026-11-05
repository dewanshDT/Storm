import 'package:flutter/material.dart';
import 'package:flutter_lucide/flutter_lucide.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../api/models.dart';
import '../state/app_state.dart';
import '../state/vault_config.dart';
import 'controls.dart';
import 'shell/storm_scaffold.dart';
import 'shell/vault_gate.dart';
import 'states.dart';
import 'tokens.dart';
import 'widgets.dart';

/// A vault's trash (decision 150b). Watches the revision so a delete made
/// elsewhere in the app shows here.
final trashProvider = FutureProvider.autoDispose.family<TrashPage, String>((
  ref,
  vaultId,
) async {
  ref.watch(vaultRevisionProvider);
  final api = ref.watch(apiProvider);
  if (api == null || vaultId.isEmpty) {
    return const TrashPage(items: [], retentionDays: 30);
  }
  return api.trash(vaultId);
});

/// `/v/<vault>/trash`.
class TrashScreen extends StatelessWidget {
  const TrashScreen({super.key});

  @override
  Widget build(BuildContext context) =>
      StormScaffold(child: TrashView(vaultId: VaultGate.of(context)));
}

/// Asks before something irreversible. Anything but the button is a no.
Future<bool> confirmDestructive(
  BuildContext context, {
  required String title,
  required String body,
  required String action,
}) async {
  final confirmed = await showDialog<bool>(
    context: context,
    builder: (c) => AlertDialog(
      title: Text(title),
      content: Text(body),
      actions: [
        TextButton(
          key: const Key('confirm-cancel'),
          onPressed: () => Navigator.pop(c, false),
          child: const Text('Cancel'),
        ),
        FilledButton(
          key: const Key('confirm-action'),
          onPressed: () => Navigator.pop(c, true),
          style: FilledButton.styleFrom(
            backgroundColor: Theme.of(c).colorScheme.error,
          ),
          child: Text(action),
        ),
      ],
    ),
  );
  return confirmed == true;
}

String notesCount(int n) => n == 1 ? '1 note' : '$n notes';

/// "3h ago", "just now".
String deletedAgo(String iso, {DateTime? now}) {
  final then = DateTime.tryParse(iso);
  if (then == null) return '';
  final d = (now ?? DateTime.now()).difference(then);
  if (d.inMinutes < 1) return 'just now';
  if (d.inHours < 1) return '${d.inMinutes}m ago';
  if (d.inDays < 1) return '${d.inHours}h ago';
  return '${d.inDays}d ago';
}

class TrashView extends ConsumerWidget {
  const TrashView({super.key, required this.vaultId});

  final String vaultId;

  static const _periods = [7, 30, 90, null];

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final t = context.tokens;
    final page = ref.watch(trashProvider(vaultId));
    final items = page.value?.items ?? const <TrashItem>[];

    return ListView(
      padding: EdgeInsets.fromLTRB(
        StormChrome.contentInset(context),
        t.sp * 2,
        StormChrome.contentInset(context),
        StormChrome.navClearance(context),
      ),
      children: [
        Row(
          children: [
            Expanded(
              child: Text(
                'Trash',
                style: TextStyle(
                  fontFamily: StormTokens.sansFamily,
                  fontSize: t.titleSize,
                  fontWeight: FontWeight.w600,
                  color: t.text,
                ),
              ),
            ),
            StormButton.outline(
              key: const Key('trash-empty'),
              label: 'Empty trash',
              icon: LucideIcons.trash_2,
              onPressed: items.isEmpty ? null : () => _empty(context, ref),
            ),
          ],
        ),
        Padding(
          padding: EdgeInsets.only(top: t.sp * 2, bottom: t.sp),
          child: const SectionLabel('Keep deleted items for'),
        ),
        ChoiceChips<int?>(
          options: [
            for (final days in _periods)
              ChoiceOption(days, days == null ? 'Never delete' : '$days days'),
          ],
          selected: page.value?.retentionDays,
          onSelected: page.hasValue
              ? (days) => _setRetention(context, ref, days)
              : null,
        ),
        SizedBox(height: t.sp * 2.5),
        page.when(
          loading: () => const SkeletonRows(),
          error: (e, _) => EmptyState(
            icon: LucideIcons.cloud_off,
            title: 'Could not load the trash',
            detail: describeFailure(e),
            action: 'Try again',
            onAction: () => ref.invalidate(trashProvider(vaultId)),
          ),
          data: (p) => p.items.isEmpty
              ? const EmptyState(
                  icon: LucideIcons.trash_2,
                  title: 'Trash is empty',
                  detail:
                      'Deleted notes and folders wait here before they are '
                      'removed for good.',
                )
              : Column(
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  spacing: t.sp,
                  children: [
                    for (final item in p.items)
                      _TrashRow(
                        key: Key('trash:${item.id}'),
                        item: item,
                        onRestore: () => _restore(context, ref, item),
                        onPurge: () => _purge(context, ref, item),
                      ),
                  ],
                ),
        ),
      ],
    );
  }

  void _changed(WidgetRef ref) {
    ref.read(vaultRevisionProvider.notifier).state++;
    ref.invalidate(treeProvider);
    ref.invalidate(trashProvider(vaultId));
  }

  void _say(BuildContext context, String message) => ScaffoldMessenger.maybeOf(
    context,
  )?.showSnackBar(SnackBar(content: Text(message)));

  Future<void> _run(
    BuildContext context,
    WidgetRef ref,
    Future<void> Function() act,
    String done,
  ) async {
    try {
      await act();
      _changed(ref);
      if (context.mounted) _say(context, done);
    } catch (e) {
      if (context.mounted) _say(context, describeFailure(e));
    }
  }

  Future<void> _restore(
    BuildContext context,
    WidgetRef ref,
    TrashItem item,
  ) async {
    final api = ref.read(apiProvider);
    if (api == null) return;
    await _run(
      context,
      ref,
      () => api.restoreTrash(vaultId, item.id),
      'Restored “${item.name}”',
    );
  }

  Future<void> _purge(
    BuildContext context,
    WidgetRef ref,
    TrashItem item,
  ) async {
    final api = ref.read(apiProvider);
    if (api == null) return;
    final ok = await confirmDestructive(
      context,
      title: 'Delete “${item.name}” forever?',
      body: item.isFolder
          ? 'The folder and its ${notesCount(item.noteCount)} are removed '
                'for good, with their history. This cannot be undone.'
          : 'The note is removed for good, with its history. '
                'This cannot be undone.',
      action: 'Delete forever',
    );
    if (!ok || !context.mounted) return;
    await _run(
      context,
      ref,
      () => api.purgeTrash(vaultId, item.id),
      'Deleted “${item.name}” forever',
    );
  }

  Future<void> _empty(BuildContext context, WidgetRef ref) async {
    final api = ref.read(apiProvider);
    if (api == null) return;
    final ok = await confirmDestructive(
      context,
      title: 'Empty trash?',
      body:
          'Everything in this vault\'s trash is removed for good, with its '
          'history. This cannot be undone.',
      action: 'Empty trash',
    );
    if (!ok || !context.mounted) return;
    await _run(context, ref, () => api.emptyTrash(vaultId), 'Trash emptied');
  }

  Future<void> _setRetention(
    BuildContext context,
    WidgetRef ref,
    int? days,
  ) async {
    final config = await ref.read(vaultConfigProvider.future);
    final saved = await saveVaultConfig(ref, config.withTrashRetention(days));
    ref.invalidate(trashProvider(vaultId));
    if (!saved && context.mounted) {
      _say(context, 'Could not change how long the trash keeps items');
    }
  }
}

class _TrashRow extends StatelessWidget {
  const _TrashRow({
    super.key,
    required this.item,
    required this.onRestore,
    required this.onPurge,
  });

  final TrashItem item;
  final VoidCallback onRestore;
  final VoidCallback onPurge;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final small = TextStyle(
      fontFamily: StormTokens.sansFamily,
      fontSize: t.codeSize,
      color: t.text3,
    );
    final meta = [
      'Deleted ${deletedAgo(item.deletedAt)} by ${item.byLabel}',
      if (item.isFolder) notesCount(item.noteCount),
    ].join(' · ');
    final reason = item.deletedBy.reason;

    return Container(
      padding: EdgeInsets.all(t.sp * 1.5),
      decoration: BoxDecoration(
        color: t.surface2,
        borderRadius: BorderRadius.circular(t.rControl),
        border: Border.all(color: t.border, width: t.bw),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        spacing: t.sp * 0.5,
        children: [
          Row(
            children: [
              Icon(
                item.isFolder ? LucideIcons.folder : LucideIcons.file_text,
                size: t.uiSize,
                color: t.text2,
              ),
              SizedBox(width: t.sp),
              Expanded(
                child: Text(
                  item.name,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: TextStyle(
                    fontFamily: StormTokens.sansFamily,
                    fontSize: t.bodySize,
                    fontWeight: FontWeight.w500,
                    color: t.text,
                  ),
                ),
              ),
            ],
          ),
          Text(item.originalPath, style: small),
          Text(meta, style: small),
          // An agent's reason is its own words: plain text, never markdown.
          if (reason != null && reason.isNotEmpty)
            Text('“$reason”', style: small.copyWith(color: t.text2)),
          Padding(
            padding: EdgeInsets.only(top: t.sp * 0.5),
            child: Wrap(
              spacing: t.sp,
              runSpacing: t.sp,
              children: [
                StormButton.soft(
                  key: Key('restore:${item.id}'),
                  label: 'Restore',
                  icon: LucideIcons.undo_2,
                  onPressed: onRestore,
                ),
                StormButton.text(
                  key: Key('purge:${item.id}'),
                  label: 'Delete forever',
                  onPressed: onPurge,
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}
