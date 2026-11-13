import 'package:flutter_test/flutter_test.dart';

import 'package:storm/agent/agent_models.dart';
import 'package:storm/agent/agent_state.dart' show AgentOverview;
import 'package:storm/api/models.dart';
import 'package:storm/keyboard/storm_commands.dart';
import 'package:storm/state/frecency.dart';
import 'package:storm/state/match.dart';
import 'package:storm/state/palette_titles.dart';
import 'package:storm/ui/palette/palette_model.dart';
import 'package:storm/ui/settings/settings_shell.dart' show kSettingsPages;

import 'agent_fakes.dart';

/// The palette's pure rules (decision 156g–i, m): the scorer, frecency,
/// prefixes, groups, caps and states, without a widget.
void main() {
  group('the scorer', () {
    test('tiers: exact, prefix, word start, substring, subsequence', () {
      expect(matchTier('Design', 'design'), MatchTier.exact);
      expect(matchTier('Design System', 'design'), MatchTier.prefix);
      expect(matchTier('Our Design', 'design'), MatchTier.wordStart);
      expect(matchTier('daily/2026-design', 'design'), MatchTier.wordStart);
      expect(matchTier('Redesign', 'design'), MatchTier.substring);
      expect(matchTier('Deep Sign', 'dsign'), MatchTier.subsequence);
      expect(matchTier('Notes', 'zz'), isNull);
    });

    test('every query word starting a word, in order, is a word start', () {
      expect(matchTier('Design System Notes', 'des sys'), MatchTier.wordStart);
      expect(
        matchTier('Design System Notes', 'sys des'),
        isNot(MatchTier.wordStart),
      );
    });

    test('a better tier always wins; within one, shorter, then a-z', () {
      final ranked = rankMatches(
        ['Redesign', 'Design System', 'Design', 'Our Design', 'Deep sign'],
        'design',
        (s) => [MatchField(s)],
      );
      expect(ranked, [
        'Design',
        'Design System',
        'Our Design',
        'Redesign',
        'Deep sign',
      ]);
    });

    test('the tie-break never lifts an item over a better tier', () {
      final ranked = rankMatches(
        ['Plan', 'Plans long', 'Replan'],
        'plan',
        (s) => [MatchField(s)],
        // Prefer the longest, as a strong frecency would.
        tieBreak: (a, b) => b.length - a.length,
      );
      expect(ranked, ['Plan', 'Plans long', 'Replan']);
      final within = rankMatches(
        ['Plans', 'Plan long'],
        'plan',
        (s) => [MatchField(s)],
        tieBreak: (a, b) => b.length - a.length,
      );
      expect(within, ['Plan long', 'Plans'], reason: 'both are prefixes');
    });

    test('an earlier field beats a later one; worst caps a field', () {
      final items = [('alpha', 'zeta'), ('zeta', 'alpha')];
      final ranked = rankMatches(
        items,
        'zeta',
        (i) => [MatchField(i.$1), MatchField(i.$2)],
      );
      expect(ranked.first.$1, 'zeta');
      expect(
        rankMatches(
          ['projects/storm/design.md'],
          'psd',
          (s) => [MatchField(s, worst: MatchTier.substring)],
        ),
        isEmpty,
      );
    });
  });

  group('frecency', () {
    final now = DateTime.utc(2026, 11, 13);

    test('two uses outrank one, and a pick outranks an open', () {
      var f = const Frecency();
      f = f.used('a', now: now).used('a', now: now).used('b', now: now);
      expect(f.compare('a', 'b'), lessThan(0));
      var g = const Frecency();
      g = g
          .used('open', now: now)
          .used('pick', weight: Frecency.pickWeight, now: now);
      expect(g.compare('pick', 'open'), lessThan(0));
    });

    test('a use halves in weight every 30 days', () {
      final f = const Frecency()
          .used('old', weight: 2, now: now.subtract(const Duration(days: 31)))
          .used('new', now: now);
      expect(f.compare('new', 'old'), lessThan(0));
      final g = const Frecency()
          .used('old', weight: 2, now: now.subtract(const Duration(days: 29)))
          .used('new', now: now);
      expect(g.compare('old', 'new'), lessThan(0));
    });

    test('never used ties, and sorts after anything used', () {
      final f = const Frecency().used('a', now: now);
      expect(f.compare('x', 'y'), 0);
      expect(f.compare('a', 'x'), lessThan(0));
    });

    test('a location names the item it shows', () {
      expect(frecencyIdOf('/v/v1/note/n1'), 'note:v1/n1');
      expect(frecencyIdOf('/agents/s/ags_1?tab=about'), 'session:ags_1');
      expect(frecencyIdOf('/settings/vaults'), 'settings:vaults');
      expect(frecencyIdOf('/v/v1/browse'), isNull);
    });
  });

  group('prefixes', () {
    test('each prefix picks its mode and drops itself', () {
      expect(parsePaletteQuery('>new'), (PaletteMode.commands, 'new'));
      expect(parsePaletteQuery('@ gate'), (PaletteMode.sessions, 'gate'));
      expect(parsePaletteQuery('#proj'), (PaletteMode.tags, 'proj'));
      expect(parsePaletteQuery('/Daily'), (PaletteMode.folders, 'Daily'));
      expect(parsePaletteQuery('?'), (PaletteMode.help, ''));
      expect(parsePaletteQuery('plain'), (PaletteMode.all, 'plain'));
    });
  });

  StormCommand command(
    String id,
    String label, {
    List<String> syn = const [],
  }) => StormCommand(
    id: id,
    label: label,
    synonyms: syn,
    group: StormCommandGroup.navigate,
    run: (_, _) {},
  );

  final commands = [
    command('search', 'Search'),
    command('new-note', 'New note', syn: ['create note']),
    command('new-session', 'New session'),
    command('switch-vault', 'Switch vault…'),
    command('open-agents', 'Open Agents'),
    command('theme', 'Theme', syn: ['dark mode']),
  ];

  PaletteNote note(String vault, String id, String path, {String title = ''}) =>
      PaletteNote(
        vaultId: vault,
        vaultName: vault == 'v1' ? 'Primary' : 'Work',
        meta: NoteMeta(
          id: id,
          path: path,
          title: title,
          version: 1,
          modified: '',
          size: 0,
        ),
      );

  AgentSession session(
    String id, {
    String name = 'gateway-impl',
    String status = 'running',
    String? attention,
    String? title,
  }) => AgentSession.fromJson({
    ...agentSession(id, name: name, status: status),
    'needs_attention': ?attention,
    'title': ?title,
  });

  PaletteSources sources({
    List<PaletteNote> notes = const [],
    List<PaletteFolder> folders = const [],
    List<AgentSession> sessions = const [],
    List<TagCount> tags = const [],
    List<RecentNote> recents = const [],
    bool offline = false,
    Frecency frecency = const Frecency(),
    PaletteText? text,
  }) => PaletteSources(
    titles: PaletteTitles(
      notes: notes,
      folders: folders,
      fetchedAt: DateTime.now(),
      offline: offline,
    ),
    commands: commands,
    settings: kSettingsPages,
    overview: AgentOverview(
      sessions: sessions,
      hosts: [AgentHost.fromJson(agentHost())],
    ),
    tags: tags,
    recents: recents,
    activeVault: 'v1',
    activeVaultName: 'Primary',
    frecency: frecency,
    text: text,
    now: DateTime.now(),
  );

  group('groups', () {
    test('a fixed order, and empty groups hidden', () {
      final view = buildPaletteView(
        sources(
          notes: [note('v1', 'n1', 'Build log.md')],
          sessions: [session('s1', name: 'build-fix')],
          folders: [
            const PaletteFolder(
              vaultId: 'v1',
              vaultName: 'Primary',
              path: 'Builds',
            ),
          ],
          tags: [const TagCount(tag: 'build', count: 3)],
        ),
        'build',
      );
      expect(view.sections.map((s) => s.group), [
        PaletteGroup.notes,
        PaletteGroup.sessions,
        PaletteGroup.foldersTags,
        PaletteGroup.hosts,
        PaletteGroup.create,
      ]);
      final tag = view.rows.firstWhere((r) => r.target is TagTarget);
      expect(tag.tone, PaletteTone.tag);
      expect(
        view.rows.where((r) => r.tone == PaletteTone.tag),
        [tag],
        reason: 'amber is for tags only',
      );
    });

    test('at most five rows a group, then Show all n', () {
      final notes = [
        for (var i = 0; i < 7; i++) note('v1', 'n$i', 'Plan $i.md'),
      ];
      final view = buildPaletteView(sources(notes: notes), 'plan');
      final section = view.sections.first;
      expect(section.rows.length, 6);
      expect(section.rows.last.label, 'Show all 7');
      expect(section.rows.last.target, isA<ShowAllTarget>());
      final open = buildPaletteView(
        sources(notes: notes),
        'plan',
        expanded: {PaletteGroup.notes},
      );
      expect(open.sections.first.rows.length, 7);
      expect(open.count, 7);
    });

    test('the active vault ranks first within a tier, never above one', () {
      final view = buildPaletteView(
        sources(
          notes: [
            note('v2', 'w1', 'Roadmap.md'),
            note('v1', 'p1', 'Roadmap.md'),
            note('v1', 'p2', 'Old roadmap.md'),
          ],
        ),
        'roadmap',
      );
      final notes = view.sections.first.rows;
      expect(notes.map((r) => (r.vault, r.label)), [
        ('Primary', 'Roadmap'),
        ('Work', 'Roadmap'),
        ('Primary', 'Old roadmap'),
      ]);
    });

    test('frecency breaks a tie', () {
      final picked = const Frecency().used(noteItemId('v1', 'b'));
      final view = buildPaletteView(
        sources(
          notes: [note('v1', 'a', 'Plan A.md'), note('v1', 'b', 'Plan B.md')],
          frecency: picked,
        ),
        'plan',
      );
      expect(view.sections.first.rows.first.label, 'Plan B');
    });

    test('sessions: needs you, then running, then ended, in words', () {
      final view = buildPaletteView(
        sources(
          sessions: [
            session('s1', name: 'gate-old', status: 'completed'),
            session('s2', name: 'gate-live'),
            session('s3', name: 'gate-ask', attention: 'question'),
          ],
        ),
        'gate',
      );
      final rows = view.sections
          .firstWhere((s) => s.group == PaletteGroup.sessions)
          .rows;
      expect(rows.map((r) => r.label), ['gate-ask', 'gate-live', 'gate-old']);
      expect(rows[0].sub, startsWith('needs you · question'));
      expect(rows[1].sub, startsWith('running'));
      expect(rows[2].sub, startsWith('completed'));
      expect(rows[0].tone, PaletteTone.loud);
    });

    test('a session matches on its title first, then its name', () {
      final view = buildPaletteView(
        sources(
          sessions: [
            session('s1', name: 'fix-login'),
            session('s2', name: 'other', title: 'Fix login flow'),
          ],
        ),
        'fix login',
      );
      final rows = view.sections
          .firstWhere((s) => s.group == PaletteGroup.sessions)
          .rows;
      expect(rows.map((r) => r.label), ['Fix login flow', 'fix-login']);
    });

    test('commands match synonyms and carry their chord', () {
      final view = buildPaletteView(sources(), 'dark');
      final row = view.rows.firstWhere((r) => r.target is CommandTarget);
      expect(row.label, 'Theme');
    });

    test('the create row is last, and gone on an exact title', () {
      final some = buildPaletteView(
        sources(notes: [note('v1', 'n1', 'Plan.md')]),
        'pla',
      );
      expect(some.sections.last.group, PaletteGroup.create);
      expect(some.rows.last.label, 'Create note “pla” in Primary');
      final exact = buildPaletteView(
        sources(notes: [note('v1', 'n1', 'Plan.md')]),
        'plan',
      );
      expect(exact.rows.where((r) => r.target is CreateNoteTarget), isEmpty);
    });

    test('no match says so, with the create row', () {
      final view = buildPaletteView(sources(), 'qqqqzz');
      expect(view.noMatch, 'No matches for ‘qqqqzz’');
      expect(view.rows.single.target, isA<CreateNoteTarget>());
    });
  });

  group('prefix narrowing', () {
    final src = sources(
      notes: [note('v1', 'n1', 'New ideas.md')],
      sessions: [session('s1', name: 'new-work')],
      folders: [
        const PaletteFolder(vaultId: 'v1', vaultName: 'Primary', path: 'News'),
      ],
      tags: [const TagCount(tag: 'news', count: 1)],
    );

    List<PaletteGroup> groups(String q) => [
      for (final s in buildPaletteView(src, q).sections) s.group,
    ];

    test('> is commands only', () {
      expect(groups('>new'), [PaletteGroup.commands]);
      expect(buildPaletteView(src, '>').rows.length, commands.length);
    });

    test('@ is sessions, # tags, / folders', () {
      expect(groups('@new'), [PaletteGroup.sessions]);
      final tags = buildPaletteView(src, '#new');
      expect(tags.sections.single.heading, 'Tags');
      expect(tags.rows.single.label, '#news');
      final folders = buildPaletteView(src, '/new');
      expect(folders.sections.single.heading, 'Folders');
      expect(folders.rows.single.label, 'News');
    });

    test('? lists the prefixes', () {
      final view = buildPaletteView(src, '?');
      expect(view.rows.map((r) => (r.target as PrefixTarget).prefix), [
        '>',
        '@',
        '#',
        '/',
      ]);
    });
  });

  group('states', () {
    test('the empty query: needs you, recent, three suggested', () {
      final view = buildPaletteView(
        sources(
          sessions: [
            session('s1', attention: 'permission_prompt'),
            session('s2'),
          ],
          recents: [
            const RecentNote(
              vaultId: 'v2',
              vaultName: 'Work',
              noteId: 'w1',
              path: 'Q4/Plan.md',
              title: '',
              modified: '',
              openedAt: '2026-11-13T10:00:00Z',
            ),
          ],
        ),
        '',
      );
      expect(view.sections.map((s) => s.group), [
        PaletteGroup.needsYou,
        PaletteGroup.recent,
        PaletteGroup.suggested,
      ]);
      expect(view.sections[0].rows.single.target, isA<SessionTarget>());
      expect(view.sections[1].rows.single.label, 'Plan');
      expect(view.sections[1].rows.single.vault, 'Work');
      expect(view.sections[2].rows.map((r) => r.label), [
        'New note',
        'New session',
        'Switch vault…',
      ]);
    });

    test('offline is said', () {
      expect(buildPaletteView(sources(offline: true), 'x').offline, isTrue);
      expect(buildPaletteView(sources(), 'x').offline, isFalse);
    });
  });

  group('in note text (156l)', () {
    CrossSearchHit hit(String vault, String id, String title, String snip) =>
        CrossSearchHit(
          vaultId: vault,
          vaultName: vault == 'v1' ? 'Primary' : 'Work',
          id: id,
          path: '$title.md',
          title: title,
          snippet: snip,
          tier: 3,
        );
    final notes = [note('v1', 'n1', 'Design Doc.md')];

    test('below every other group, and above only the create row', () {
      final view = buildPaletteView(
        sources(
          notes: notes,
          text: PaletteText(
            query: 'design',
            hits: [hit('v2', 'w9', 'Meeting', 'the <<design>> call')],
          ),
        ),
        'design',
      );
      final groups = [for (final s in view.sections) s.group];
      expect(groups.first, PaletteGroup.notes);
      expect(groups.sublist(groups.length - 2), [
        PaletteGroup.text,
        PaletteGroup.create,
      ]);
      final row = view.rows.firstWhere((r) => r.group == PaletteGroup.text);
      expect(row.label, 'Meeting');
      expect(row.snippet, 'the <<design>> call');
      expect(row.vault, 'Work');
      expect(row.target, isA<NoteTarget>());
      expect(view.count, 2);
    });

    test('a note the Notes group found by title is not listed again', () {
      final view = buildPaletteView(
        sources(
          notes: notes,
          text: PaletteText(
            query: 'design',
            hits: [hit('v1', 'n1', 'Design Doc', '<<Design>> Doc')],
          ),
        ),
        'design',
      );
      expect(
        view.sections.map((s) => s.group),
        isNot(contains(PaletteGroup.text)),
      );
    });

    test('pending is a skeleton, not a row; it holds back "No matches"', () {
      final view = buildPaletteView(
        sources(text: const PaletteText(query: 'zzz', pending: true)),
        'zzz',
      );
      final text = view.sections.firstWhere(
        (s) => s.group == PaletteGroup.text,
      );
      expect(text.pending, isTrue);
      expect(text.rows, isEmpty);
      expect(view.noMatch, isNull);
      final done = buildPaletteView(
        sources(text: const PaletteText(query: 'zzz')),
        'zzz',
      );
      expect(done.noMatch, isNotNull);
      expect(
        done.sections.map((s) => s.group),
        isNot(contains(PaletteGroup.text)),
      );
    });

    test('skipped vaults are one line; a failure is one quiet line', () {
      final skipped = buildPaletteView(
        sources(
          text: PaletteText(
            query: 'plan',
            hits: [hit('v1', 'p1', 'Plan', '<<plan>>')],
            skipped: const [
              SkippedVault(vaultId: 'v2', vaultName: 'Work', reason: 'busy'),
              SkippedVault(vaultId: 'v3', vaultName: 'Home', reason: 'busy'),
            ],
          ),
        ),
        'plan',
      );
      final text = skipped.sections.firstWhere(
        (s) => s.group == PaletteGroup.text,
      );
      expect(text.rows.single.label, 'Plan');
      expect(text.lines, ['Couldn’t search Work, Home']);

      final failed = buildPaletteView(
        sources(
          notes: notes,
          text: const PaletteText(query: 'design', failed: true),
        ),
        'design',
      );
      expect(failed.sections.first.group, PaletteGroup.notes);
      expect(
        failed.sections.firstWhere((s) => s.group == PaletteGroup.text).lines,
        ['Couldn’t search note text'],
      );
    });

    test('hidden offline, for another query, a prefix or one letter', () {
      final text = PaletteText(
        query: 'design',
        hits: [hit('v2', 'w9', 'Meeting', '<<design>>')],
      );
      bool shown(PaletteSources src, String raw) => buildPaletteView(
        src,
        raw,
      ).sections.any((s) => s.group == PaletteGroup.text);
      expect(shown(sources(text: text), 'design'), isTrue);
      expect(shown(sources(text: text, offline: true), 'design'), isFalse);
      expect(shown(sources(text: text), 'designs'), isFalse);
      expect(paletteTextQuery('>design'), isNull);
      expect(paletteTextQuery('d'), isNull);
      expect(paletteTextQuery('  de '), 'de');
    });
  });
}
