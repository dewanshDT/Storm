import '../../agent/agent_models.dart';
import '../../agent/agent_state.dart' show AgentOverview;
import '../../agent/sidebar_rules.dart';
import '../../api/models.dart' show RecentNote, TagCount;
import '../../keyboard/storm_commands.dart' show StormCommand;
import '../../state/frecency.dart';
import '../../state/match.dart';
import '../../state/palette_titles.dart';
import '../settings/settings_shell.dart' show SettingsPageInfo;

/// What the palette's rows are built from, and how it ranks them (decision
/// 156g–i, m). Pure, so every rule is tested without a widget.

/// Where a typed prefix narrows the list (156h).
enum PaletteMode { all, commands, sessions, tags, folders, help }

const kPalettePrefixes = {
  '>': PaletteMode.commands,
  '@': PaletteMode.sessions,
  '#': PaletteMode.tags,
  '/': PaletteMode.folders,
  '?': PaletteMode.help,
};

/// The mode a query's first character picks, and the rest of it.
(PaletteMode, String) parsePaletteQuery(String raw) {
  final q = raw.trimLeft();
  if (q.isNotEmpty) {
    if (kPalettePrefixes[q[0]] case final mode?) {
      return (mode, q.substring(1).trim());
    }
  }
  return (PaletteMode.all, q.trim());
}

/// The groups, in the order they are drawn: the empty query's three, then
/// the fixed order of 156g. In note text (7) arrives with 156-3.
enum PaletteGroup {
  needsYou('Needs you'),
  recent('Recent'),
  suggested('Suggested'),
  prefixes('Narrow to'),
  notes('Notes'),
  sessions('Sessions'),
  commands('Commands'),
  settings('Settings'),
  foldersTags('Folders & tags'),
  hosts('Hosts'),
  create('');

  const PaletteGroup(this.title);

  final String title;
}

/// What a row does when it is run.
sealed class PaletteTarget {
  const PaletteTarget();
}

class NoteTarget extends PaletteTarget {
  const NoteTarget(this.vaultId, this.noteId);
  final String vaultId;
  final String noteId;
}

class SessionTarget extends PaletteTarget {
  const SessionTarget(this.sessionId);
  final String sessionId;
}

class CommandTarget extends PaletteTarget {
  const CommandTarget(this.command);
  final StormCommand command;
}

class SettingsTarget extends PaletteTarget {
  const SettingsTarget(this.pageId);
  final String pageId;
}

class FolderTarget extends PaletteTarget {
  const FolderTarget(this.vaultId, this.path);
  final String vaultId;
  final String path;
}

class TagTarget extends PaletteTarget {
  const TagTarget(this.tag);
  final String tag;
}

class HostTarget extends PaletteTarget {
  const HostTarget(this.hostId);
  final String hostId;
}

class CreateNoteTarget extends PaletteTarget {
  const CreateNoteTarget(this.name);
  final String name;
}

class ShowAllTarget extends PaletteTarget {
  const ShowAllTarget(this.group);
  final PaletteGroup group;
}

class PrefixTarget extends PaletteTarget {
  const PrefixTarget(this.prefix);
  final String prefix;
}

/// How a row's words are coloured; amber is for tags only (153e).
enum PaletteTone { plain, tag, loud }

class PaletteRow {
  const PaletteRow({
    required this.id,
    required this.group,
    required this.label,
    required this.target,
    this.sub,
    this.vault,
    this.hint,
    this.tone = PaletteTone.plain,
  });

  /// Stable across rebuilds, so the selection follows the row, not the
  /// index (156l).
  final String id;
  final PaletteGroup group;
  final String label;
  final String? sub;

  /// The vault's name, drawn as a `VaultTag`.
  final String? vault;

  /// The chord, right-aligned.
  final String? hint;
  final PaletteTone tone;
  final PaletteTarget target;

  /// The frecency id of what it opens, or null for a row that only steers
  /// the palette.
  String? get pickId => switch (target) {
    NoteTarget(:final vaultId, :final noteId) => noteItemId(vaultId, noteId),
    SessionTarget(:final sessionId) => 'session:$sessionId',
    CommandTarget(:final command) => 'command:${command.id}',
    SettingsTarget(:final pageId) => 'settings:$pageId',
    FolderTarget(:final vaultId, :final path) => 'folder:$vaultId/$path',
    TagTarget(:final tag) => 'tag:$tag',
    HostTarget(:final hostId) => 'host:$hostId',
    _ => null,
  };
}

class PaletteSection {
  const PaletteSection(this.group, this.rows, {this.total, this.title});

  final PaletteGroup group;

  /// What is drawn, the "Show all n" row included.
  final List<PaletteRow> rows;
  final int? total;
  final String? title;

  String get heading => title ?? group.title;
}

class PaletteView {
  const PaletteView({
    required this.sections,
    this.offline = false,
    this.noMatch,
  });

  final List<PaletteSection> sections;
  final bool offline;

  /// "No matches for '{q}'", when nothing but the create row matched.
  final String? noMatch;

  List<PaletteRow> get rows => [for (final s in sections) ...s.rows];

  /// Matches, not counting "Show all" or the create row: what the live
  /// region announces.
  int get count => [
    for (final r in rows)
      if (r.target is! ShowAllTarget && r.target is! CreateNoteTarget) r,
  ].length;
}

/// Everything the palette searches, as it stands.
class PaletteSources {
  const PaletteSources({
    required this.titles,
    required this.commands,
    required this.settings,
    this.overview,
    this.tags = const [],
    this.recents = const [],
    this.activeVault = '',
    this.activeVaultName = '',
    this.frecency = const Frecency(),
    required this.now,
  });

  final PaletteTitles titles;

  /// The registry's, already filtered by `when`.
  final List<StormCommand> commands;
  final List<SettingsPageInfo> settings;
  final AgentOverview? overview;
  final List<TagCount> tags;
  final List<RecentNote> recents;
  final String activeVault;
  final String activeVaultName;
  final Frecency frecency;
  final DateTime now;

  List<AgentSession> get sessions => overview?.sessions ?? const [];
}

/// Rows a group shows before "Show all n" (156g).
const kPaletteGroupCap = 5;

/// A prefixed list has one group, so it shows this many instead.
const kPaletteNarrowCap = 100;

/// Commands the empty query suggests, the first three that can run.
const kSuggestedCommands = [
  'new-note',
  'new-session',
  'switch-vault',
  'open-agents',
  'open-notes',
  'theme',
];

/// The status in words (AM44): "needs you · question", "running".
String sessionStatusWords(AgentSession s, DateTime now, AgentOverview? o) {
  final tier = tierOf(s, now);
  final reason = reasonLine(s, now: now, tier: tier, o: o)?.text;
  if (tier == SidebarTier.needsYou) {
    return reason == null ? 'needs you' : 'needs you · $reason';
  }
  return reason ?? s.statusLabel.toLowerCase();
}

/// Needs you, then running, then ended (156g).
int sessionBucket(AgentSession s) => needsYou(s)
    ? 0
    : s.ended
    ? 2
    : 1;

String _folderOf(String path) {
  final cut = path.lastIndexOf('/');
  return cut < 0 ? '' : path.substring(0, cut);
}

PaletteRow noteRow(PaletteNote n, {PaletteGroup group = PaletteGroup.notes}) =>
    PaletteRow(
      id: 'note:${n.vaultId}/${n.meta.id}',
      group: group,
      label: n.name,
      sub: _folderOf(n.meta.path),
      vault: n.vaultName,
      target: NoteTarget(n.vaultId, n.meta.id),
    );

PaletteRow recentRow(RecentNote r) {
  final file = r.path.split('/').last;
  final name = r.title.isNotEmpty
      ? r.title
      : (file.endsWith('.md') ? file.substring(0, file.length - 3) : file);
  return PaletteRow(
    id: 'recent:${r.vaultId}/${r.noteId}',
    group: PaletteGroup.recent,
    label: name,
    sub: _folderOf(r.path),
    vault: r.vaultName,
    target: NoteTarget(r.vaultId, r.noteId),
  );
}

PaletteRow sessionRow(
  AgentSession s,
  PaletteSources src, {
  PaletteGroup group = PaletteGroup.sessions,
}) {
  final o = src.overview;
  final host = o?.hostName(s.hostId) ?? s.hostName;
  return PaletteRow(
    id: 'session:${s.id}',
    group: group,
    label: s.displayName,
    sub: [sessionStatusWords(s, src.now, o), ?host, s.workspace].join(' · '),
    tone: needsYou(s) ? PaletteTone.loud : PaletteTone.plain,
    target: SessionTarget(s.id),
  );
}

PaletteRow commandRow(StormCommand c, {PaletteGroup? group}) => PaletteRow(
  id: 'command:${c.id}',
  group: group ?? PaletteGroup.commands,
  label: c.label,
  hint: c.hint,
  target: CommandTarget(c),
);

PaletteRow settingsRow(SettingsPageInfo p) => PaletteRow(
  id: 'settings:${p.id}',
  group: PaletteGroup.settings,
  label: p.label,
  sub: 'Settings',
  target: SettingsTarget(p.id),
);

PaletteRow folderRow(PaletteFolder f) => PaletteRow(
  id: 'folder:${f.vaultId}/${f.path}',
  group: PaletteGroup.foldersTags,
  label: f.path,
  sub: 'Folder',
  vault: f.vaultName,
  target: FolderTarget(f.vaultId, f.path),
);

PaletteRow tagRow(TagCount t) => PaletteRow(
  id: 'tag:${t.tag}',
  group: PaletteGroup.foldersTags,
  label: '#${t.tag}',
  sub: t.count == 1 ? '1 note' : '${t.count} notes',
  tone: PaletteTone.tag,
  target: TagTarget(t.tag),
);

PaletteRow hostRow(AgentHost h) => PaletteRow(
  id: 'host:${h.id}',
  group: PaletteGroup.hosts,
  label: h.name,
  sub: 'Host · ${h.status}',
  target: HostTarget(h.id),
);

const _prefixHelp = [
  ('>', 'Commands'),
  ('@', 'Sessions'),
  ('#', 'Tags'),
  ('/', 'Folders'),
];

/// The palette's list for [raw]. [expanded] are the groups whose "Show all"
/// was run.
PaletteView buildPaletteView(
  PaletteSources src,
  String raw, {
  Set<PaletteGroup> expanded = const {},
}) {
  final (mode, q) = parsePaletteQuery(raw);
  final want = q.toLowerCase();
  final offline = src.titles.offline;
  final narrow = mode != PaletteMode.all;
  final sections = <PaletteSection>[];

  void add(PaletteGroup group, List<PaletteRow> rows, {String? title}) {
    if (rows.isEmpty) return;
    final cap = narrow ? kPaletteNarrowCap : kPaletteGroupCap;
    if (rows.length <= cap || expanded.contains(group)) {
      sections.add(PaletteSection(group, rows, title: title));
      return;
    }
    sections.add(
      PaletteSection(
        group,
        [
          ...rows.take(cap),
          PaletteRow(
            id: 'more:${group.name}',
            group: group,
            label: 'Show all ${rows.length}',
            target: ShowAllTarget(group),
          ),
        ],
        total: rows.length,
        title: title,
      ),
    );
  }

  int byPick(String a, String b) => src.frecency.compare(a, b);

  if (mode == PaletteMode.help) {
    add(PaletteGroup.prefixes, [
      for (final (p, what) in _prefixHelp)
        PaletteRow(
          id: 'prefix:$p',
          group: PaletteGroup.prefixes,
          label: '$p  $what',
          sub: 'Type $p to search only ${what.toLowerCase()}',
          target: PrefixTarget(p),
        ),
    ]);
    return PaletteView(sections: sections, offline: offline);
  }

  if (mode == PaletteMode.all && want.isEmpty) {
    add(PaletteGroup.needsYou, [
      for (final s in src.sessions)
        if (needsYou(s)) sessionRow(s, src, group: PaletteGroup.needsYou),
    ]);
    add(PaletteGroup.recent, [for (final r in src.recents) recentRow(r)]);
    final suggested = [
      for (final id in kSuggestedCommands)
        ?src.commands.where((c) => c.id == id).firstOrNull,
    ].take(3);
    add(PaletteGroup.suggested, [
      for (final c in suggested) commandRow(c, group: PaletteGroup.suggested),
    ]);
    return PaletteView(sections: sections, offline: offline);
  }

  if (mode == PaletteMode.all) {
    final notes = rankMatches(
      src.titles.notes,
      want,
      (n) => [
        MatchField(n.name),
        MatchField(n.meta.path, worst: MatchTier.substring),
      ],
      tieBreak: (a, b) {
        final av = a.vaultId == src.activeVault ? 0 : 1;
        final bv = b.vaultId == src.activeVault ? 0 : 1;
        if (av != bv) return av - bv;
        return byPick(
          noteItemId(a.vaultId, a.meta.id),
          noteItemId(b.vaultId, b.meta.id),
        );
      },
    );
    add(PaletteGroup.notes, [for (final n in notes) noteRow(n)]);
  }

  if (mode == PaletteMode.all || mode == PaletteMode.sessions) {
    final sessions = want.isEmpty
        ? [...src.sessions]
        : rankMatches(
            src.sessions,
            want,
            (s) {
              final context = s.context?.title;
              return [
                MatchField(s.displayName),
                MatchField(s.name),
                if (context != null && context.isNotEmpty) MatchField(context),
                MatchField(
                  providerLabel(s.provider),
                  worst: MatchTier.substring,
                ),
                MatchField(
                  src.overview?.hostName(s.hostId) ?? s.hostName ?? '',
                  worst: MatchTier.substring,
                ),
                MatchField(s.workspace, worst: MatchTier.substring),
              ];
            },
            tieBreak: (a, b) => byPick('session:${a.id}', 'session:${b.id}'),
          );
    // A stable sort: the bucket first, the match order within it.
    final ordered = [for (final (i, s) in sessions.indexed) (i, s)]
      ..sort((a, b) {
        final d = sessionBucket(a.$2) - sessionBucket(b.$2);
        return d != 0 ? d : a.$1 - b.$1;
      });
    add(PaletteGroup.sessions, [
      for (final (_, s) in ordered) sessionRow(s, src),
    ]);
  }

  if (mode == PaletteMode.all || mode == PaletteMode.commands) {
    final commands = want.isEmpty
        ? src.commands
        : rankMatches(
            src.commands,
            want,
            (c) => [
              MatchField(c.label),
              for (final s in c.synonyms)
                MatchField(s, worst: MatchTier.wordStart),
            ],
            tieBreak: (a, b) => byPick('command:${a.id}', 'command:${b.id}'),
          );
    add(PaletteGroup.commands, [for (final c in commands) commandRow(c)]);
  }

  if (mode == PaletteMode.all) {
    final pages = rankMatches(
      src.settings,
      want,
      (p) => [MatchField(p.label)],
      tieBreak: (a, b) => byPick('settings:${a.id}', 'settings:${b.id}'),
    );
    add(PaletteGroup.settings, [for (final p in pages) settingsRow(p)]);
  }

  if (mode == PaletteMode.all || mode == PaletteMode.folders) {
    final folders = want.isEmpty
        ? src.titles.folders
        : rankMatches(
            src.titles.folders,
            want,
            (f) => [MatchField(f.path.split('/').last), MatchField(f.path)],
            tieBreak: (a, b) {
              final av = a.vaultId == src.activeVault ? 0 : 1;
              final bv = b.vaultId == src.activeVault ? 0 : 1;
              return av - bv;
            },
          );
    final tags = mode == PaletteMode.folders
        ? const <TagCount>[]
        : rankMatches(src.tags, want, (t) => [MatchField(t.tag)]);
    add(PaletteGroup.foldersTags, [
      for (final f in folders) folderRow(f),
      for (final t in tags) tagRow(t),
    ], title: mode == PaletteMode.folders ? 'Folders' : null);
  }

  if (mode == PaletteMode.tags) {
    final tags = want.isEmpty
        ? src.tags
        : rankMatches(src.tags, want, (t) => [MatchField(t.tag)]);
    add(PaletteGroup.foldersTags, [
      for (final t in tags) tagRow(t),
    ], title: 'Tags');
  }

  if (mode == PaletteMode.all) {
    final hosts = rankMatches(
      src.overview?.hosts ?? const <AgentHost>[],
      want,
      (h) => [MatchField(h.name)],
    );
    add(PaletteGroup.hosts, [for (final h in hosts) hostRow(h)]);
  }

  final matched = sections.isNotEmpty;
  if (mode == PaletteMode.all && src.activeVault.isNotEmpty) {
    final exact = src.titles.notes.any(
      (n) => n.vaultId == src.activeVault && n.name.toLowerCase() == want,
    );
    if (!exact) {
      final where = src.activeVaultName.isEmpty
          ? ''
          : ' in ${src.activeVaultName}';
      sections.add(
        PaletteSection(PaletteGroup.create, [
          PaletteRow(
            id: 'create',
            group: PaletteGroup.create,
            label: 'Create note “$q”$where',
            target: CreateNoteTarget(q),
          ),
        ]),
      );
    }
  }
  return PaletteView(
    sections: sections,
    offline: offline,
    noMatch: matched || want.isEmpty ? null : 'No matches for ‘$q’',
  );
}
