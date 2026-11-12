import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_lucide/flutter_lucide.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../api/models.dart';
import '../router.dart';
import '../state/app_state.dart';
import 'controls.dart';
import 'decision_strip.dart';
import 'settings/settings_widgets.dart' show TextAction;
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
    return 'kept for ${daysWord(days!)}';
  }

  /// The first line under the Trash title (153l).
  String get intro => !known
      ? 'Deleted notes are kept for the vault’s retention period.'
      : never
      ? 'Deleted notes are kept until you empty the trash.'
      : 'Deleted notes are kept for ${daysWord(days!)}, '
            'then removed for good.';

  /// "27 days left", "less than a day left", or null when nothing counts
  /// down: kept until emptied, or the retention isn't known.
  String? timeLeft(String deletedAt, {DateTime? now}) {
    final keep = days;
    final at = DateTime.tryParse(deletedAt);
    if (keep == null || at == null) return null;
    final left = at.add(Duration(days: keep)).difference(now ?? DateTime.now());
    if (left.inDays < 1) return 'less than a day left';
    return '${daysWord(left.inDays)} left';
  }
}

String daysWord(int n) => n == 1 ? '1 day' : '$n days';

/// [vaultId]'s own retention, read from its trash (153g): never a constant,
/// since each vault sets its own.
final trashRetentionProvider = Provider.autoDispose
    .family<TrashRetention, String>((ref, vaultId) {
      if (ref.watch(apiProvider) == null || vaultId.isEmpty) {
        return const TrashRetention.unknown();
      }
      final page = ref.watch(trashProvider(vaultId));
      return switch (page) {
        AsyncData(:final value) => retentionOf(value.retentionDays),
        _ => const TrashRetention.unknown(),
      };
    });

TrashRetention retentionOf(int? days) =>
    days == null ? const TrashRetention.never() : TrashRetention.days(days);

/// `/v/<vault>/trash`. On a phone the selection bar takes the nav's place.
class TrashScreen extends StatefulWidget {
  const TrashScreen({super.key});

  @override
  State<TrashScreen> createState() => _TrashScreenState();
}

class _TrashScreenState extends State<TrashScreen> {
  bool _selecting = false;

  @override
  Widget build(BuildContext context) => StormScaffold(
    showNav: context.isExpanded || !_selecting,
    child: TrashView(
      vaultId: VaultGate.of(context),
      onSelecting: (on) {
        if (mounted && on != _selecting) setState(() => _selecting = on);
      },
    ),
  );
}

String notesCount(int n) => n == 1 ? '1 note' : '$n notes';

String itemsCount(int n) => n == 1 ? '1 item' : '$n items';

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

/// What the strip at the top of Trash is asking (153g): Delete forever for
/// the selected [items], or Empty trash (empty [items]).
typedef _Confirm = ({
  String title,
  String note,
  String action,
  List<TrashItem> items,
});

class TrashView extends ConsumerStatefulWidget {
  const TrashView({super.key, required this.vaultId, this.onSelecting});

  final String vaultId;

  /// Told when a selection starts or ends.
  final ValueChanged<bool>? onSelecting;

  @override
  ConsumerState<TrashView> createState() => _TrashViewState();
}

class _TrashViewState extends ConsumerState<TrashView> {
  _Confirm? _confirm;
  bool _busy = false;
  final _selected = <String>{};

  String get vaultId => widget.vaultId;

  @override
  void initState() {
    super.initState();
    HardwareKeyboard.instance.addHandler(_key);
  }

  @override
  void dispose() {
    HardwareKeyboard.instance.removeHandler(_key);
    super.dispose();
  }

  /// Esc clears the selection, unless the strip is asking: then Esc is its
  /// Cancel, and the selection waits for the answer.
  bool _key(KeyEvent e) {
    if (e is! KeyDownEvent ||
        e.logicalKey != LogicalKeyboardKey.escape ||
        _selected.isEmpty ||
        _confirm != null ||
        !mounted) {
      return false;
    }
    final route = ModalRoute.of(context);
    if (route != null && !route.isCurrent) return false;
    _clear();
    return true;
  }

  void _select(void Function() change) {
    final was = _selected.isNotEmpty;
    setState(change);
    if (was != _selected.isNotEmpty) {
      widget.onSelecting?.call(_selected.isNotEmpty);
    }
  }

  void _toggle(TrashItem item) => _select(() {
    if (!_selected.remove(item.id)) _selected.add(item.id);
  });

  void _clear() => _select(_selected.clear);

  List<TrashItem> _picked(List<TrashItem> items) => [
    for (final i in items)
      if (_selected.contains(i.id)) i,
  ];

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final wide = context.isExpanded;
    final page = ref.watch(trashProvider(vaultId));
    final keep = ref.watch(trashRetentionProvider(vaultId));
    final items = page.value?.items ?? const <TrashItem>[];
    final picked = _picked(items);
    final vault = ref
        .watch(vaultsProvider)
        .value
        ?.where((v) => v.id == vaultId)
        .firstOrNull;
    final confirm = _confirm;
    final selectionBar = picked.isEmpty
        ? null
        : _SelectionBar(
            count: picked.length,
            busy: _busy,
            docked: !wide,
            onRestore: () => _restoreAll(picked),
            onPurge: () => _askPurge(picked),
            onClear: _clear,
          );

    final body = Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Row(
          children: [
            Expanded(
              child: Text(
                vault == null ? 'Trash' : 'Trash · ${vault.name}',
                key: const Key('trash-title'),
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
          padding: EdgeInsets.only(top: t.sp, bottom: t.sp * 2.5),
          child: Wrap(
            spacing: t.sp * 0.75,
            crossAxisAlignment: WrapCrossAlignment.center,
            children: [
              Text(
                keep.intro,
                key: const Key('trash-intro'),
                style: TextStyle(
                  fontFamily: StormTokens.sansFamily,
                  fontSize: t.uiSize,
                  color: t.text2,
                  height: 1.5,
                ),
              ),
              TextAction(
                key: const Key('trash-retention-link'),
                label: 'Change in Settings › Vaults ›',
                accent: true,
                large: true,
                onTap: () =>
                    GoRouter.of(context).go(Routes.settingsPage('vaults')),
              ),
            ],
          ),
        ),
        if (wide && selectionBar != null) selectionBar,
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
                      _TrashCard(
                        key: Key('trash:${item.id}'),
                        item: item,
                        keep: keep,
                        selected: _selected.contains(item.id),
                        selecting: wide || picked.isNotEmpty,
                        onToggle: () => _toggle(item),
                        onRestore: _busy ? null : () => _restoreAll([item]),
                      ),
                  ],
                ),
        ),
      ],
    );

    final list = ListView(
      padding: EdgeInsets.fromLTRB(
        wide ? t.sp * 5 : StormChrome.contentInset(context),
        wide ? pageTopInset(t) : t.sp * 2,
        wide ? t.sp * 5 : StormChrome.contentInset(context),
        selectionBar != null && !wide
            ? t.sp * 3
            : StormChrome.navClearance(context),
      ),
      children: [
        Align(
          alignment: Alignment.topLeft,
          child: ConstrainedBox(
            constraints: BoxConstraints(
              maxWidth: wide ? t.sp * 80 : double.infinity,
            ),
            child: body,
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
        if (!wide && selectionBar != null) selectionBar,
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

  void _say(String message) => ScaffoldMessenger.maybeOf(
    context,
  )?.showSnackBar(SnackBar(content: Text(message)));

  /// Runs [act] on each of [items] in turn, since the server takes one at a
  /// time. What failed stays selected, and one toast says what and why.
  Future<void> _each(
    List<TrashItem> items,
    Future<void> Function(TrashItem) act, {
    required String done,
    required String verb,
  }) async {
    setState(() => _busy = true);
    final failed = <TrashItem>[];
    Object? why;
    for (final item in items) {
      try {
        await act(item);
      } catch (e) {
        failed.add(item);
        why ??= e;
      }
    }
    if (!mounted) return;
    _changed();
    _select(() {
      _busy = false;
      _selected
        ..clear()
        ..addAll(failed.map((i) => i.id));
    });
    if (failed.isEmpty) {
      _say(done);
    } else if (items.length == 1) {
      _say(describeFailure(why!));
    } else {
      _say(
        'Couldn’t $verb ${failed.length} of ${itemsCount(items.length)}: '
        '${describeFailure(why!)}',
      );
    }
  }

  Future<void> _restoreAll(List<TrashItem> items) async {
    final api = ref.read(apiProvider);
    if (api == null || items.isEmpty) return;
    await _each(
      items,
      (i) => api.restoreTrash(vaultId, i.id),
      done: items.length == 1
          ? 'Restored “${items.single.name}”'
          : 'Restored ${itemsCount(items.length)}',
      verb: 'restore',
    );
  }

  void _askPurge(List<TrashItem> items) => setState(() {
    final one = items.length == 1 ? items.single : null;
    _confirm = (
      title: one == null
          ? 'Delete ${itemsCount(items.length)} forever?'
          : 'Delete “${one.name}” forever?',
      note: one == null
          ? 'They are removed for good, with their history. '
                'This cannot be undone.'
          : one.isFolder
          ? 'The folder and its ${notesCount(one.noteCount)} are removed '
                'for good, with their history. This cannot be undone.'
          : 'The note is removed for good, with its history. '
                'This cannot be undone.',
      action: 'Delete forever',
      items: items,
    );
  });

  void _askEmpty() => setState(
    () => _confirm = (
      title: 'Empty trash?',
      note:
          'Everything in this vault’s trash is removed for good, with its '
          'history. This cannot be undone.',
      action: 'Empty trash',
      items: const [],
    ),
  );

  Future<void> _go(_Confirm confirm) async {
    final api = ref.read(apiProvider);
    if (api == null) return;
    final items = confirm.items;
    setState(() => _confirm = null);
    if (items.isEmpty) {
      setState(() => _busy = true);
      try {
        await api.emptyTrash(vaultId);
        if (mounted) {
          _changed();
          _say('Trash emptied');
        }
      } catch (e) {
        if (mounted) _say(describeFailure(e));
      }
      if (mounted) _select(() => _busy = false);
      return;
    }
    await _each(
      items,
      (i) => api.purgeTrash(vaultId, i.id),
      done: items.length == 1
          ? 'Deleted “${items.single.name}” forever'
          : 'Deleted ${itemsCount(items.length)} forever',
      verb: 'delete',
    );
  }
}

/// "{n} selected · Restore · Delete forever… · Esc": above the cards at
/// desk width, docked at the bottom on a phone.
class _SelectionBar extends StatelessWidget {
  const _SelectionBar({
    required this.count,
    required this.busy,
    required this.docked,
    required this.onRestore,
    required this.onPurge,
    required this.onClear,
  });

  final int count;
  final bool busy;
  final bool docked;
  final VoidCallback onRestore;
  final VoidCallback onPurge;
  final VoidCallback onClear;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final counted = Text(
      '$count selected',
      maxLines: 1,
      overflow: TextOverflow.ellipsis,
      style: TextStyle(
        fontFamily: StormTokens.sansFamily,
        fontSize: t.uiSize,
        color: t.text,
      ),
    );
    final restore = StormButton.soft(
      key: const Key('selection-restore'),
      label: 'Restore',
      expand: docked,
      onPressed: busy ? null : onRestore,
    );
    final purge = TextAction(
      key: const Key('selection-purge'),
      label: 'Delete forever…',
      danger: true,
      large: true,
      onTap: busy ? null : onPurge,
    );
    final clear = TextAction(
      key: const Key('selection-clear'),
      label: docked ? 'Clear' : 'Esc',
      large: true,
      onTap: onClear,
    );
    if (docked) {
      return Container(
        key: const Key('trash-selection'),
        padding: EdgeInsets.fromLTRB(
          StormChrome.contentInset(context),
          t.sp * 1.5,
          StormChrome.contentInset(context),
          t.sp * 1.5 + MediaQuery.paddingOf(context).bottom,
        ),
        decoration: BoxDecoration(
          color: t.surface,
          border: Border(
            top: BorderSide(color: t.border, width: t.bw),
          ),
        ),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          spacing: t.sp * 1.25,
          children: [
            Row(
              children: [
                Expanded(child: counted),
                clear,
              ],
            ),
            Row(
              spacing: t.sp * 2,
              children: [
                Expanded(child: restore),
                purge,
              ],
            ),
          ],
        ),
      );
    }
    final row = Row(
      spacing: t.sp * 1.5,
      children: [
        Expanded(child: counted),
        restore,
        purge,
        clear,
      ],
    );
    return Container(
      key: const Key('trash-selection'),
      margin: EdgeInsets.only(bottom: t.sp * 1.5),
      padding: EdgeInsets.only(bottom: t.sp * 1.25),
      decoration: BoxDecoration(
        border: Border(
          bottom: BorderSide(color: t.border, width: t.bw),
        ),
      ),
      child: row,
    );
  }
}

class _TrashCard extends StatelessWidget {
  const _TrashCard({
    super.key,
    required this.item,
    required this.keep,
    required this.selected,
    required this.selecting,
    required this.onToggle,
    required this.onRestore,
  });

  final TrashItem item;
  final TrashRetention keep;
  final bool selected;

  /// Checkboxes show: always at desk width, on a phone once one is chosen.
  final bool selecting;
  final VoidCallback onToggle;
  final VoidCallback? onRestore;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final mono = TextStyle(
      fontFamily: StormTokens.monoFamily,
      fontSize: t.labelSize,
      color: t.text3,
      height: 1.5,
    );
    final where = [
      item.originalPath,
      if (item.isFolder) notesCount(item.noteCount),
      ?keep.timeLeft(item.deletedAt),
    ].join(' · ');
    final reason = item.deletedBy.reason;

    return Semantics(
      selected: selected,
      child: Material(
        color: t.surface2,
        shape: RoundedRectangleBorder(
          borderRadius: BorderRadius.circular(t.rCard),
          side: BorderSide(color: selected ? t.accent : t.border, width: t.bw),
        ),
        clipBehavior: Clip.antiAlias,
        child: InkWell(
          onTap: selecting && !context.isExpanded ? onToggle : null,
          onLongPress: onToggle,
          child: Padding(
            padding: EdgeInsets.all(t.sp * 2),
            child: Row(
              crossAxisAlignment: CrossAxisAlignment.start,
              spacing: t.sp * 1.5,
              children: [
                Expanded(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    spacing: t.sp * 0.5,
                    children: [
                      Row(
                        spacing: t.sp,
                        children: [
                          if (selecting)
                            StormCheckbox(
                              key: Key('trash-check:${item.id}'),
                              value: selected,
                              size: t.sp * 2,
                              onChanged: (_) => onToggle(),
                            )
                          else
                            Icon(
                              item.isFolder
                                  ? LucideIcons.folder
                                  : LucideIcons.file_text,
                              size: t.uiSize,
                              color: t.text2,
                            ),
                          Expanded(
                            child: Text(
                              item.name,
                              maxLines: 1,
                              overflow: TextOverflow.ellipsis,
                              style: TextStyle(
                                fontFamily: StormTokens.sansFamily,
                                fontSize: t.uiSize,
                                fontWeight: FontWeight.w600,
                                color: t.text,
                              ),
                            ),
                          ),
                        ],
                      ),
                      Text(where, style: mono),
                      Text(
                        'Deleted ${deletedAgo(item.deletedAt)} by '
                        '${item.byLabel}',
                        style: TextStyle(
                          fontFamily: StormTokens.sansFamily,
                          fontSize: t.codeSize,
                          color: t.text2,
                        ),
                      ),
                      // An agent's reason is its own words: plain text,
                      // never markdown.
                      if (reason != null && reason.isNotEmpty)
                        Container(
                          margin: EdgeInsets.only(top: t.sp * 0.5),
                          padding: EdgeInsets.symmetric(
                            horizontal: t.sp,
                            vertical: t.sp * 0.75,
                          ),
                          decoration: BoxDecoration(
                            color: t.bg,
                            borderRadius: BorderRadius.circular(t.rControl),
                          ),
                          child: Text(
                            '“$reason”',
                            style: mono.copyWith(color: t.text2),
                          ),
                        ),
                    ],
                  ),
                ),
                StormButton.soft(
                  key: Key('restore:${item.id}'),
                  label: 'Restore',
                  onPressed: onRestore,
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}
