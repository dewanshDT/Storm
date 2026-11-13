import 'dart:async';
import 'dart:math' as math;

import 'package:flutter/foundation.dart' show kIsWeb;
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_lucide/flutter_lucide.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../../agent/agent_state.dart'
    show agentOverviewProvider, openAgentSession;
import '../../agent/agents_screen.dart' show launchAgentSession;
import '../../api/models.dart' show VaultInfo;
import '../../keyboard/storm_activators.dart';
import '../../keyboard/storm_commands.dart';
import '../../router.dart';
import '../../state/app_state.dart';
import '../../state/frecency.dart';
import '../../state/palette_text.dart';
import '../../state/palette_titles.dart';
import '../breakpoints.dart';
import '../settings/settings_shell.dart' show kSettingsPages;
import '../shell/create_here.dart' show createNote;
import '../tokens.dart';
import 'palette_model.dart';
import 'palette_rows.dart';

/// What the palette was closed with: the row to run, and whether it was
/// the secondary action (⌘Enter).
class PaletteChoice {
  const PaletteChoice(this.target, {this.secondary = false});

  final PaletteTarget target;
  final bool secondary;
}

/// The palette's route while it is up, so a second ⌘K doesn't stack one.
Route<PaletteChoice>? _route;

/// Opens the palette (decision 156a, n): a top-aligned dialog on the root
/// navigator at desk width, a full-screen route below it. [initial] is
/// what the field starts with: `>` for command mode.
///
/// [context] and [ref] are the shell's: what the chosen row runs against
/// once the palette is gone.
Future<void> openPalette(
  BuildContext context,
  WidgetRef ref, {
  String initial = '',
}) async {
  if (_route?.isActive ?? false) return;
  final before = FocusManager.instance.primaryFocus;
  final router = GoRouter.of(context);
  final from = router.state.uri.toString();
  refreshPaletteTitlesIfStale(ref);
  // Asked before the palette takes focus: Back's `when` looks at what has it.
  final commands = [
    for (final c in stormCommands())
      if (c.enabled(context, ref)) c,
  ];
  final palette = StormPalette(initial: initial, commands: commands);
  final route = context.isExpanded
      ? _dialogRoute(context, palette)
      : PageRouteBuilder<PaletteChoice>(
          fullscreenDialog: true,
          transitionDuration: context.tokens.duration,
          reverseTransitionDuration: context.tokens.duration,
          pageBuilder: (_, _, _) => palette,
          transitionsBuilder: (_, animation, _, child) =>
              FadeTransition(opacity: animation, child: child),
        );
  _route = route;
  final choice = await Navigator.of(context, rootNavigator: true).push(route);
  if (identical(_route, route)) _route = null;
  if (!context.mounted) return;
  // Back to where it was first: Read/Edit and Find act on what has focus.
  if (before != null && before.context != null && before.canRequestFocus) {
    before.requestFocus();
  }
  if (choice == null) return;
  await WidgetsBinding.instance.endOfFrame;
  if (!context.mounted) return;
  await runPaletteChoice(context, ref, choice);
  if (!context.mounted) return;
  if (router.state.uri.toString() != from && before != null) {
    // Somewhere new: the old focus is not the destination's, so the layer
    // (and whatever the page autofocuses) takes it.
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (before.hasPrimaryFocus) {
        before.unfocus(disposition: UnfocusDisposition.scope);
      }
    });
  }
}

/// What `showGeneralDialog` would push, kept so [openPalette] can tell
/// whether it is still up.
Route<PaletteChoice> _dialogRoute(BuildContext context, Widget palette) {
  final t = context.tokens;
  return RawDialogRoute<PaletteChoice>(
    barrierDismissible: true,
    barrierLabel: 'Close',
    barrierColor: Theme.of(context).colorScheme.scrim.withValues(alpha: 0.4),
    transitionDuration: t.duration,
    pageBuilder: (dialog, _, _) {
      final size = MediaQuery.sizeOf(dialog);
      final top = math.min(t.sp * 12, size.height * 0.12);
      return Align(
        alignment: Alignment.topCenter,
        child: Padding(
          padding: EdgeInsets.only(top: top, bottom: t.sp * 3),
          child: SizedBox(
            key: const Key('palette-dialog'),
            width: math.min(t.sp * 80, size.width - t.sp * 4),
            child: DecoratedBox(
              decoration: BoxDecoration(
                borderRadius: BorderRadius.circular(t.rCard),
                boxShadow: t.shadow,
              ),
              child: Material(
                color: t.surface,
                clipBehavior: Clip.antiAlias,
                shape: RoundedRectangleBorder(
                  borderRadius: BorderRadius.circular(t.rCard),
                  side: BorderSide(color: t.border, width: t.bw),
                ),
                child: ConstrainedBox(
                  constraints: BoxConstraints(
                    maxHeight: math.max(
                      t.sp * 20,
                      size.height - top - t.sp * 3,
                    ),
                  ),
                  child: palette,
                ),
              ),
            ),
          ),
        ),
      );
    },
  );
}

/// Runs what the palette was closed with, against the shell's [context].
Future<void> runPaletteChoice(
  BuildContext context,
  WidgetRef ref,
  PaletteChoice choice,
) async {
  switch (choice.target) {
    case NoteTarget(:final vaultId, :final noteId):
      if (choice.secondary) {
        // Orchestration only (156q): the launcher, prefilled; never a spawn.
        await launchAgentSession(
          context,
          ref,
          contextNote: (vaultId: vaultId, noteId: noteId),
        );
      } else {
        context.go(Routes.note(vaultId, noteId));
      }
    case SessionTarget(:final sessionId):
      openAgentSession(context, sessionId);
    case CommandTarget(:final command):
      command.run(context, ref);
    case SettingsTarget(:final pageId):
      context.go(Routes.settingsPage(pageId));
    case FolderTarget(:final vaultId, :final path):
      context.go(Routes.folder(vaultId, path));
    case TagTarget(:final tag):
      final vaultId = ref.read(activeVaultProvider);
      if (vaultId.isEmpty) return;
      ref.read(selectedTagProvider.notifier).state = tag;
      context.go(Routes.tags(vaultId));
    case HostTarget():
      context.go(Routes.settingsPage('hosts'));
    case CreateNoteTarget(:final name):
      await createNote(context, ref, name: name);
    case ShowAllTarget() || PrefixTarget():
      break;
  }
}

class _Move extends Intent {
  const _Move(this.by);
  final int by;
}

class _Run extends Intent {
  const _Run();
}

class _Create extends Intent {
  const _Create();
}

class _Secondary extends Intent {
  const _Secondary();
}

class _Escape extends Intent {
  const _Escape();
}

/// The field and the list. Focus never leaves the field; the arrows move
/// the selection, which is held by row id so a list that updates under it
/// keeps it (156l).
class StormPalette extends ConsumerStatefulWidget {
  const StormPalette({super.key, this.initial = '', this.commands = const []});

  final String initial;

  /// The registry's commands that can run where the palette was opened.
  final List<StormCommand> commands;

  @override
  ConsumerState<StormPalette> createState() => _StormPaletteState();
}

class _StormPaletteState extends ConsumerState<StormPalette> {
  late final _controller = TextEditingController(text: widget.initial);
  final _field = FocusNode(debugLabel: 'palette-field');
  final _scroll = ScrollController();
  final _keys = <String, GlobalKey>{};
  final _expanded = <PaletteGroup>{};
  String? _selected;
  String _query = '';
  List<PaletteRow> _rows = const [];

  Timer? _announce;
  int? _counted;
  String _announced = '';

  /// The In note text slot (156l): debounced, and numbered so an answer
  /// to an older query is dropped.
  static const _textDebounce = Duration(milliseconds: 150);
  Timer? _textTimer;
  int _textSeq = 0;
  PaletteText? _text;

  @override
  void initState() {
    super.initState();
    _query = widget.initial;
    _controller.addListener(_onText);
    _askText();
  }

  @override
  void dispose() {
    _textTimer?.cancel();
    _announce?.cancel();
    _controller.dispose();
    _field.dispose();
    _scroll.dispose();
    super.dispose();
  }

  void _onText() {
    if (_controller.text == _query) return;
    setState(() {
      _query = _controller.text;
      _selected = null;
      _expanded.clear();
      _askText();
    });
  }

  void _askText() {
    final q = paletteTextQuery(_query);
    if (q != null && q == _text?.query) return;
    _textTimer?.cancel();
    final seq = ++_textSeq;
    final offline = ref.read(paletteTitlesProvider).value?.offline ?? false;
    final search = ref.read(paletteTextSearchProvider);
    if (q == null || offline || search == null) {
      _text = null;
      return;
    }
    _text = PaletteText(query: q, pending: true);
    _textTimer = Timer(_textDebounce, () async {
      PaletteText answer;
      try {
        final found = await search(q);
        answer = PaletteText(
          query: q,
          hits: found.hits,
          skipped: found.skipped,
        );
      } catch (_) {
        answer = PaletteText(query: q, failed: true);
      }
      if (!mounted || seq != _textSeq) return;
      setState(() => _text = answer);
    });
  }

  bool get _composing => _controller.value.composing.isValid;

  PaletteRow? get _current {
    if (_rows.isEmpty) return null;
    return _rows.where((r) => r.id == _selected).firstOrNull ?? _rows.first;
  }

  void _move(int by) {
    if (_rows.isEmpty) return;
    final at = _rows.indexOf(_current!);
    final next = _rows[(at + by) % _rows.length];
    setState(() => _selected = next.id);
    WidgetsBinding.instance.addPostFrameCallback((_) {
      final row = _keys[next.id]?.currentContext;
      if (row != null && row.mounted) {
        Scrollable.ensureVisible(
          row,
          alignmentPolicy: by > 0
              ? ScrollPositionAlignmentPolicy.keepVisibleAtEnd
              : ScrollPositionAlignmentPolicy.keepVisibleAtStart,
        );
      }
    });
  }

  void _setQuery(String text) {
    _controller.value = TextEditingValue(
      text: text,
      selection: TextSelection.collapsed(offset: text.length),
    );
  }

  void _run(PaletteRow row, {bool secondary = false}) {
    switch (row.target) {
      case ShowAllTarget(:final group):
        final at = _rows.indexOf(row);
        setState(() => _expanded.add(group));
        // The row that takes "Show all"'s place is the first one it showed.
        WidgetsBinding.instance.addPostFrameCallback((_) {
          if (mounted && at >= 0 && at < _rows.length) {
            setState(() => _selected = _rows[at].id);
          }
        });
        return;
      case PrefixTarget(:final prefix):
        _setQuery(prefix);
        return;
      case CommandTarget(:final command) when command.id == 'search':
        _setQuery('');
        return;
      case CommandTarget(:final command) when command.id == 'commands':
        _setQuery('>');
        return;
      default:
    }
    if (secondary && row.target is! NoteTarget) return;
    if (row.pickId case final id?) {
      unawaited(ref.read(frecencyProvider.notifier).picked(id));
    }
    Navigator.of(context).pop(PaletteChoice(row.target, secondary: secondary));
  }

  void _create() {
    final (mode, q) = parsePaletteQuery(_query);
    if (mode != PaletteMode.all || q.isEmpty) return;
    if (ref.read(activeVaultProvider).isEmpty) return;
    Navigator.of(context).pop(PaletteChoice(CreateNoteTarget(q)));
  }

  void _escape() {
    if (_controller.text.isNotEmpty) {
      _setQuery('');
    } else {
      Navigator.of(context).pop();
    }
  }

  PaletteSources _sources() {
    final active = ref.watch(activeVaultProvider);
    final vaults = ref.watch(vaultsProvider).value ?? const <VaultInfo>[];
    return PaletteSources(
      titles: ref.watch(paletteTitlesProvider).value ?? PaletteTitles.empty,
      commands: widget.commands,
      settings: kSettingsPages,
      overview: ref.watch(agentOverviewProvider).value,
      tags: ref.watch(tagsProvider).value ?? const [],
      recents: ref.watch(recentsProvider).value ?? const [],
      activeVault: active,
      activeVaultName:
          vaults.where((v) => v.id == active).firstOrNull?.name ?? '',
      frecency: ref.watch(frecencyProvider).value ?? const Frecency(),
      text: _text,
      now: DateTime.now(),
    );
  }

  void _count(int n) {
    if (n == _counted) return;
    _counted = n;
    _announce?.cancel();
    _announce = Timer(const Duration(milliseconds: 400), () {
      if (!mounted) return;
      setState(() => _announced = n == 1 ? '1 result' : '$n results');
    });
  }

  Map<ShortcutActivator, Intent> get _shortcuts => {
    const SingleActivator(LogicalKeyboardKey.arrowDown): const _Move(1),
    const SingleActivator(LogicalKeyboardKey.arrowUp): const _Move(-1),
    const SingleActivator(LogicalKeyboardKey.enter): const _Run(),
    const SingleActivator(LogicalKeyboardKey.numpadEnter): const _Run(),
    const SingleActivator(LogicalKeyboardKey.enter, shift: true):
        const _Create(),
    SingleActivator(
      LogicalKeyboardKey.enter,
      meta: stormUsesMetaModifier,
      control: !stormUsesMetaModifier,
    ): const _Secondary(),
    const SingleActivator(LogicalKeyboardKey.escape): const _Escape(),
  };

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final view = buildPaletteView(_sources(), _query, expanded: _expanded);
    _rows = view.rows;
    _count(view.count);
    final current = _current;
    // A phone has no keys; the web's browser keeps several (Q10).
    final hints = stormHasKeyboard && !kIsWeb;
    final full = !context.isExpanded;

    final field = Shortcuts(
      shortcuts: _shortcuts,
      child: Actions(
        actions: {
          _Move: _Guarded<_Move>(this, (i) => _move(i.by)),
          _Run: _Guarded<_Run>(this, (_) {
            if (_current case final row?) _run(row);
          }),
          _Create: _Guarded<_Create>(this, (_) => _create()),
          _Secondary: _Guarded<_Secondary>(this, (_) {
            if (_current case final row?) _run(row, secondary: true);
          }),
          _Escape: _Guarded<_Escape>(this, (_) => _escape()),
        },
        child: TextField(
          key: const Key('palette-field'),
          controller: _controller,
          focusNode: _field,
          autofocus: true,
          autocorrect: false,
          textInputAction: TextInputAction.go,
          style: TextStyle(fontSize: t.bodySize, color: t.text),
          decoration: InputDecoration(
            hintText: 'Search notes, sessions, commands…',
            hintStyle: TextStyle(fontSize: t.bodySize, color: t.text3),
            filled: false,
            isDense: true,
            border: InputBorder.none,
            enabledBorder: InputBorder.none,
            focusedBorder: InputBorder.none,
            contentPadding: EdgeInsets.symmetric(vertical: t.sp),
          ),
        ),
      ),
    );

    final list = <Widget>[];
    for (final section in view.sections) {
      if (section.heading.isNotEmpty) {
        list.add(PaletteHeading(section.heading));
      } else {
        list.add(SizedBox(height: t.sp * 0.5));
      }
      for (final row in section.rows) {
        list.add(
          PaletteRowTile(
            key: _keys.putIfAbsent(row.id, GlobalKey.new),
            row: row,
            selected: row.id == current?.id,
            showHint: hints,
            onTap: () => _run(row),
          ),
        );
      }
      if (section.pending) {
        list.add(const PalettePendingRow(key: Key('palette-text-pending')));
      }
      for (final line in section.lines) {
        list.add(PaletteQuietLine(line, key: ValueKey('palette-line:$line')));
      }
    }

    final body = Column(
      mainAxisSize: full ? MainAxisSize.max : MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Padding(
          padding: EdgeInsets.fromLTRB(t.sp * 2, t.sp, t.sp, t.sp),
          child: Row(
            children: [
              Icon(LucideIcons.search, size: t.uiSize, color: t.text3),
              SizedBox(width: t.sp * 1.25),
              Expanded(child: field),
              if (full)
                IconButton(
                  tooltip: 'Close',
                  onPressed: () => Navigator.of(context).pop(),
                  icon: Icon(LucideIcons.x, size: t.uiSize, color: t.text2),
                ),
            ],
          ),
        ),
        Divider(height: t.bw, thickness: t.bw, color: t.border),
        if (view.offline)
          Padding(
            key: const Key('palette-offline'),
            padding: EdgeInsets.fromLTRB(t.sp * 2, t.sp, t.sp * 2, 0),
            child: Text(
              'Offline · searching this device',
              style: TextStyle(fontSize: t.labelSize, color: t.text3),
            ),
          ),
        if (view.noMatch case final words?)
          Padding(
            padding: EdgeInsets.fromLTRB(t.sp * 2, t.sp * 1.5, t.sp * 2, 0),
            child: Text(
              words,
              style: TextStyle(fontSize: t.uiSize, color: t.text2),
            ),
          ),
        Flexible(
          fit: full ? FlexFit.tight : FlexFit.loose,
          child: ListView(
            key: const Key('palette-list'),
            controller: _scroll,
            shrinkWrap: !full,
            padding: EdgeInsets.only(bottom: t.sp),
            children: list,
          ),
        ),
        Semantics(
          liveRegion: true,
          label: _announced,
          child: const SizedBox.shrink(),
        ),
      ],
    );

    if (!full) return body;
    return Scaffold(
      key: const Key('palette-screen'),
      backgroundColor: t.surface,
      body: SafeArea(child: body),
    );
  }
}

/// A key the palette takes, except while an IME is composing: Enter then
/// commits the composition and the arrows move through its candidates.
class _Guarded<T extends Intent> extends Action<T> {
  _Guarded(this.state, this.run);

  final _StormPaletteState state;
  final void Function(T intent) run;

  @override
  bool isEnabled(T intent) => !state._composing;

  @override
  Object? invoke(T intent) {
    run(intent);
    return null;
  }
}
