import 'package:flutter/material.dart';
import 'package:flutter_lucide/flutter_lucide.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../api/models.dart';
import '../router.dart';
import '../state/app_state.dart';
import '../state/vault_config.dart';
import 'breakpoints.dart';
import 'tokens.dart';
import 'widgets.dart';
import 'shell/nav_bubble.dart' show NewNoteRequest;
import 'shell/sidebar_rows.dart';
import 'states.dart';
import 'shell/storm_scaffold.dart';
import 'shell/vault_sidebar.dart' show NoNoteSelected;
import 'shell/vault_gate.dart';

/// The phone's Notes list (handoff §2.4): the vault root with RECENT above
/// FOLDERS, or one folder with "‹ parent" above its name.
class BrowseScreen extends ConsumerWidget {
  const BrowseScreen({super.key, required this.folder});

  /// Vault-relative, `''` at the root.
  final String folder;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final notes = ref.watch(treeProvider);
    final vaultId = VaultGate.of(context);
    final known = ref.watch(vaultFoldersProvider);

    // The sidebar is the browser at this width.
    if (context.isExpanded) return const NoNoteSelected();

    final t = context.tokens;
    final vaultName =
        (ref.watch(vaultsProvider).value ?? const <VaultInfo>[])
            .where((v) => v.id == vaultId)
            .firstOrNull
            ?.name ??
        'Vault';
    final parts = folder.isEmpty ? const <String>[] : folder.split('/');
    final parent = parts.length > 1
        ? parts.sublist(0, parts.length - 1).join('/')
        : '';

    final head = <Widget>[
      if (parts.isNotEmpty)
        Align(
          alignment: Alignment.centerLeft,
          child: BackLink(
            label: parent.isEmpty ? vaultName : parent.split('/').last,
            onTap: () => leaveTo(
              context,
              parent.isEmpty
                  ? Routes.browse(vaultId)
                  : Routes.folder(vaultId, parent),
            ),
          ),
        ),
      Padding(
        padding: EdgeInsets.only(top: t.sp * 0.5, bottom: t.sp * 0.75),
        child: Text(
          parts.isEmpty ? vaultName : parts.last,
          key: const Key('browse-title'),
          maxLines: 2,
          overflow: TextOverflow.ellipsis,
          style: TextStyle(
            fontFamily: StormTokens.sansFamily,
            fontSize: t.titleSize,
            fontWeight: FontWeight.w600,
            color: t.text,
            height: 1.2,
          ),
        ),
      ),
      if (parts.isEmpty) ...[
        _PhoneRecents(vaultId: vaultId),
        Padding(
          padding: EdgeInsets.only(top: t.sp * 2.75, bottom: t.sp * 0.5),
          child: const SectionLabel('Folders'),
        ),
      ],
    ];

    return StormScaffold(
      child: notes.when(
        loading: () => Padding(
          padding: EdgeInsets.symmetric(horizontal: t.sp * 2.5),
          child: const SkeletonRows(),
        ),
        error: (e, _) => EmptyState(
          fill: true,
          icon: LucideIcons.cloud_off,
          title: 'Could not list this folder',
          detail: describeFailure(e),
          action: 'Try again',
          onAction: () => ref.invalidate(treeProvider),
        ),
        data: (list) {
          final entries = _childrenOf(list, folder, known);
          return ListView(
            padding: EdgeInsets.fromLTRB(
              StormChrome.contentInset(context),
              0,
              StormChrome.contentInset(context),
              StormChrome.navClearance(context),
            ),
            children: [
              ...head,
              if (entries.isEmpty)
                EmptyState(
                  icon: LucideIcons.folder_open,
                  title: 'Nothing in this folder',
                  detail: folder.isEmpty
                      ? 'New notes will land at the top of this vault.'
                      : 'New notes made here will land in ${parts.last}.',
                  action: 'New note',
                  onAction: () => NewNoteRequest.of(context)?.call(),
                )
              else
                for (final entry in entries) EntryTile(entry: entry),
            ],
          );
        },
      ),
    );
  }
}

/// Pops when there is somewhere to pop to, else goes to [fallback] — the
/// logical parent a deep link has no route for.
void leaveTo(BuildContext context, String fallback) {
  if (context.canPop()) {
    context.pop();
  } else {
    context.go(fallback);
  }
}

/// "‹ parent" in accent.
class BackLink extends StatelessWidget {
  const BackLink({super.key, required this.label, required this.onTap});

  final String label;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Semantics(
      button: true,
      label: 'Back to $label',
      excludeSemantics: true,
      onTap: onTap,
      child: InkWell(
        key: const Key('back-link'),
        onTap: onTap,
        borderRadius: BorderRadius.circular(t.rControl * 0.6),
        child: Padding(
          padding: EdgeInsets.symmetric(vertical: t.sp * 0.75),
          child: Text(
            '‹ $label',
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
            style: TextStyle(
              fontFamily: StormTokens.sansFamily,
              fontSize: t.uiSize,
              color: t.accent,
            ),
          ),
        ),
      ),
    );
  }
}

/// The four most recent notes from every vault, with where each lives.
class _PhoneRecents extends ConsumerWidget {
  const _PhoneRecents({required this.vaultId});

  final String vaultId;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final t = context.tokens;
    final recents = (ref.watch(recentsProvider).value ?? const <RecentNote>[])
        .take(4)
        .toList();
    if (recents.isEmpty) return const SizedBox.shrink();
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Padding(
          padding: EdgeInsets.only(top: t.sp * 1.75, bottom: t.sp * 0.5),
          child: const SectionLabel('Recent'),
        ),
        for (final r in recents)
          _ListRow(
            key: Key('recent:${r.noteId}'),
            vertical: t.sp * 1.375,
            onTap: () => r.vaultId == vaultId
                ? context.push(Routes.note(r.vaultId, r.noteId))
                : context.go(Routes.note(r.vaultId, r.noteId)),
            meta: shortAge(
              DateTime.tryParse(r.modified) ?? DateTime.tryParse(r.openedAt),
            ),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              spacing: t.sp * 0.25,
              children: [
                Text(
                  r.displayTitle,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: TextStyle(
                    fontFamily: StormTokens.sansFamily,
                    fontSize: t.bodySize * 0.95,
                    fontWeight: FontWeight.w500,
                    color: t.text,
                  ),
                ),
                Text(
                  [
                    r.vaultName,
                    ...r.folder.split('/'),
                  ].where((p) => p.isNotEmpty).join(' · '),
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: TextStyle(
                    fontFamily: StormTokens.sansFamily,
                    fontSize: t.codeSize,
                    color: t.text3,
                  ),
                ),
              ],
            ),
          ),
      ],
    );
  }
}

/// "2h", "4d": how long ago, as a list's trailing meta.
String shortAge(DateTime? then) {
  if (then == null) return '';
  final d = DateTime.now().difference(then.toLocal());
  if (d.inMinutes < 1) return 'now';
  if (d.inHours < 1) return '${d.inMinutes}m';
  if (d.inDays < 1) return '${d.inHours}h';
  return '${d.inDays}d';
}

/// A divided phone row with a mono meta on the right.
class _ListRow extends StatelessWidget {
  const _ListRow({
    super.key,
    required this.child,
    required this.vertical,
    this.leading,
    this.meta,
    this.unseen = false,
    this.trailing,
    this.onTap,
    this.onLongPress,
  });

  final Widget child;
  final double vertical;
  final Widget? leading;
  final String? meta;
  final bool unseen;
  final Widget? trailing;
  final VoidCallback? onTap;
  final VoidCallback? onLongPress;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return InkWell(
      onTap: onTap,
      onLongPress: onLongPress,
      child: Container(
        padding: EdgeInsets.symmetric(vertical: vertical),
        decoration: BoxDecoration(
          border: Border(
            bottom: BorderSide(color: t.border, width: t.bw),
          ),
        ),
        child: Row(
          spacing: t.sp * 1.5,
          children: [
            ?leading,
            Expanded(child: child),
            if (unseen) const UnseenDot(),
            ?trailing,
            if (meta != null && meta!.isNotEmpty)
              Text(
                meta!,
                style: TextStyle(
                  fontFamily: StormTokens.monoFamily,
                  fontSize: t.labelSize,
                  color: t.text3,
                ),
              ),
          ],
        ),
      ),
    );
  }
}

/// One row of the phone's folder list: folders first with their counts,
/// then notes with their ages.
class EntryTile extends ConsumerWidget {
  const EntryTile({super.key, required this.entry, this.unseen = false});

  final BrowseEntry entry;

  /// Slice 8 feeds this from the agent-writes data.
  final bool unseen;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final t = context.tokens;
    final vaultId = VaultGate.of(context);
    final name = Text(
      entry.name,
      maxLines: 1,
      overflow: TextOverflow.ellipsis,
      style: TextStyle(
        fontFamily: StormTokens.sansFamily,
        fontSize: t.bodySize * 0.95,
        color: t.text,
      ),
    );
    final mark = SizedBox(
      width: t.sp * 2,
      child: Center(
        child: entry.isFolder
            ? const Twisty(open: false)
            : Text(
                '·',
                style: TextStyle(
                  fontFamily: StormTokens.sansFamily,
                  fontSize: t.codeSize,
                  color: t.text3,
                ),
              ),
      ),
    );

    if (entry.isFolder) {
      return _ListRow(
        key: ValueKey('folder:${entry.path}'),
        vertical: t.sp * 1.5,
        leading: mark,
        unseen: unseen,
        meta: '${entry.childCount}',
        onTap: () => context.push(Routes.folder(vaultId, entry.path)),
        onLongPress: () => showFolderActions(context, ref, vaultId, entry),
        child: name,
      );
    }

    final note = entry.note!;
    final pinned = ref.watch(pinnedNotesProvider).value ?? const <String>{};
    return _ListRow(
      key: ValueKey('note:${note.id}'),
      vertical: t.sp * 1.5,
      leading: mark,
      unseen: unseen,
      meta: shortAge(DateTime.tryParse(note.modified)),
      trailing: pinned.contains(note.id)
          ? Icon(LucideIcons.pin, size: t.labelSize, color: t.accent)
          : null,
      onTap: () => context.push(Routes.note(vaultId, note.id)),
      child: name,
    );
  }
}

class BrowseEntry {
  const BrowseEntry.folder(this.name, this.path) : note = null, childCount = 0;
  const BrowseEntry.folderWith(this.name, this.path, this.childCount)
    : note = null;
  const BrowseEntry.note(this.name, this.note) : path = '', childCount = 0;

  final String name;
  final String path;
  final NoteMeta? note;
  final int childCount;

  bool get isFolder => note == null;
}

/// Everything directly inside [folder] — one level, not the whole subtree.
///
/// Derived from note paths, exactly as the vault's folders are: the server
/// has no separate folder record because the vault is a directory of files.
List<BrowseEntry> childrenOfFolder(
  List<NoteMeta> notes,
  String folder, [
  List<String> knownFolders = const [],
]) => _childrenOf(notes, folder, knownFolders);

List<BrowseEntry> _childrenOf(
  List<NoteMeta> notes,
  String folder,
  List<String> knownFolders,
) {
  final prefix = folder.isEmpty ? '' : '$folder/';
  final folders = <String, int>{};
  final direct = <BrowseEntry>[];

  for (final note in notes) {
    // `_storm/` is Storm's own configuration, not the user's notes.
    if (isVaultConfigPath(note.path)) continue;
    if (!note.path.startsWith(prefix)) continue;
    final rest = note.path.substring(prefix.length);
    if (rest.isEmpty) continue;

    final slash = rest.indexOf('/');
    if (slash < 0) {
      direct.add(BrowseEntry.note(_stripExtension(rest), note));
    } else {
      final name = rest.substring(0, slash);
      folders[name] = (folders[name] ?? 0) + 1;
    }
  }

  // Folders the server knows about that no note put here — the empty ones.
  for (final known in knownFolders) {
    if (isVaultConfigPath('$known/')) continue;
    if (!known.startsWith(prefix)) continue;
    final rest = known.substring(prefix.length);
    if (rest.isEmpty || rest.contains('/')) continue;
    folders.putIfAbsent(rest, () => 0);
  }

  final folderEntries =
      folders.entries
          .map((e) => BrowseEntry.folderWith(e.key, '$prefix${e.key}', e.value))
          .toList()
        ..sort((a, b) => a.name.toLowerCase().compareTo(b.name.toLowerCase()));

  direct.sort((a, b) => a.name.toLowerCase().compareTo(b.name.toLowerCase()));

  // Folders first, then notes — matching Obsidian.
  return [...folderEntries, ...direct];
}

String _stripExtension(String fileName) => fileName.endsWith('.md')
    ? fileName.substring(0, fileName.length - 3)
    : fileName;

Future<void> showFolderActions(
  BuildContext context,
  WidgetRef ref,
  String vaultId,
  BrowseEntry entry,
) async {
  final action = await showModalBottomSheet<String>(
    context: context,
    builder: (c) => SafeArea(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          ListTile(
            leading: const Icon(LucideIcons.pencil_line),
            title: Text('Rename “${entry.name}”'),
            onTap: () => Navigator.pop(c, 'rename'),
          ),
          ListTile(
            leading: Icon(
              LucideIcons.trash_2,
              color: Theme.of(c).colorScheme.error,
            ),
            title: const Text('Delete folder'),
            onTap: () => Navigator.pop(c, 'delete'),
          ),
        ],
      ),
    ),
  );
  if (action == null || !context.mounted) return;

  final api = ref.read(apiProvider);
  if (api == null) return;

  if (action == 'rename') {
    final name = await promptForPath(
      context,
      title: 'Rename folder',
      initial: entry.name,
      isFolder: true,
    );
    if (name == null || !context.mounted) return;
    final parent = entry.path.contains('/')
        ? entry.path.substring(0, entry.path.lastIndexOf('/'))
        : '';
    final to = parent.isEmpty ? name : '$parent/$name';
    try {
      await api.renameFolder(vaultId, entry.path, to);
      ref.read(vaultRevisionProvider.notifier).state++;
      ref.invalidate(treeProvider);
    } catch (e) {
      if (context.mounted) _toast(context, describeFailure(e));
    }
    return;
  }

  try {
    await api.deleteFolder(vaultId, entry.path);
    ref.read(vaultRevisionProvider.notifier).state++;
    ref.invalidate(treeProvider);
  } catch (e) {
    // The server refuses a folder that still holds notes rather than taking
    // them with it. Surfacing its wording keeps the count accurate.
    if (context.mounted) _toast(context, describeFailure(e));
  }
}

/// Creates a folder inside [parent] (`''` at the vault root).
Future<void> createFolder(
  BuildContext context,
  WidgetRef ref,
  String vaultId,
  String parent,
) async {
  final name = await promptForPath(
    context,
    title: 'New folder',
    initial: '',
    isFolder: true,
  );
  if (name == null || name.trim().isEmpty || !context.mounted) return;

  final api = ref.read(apiProvider);
  if (api == null) return;
  final path = parent.isEmpty ? name : '$parent/$name';
  try {
    await api.createFolder(vaultId, path);
    ref.read(vaultRevisionProvider.notifier).state++;
    ref.invalidate(treeProvider);
  } catch (e) {
    if (context.mounted) _toast(context, describeFailure(e));
  }
}

void _toast(BuildContext context, String message) => ScaffoldMessenger.of(
  context,
).showSnackBar(SnackBar(content: Text(message)));
