import 'package:flutter/material.dart';
import 'package:flutter_lucide/flutter_lucide.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../api/models.dart';
import '../state/app_state.dart';
import '../state/vault_config.dart';
import 'controls.dart';
import 'decision_strip.dart';
import 'shell/storm_scaffold.dart';
import 'breakpoints.dart';
import 'shell/app_bar.dart' show pageTopInset;
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

/// How long a vault's trash keeps an item (decision 150b), as its trash
/// says: [days], never ([never]), or not known here ([known] false).
class TrashRetention {
  const TrashRetention.days(int this.days) : known = true, never = false;
  const TrashRetention.never() : days = null, known = true, never = true;
  const TrashRetention.unknown() : days = null, known = false, never = false;

  final int? days;
  final bool known;
  final bool never;

  /// "kept for 30 days", "kept until you empty it", or, unknown, "kept for
  /// the vault's retention period".
  String get keptFor {
    if (!known) return 'kept for the vault’s retention period';
    if (never) return 'kept until you empty it';
    return days == 1 ? 'kept for 1 day' : 'kept for $days days';
  }
}

/// [vaultId]'s own retention, read from its trash (153g): never a constant,
/// since each vault sets its own.
final trashRetentionProvider = Provider.autoDispose
    .family<TrashRetention, String>((ref, vaultId) {
      if (ref.watch(apiProvider) == null || vaultId.isEmpty) {
        return const TrashRetention.unknown();
      }
      final page = ref.watch(trashProvider(vaultId));
      return switch (page) {
        AsyncData(:final value) =>
          value.retentionDays == null
              ? const TrashRetention.never()
              : TrashRetention.days(value.retentionDays!),
        _ => const TrashRetention.unknown(),
      };
    });

/// `/v/<vault>/trash`.
class TrashScreen extends StatelessWidget {
  const TrashScreen({super.key});

  @override
  Widget build(BuildContext context) =>
      StormScaffold(child: TrashView(vaultId: VaultGate.of(context)));
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

/// What the strip at the top of Trash is asking (153g): one item's Delete
/// forever, or Empty trash.
typedef _Confirm = ({
  String title,
  String note,
  String action,
  TrashItem? item,
});

class TrashView extends ConsumerStatefulWidget {
  const TrashView({super.key, required this.vaultId});

  final String vaultId;

  @override
  ConsumerState<TrashView> createState() => _TrashViewState();
}

class _TrashViewState extends ConsumerState<TrashView> {
  static const _periods = [7, 30, 90, null];

  _Confirm? _confirm;
  bool _busy = false;

  String get vaultId => widget.vaultId;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final page = ref.watch(trashProvider(vaultId));
    final items = page.value?.items ?? const <TrashItem>[];
    final confirm = _confirm;
    final list = ListView(
      padding: EdgeInsets.fromLTRB(
        StormChrome.contentInset(context),
        context.isExpanded ? pageTopInset(t) : t.sp * 2,
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
              onPressed: items.isEmpty ? null : _askEmpty,
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
              ? (days) => _setRetention(context, days)
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
                        onRestore: () => _restore(context, item),
                        onPurge: () => _askPurge(item),
                      ),
                  ],
                ),
        ),
      ],
    );
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        if (confirm != null)
          DecisionStrip(
            key: const Key('trash-confirm'),
            label: 'CONFIRM',
            title: confirm.title,
            note: confirm.note,
            onEscape: _cancel,
            actions: [
              DecisionAction(
                key: const Key('confirm-cancel'),
                label: 'Cancel',
                kind: DecisionActionKind.text,
                onPressed: _cancel,
              ),
              DecisionAction(
                key: const Key('confirm-action'),
                label: confirm.action,
                kind: DecisionActionKind.danger,
                onPressed: _busy ? null : () => _go(confirm),
              ),
            ],
          ),
        Expanded(child: list),
      ],
    );
  }

  void _cancel() {
    if (mounted) setState(() => _confirm = null);
  }

  void _changed() {
    ref.read(vaultRevisionProvider.notifier).state++;
    ref.invalidate(treeProvider);
    ref.invalidate(trashProvider(vaultId));
  }

  void _say(BuildContext context, String message) => ScaffoldMessenger.maybeOf(
    context,
  )?.showSnackBar(SnackBar(content: Text(message)));

  Future<void> _run(Future<void> Function() act, String done) async {
    try {
      await act();
      _changed();
      if (mounted) _say(context, done);
    } catch (e) {
      if (mounted) _say(context, describeFailure(e));
    }
  }

  Future<void> _restore(BuildContext context, TrashItem item) async {
    final api = ref.read(apiProvider);
    if (api == null) return;
    await _run(
      () => api.restoreTrash(vaultId, item.id),
      'Restored “${item.name}”',
    );
  }

  void _askPurge(TrashItem item) => setState(
    () => _confirm = (
      title: 'Delete “${item.name}” forever?',
      note: item.isFolder
          ? 'The folder and its ${notesCount(item.noteCount)} are removed '
                'for good, with their history. This cannot be undone.'
          : 'The note is removed for good, with its history. '
                'This cannot be undone.',
      action: 'Delete forever',
      item: item,
    ),
  );

  void _askEmpty() => setState(
    () => _confirm = (
      title: 'Empty trash?',
      note:
          'Everything in this vault’s trash is removed for good, with its '
          'history. This cannot be undone.',
      action: 'Empty trash',
      item: null,
    ),
  );

  Future<void> _go(_Confirm confirm) async {
    final api = ref.read(apiProvider);
    if (api == null) return;
    setState(() => _busy = true);
    final item = confirm.item;
    await _run(
      item == null
          ? () => api.emptyTrash(vaultId)
          : () => api.purgeTrash(vaultId, item.id),
      item == null ? 'Trash emptied' : 'Deleted “${item.name}” forever',
    );
    if (mounted) {
      setState(() {
        _busy = false;
        _confirm = null;
      });
    }
  }

  Future<void> _setRetention(BuildContext context, int? days) async {
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
