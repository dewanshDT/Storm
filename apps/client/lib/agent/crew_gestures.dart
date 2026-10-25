import 'dart:math' as math;

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_riverpod/legacy.dart';

import '../ui/controls.dart';
import '../ui/states.dart' show describeFailure;
import '../ui/surfaces.dart';
import '../ui/tokens.dart';
import '../ui/widgets.dart' show PopoverItem;
import 'agent_api.dart';
import 'agent_models.dart';
import 'agent_state.dart';
import 'crew_prefs.dart';
import 'crew_widgets.dart';

/// The desktop sidebar's crew gestures (decision 138n, plan §4.4): drag and
/// drop, ⌘/Ctrl- and Shift-click selection with its foot bar, and the
/// inline rename a new crew opens with. Never on a phone: nothing here is
/// built below 900, so a phone's scrolling is untouched.

/// A session being dragged.
class SessionDrag {
  const SessionDrag(this.session);

  final AgentSession session;
}

/// A crew header being dragged.
class CrewDrag {
  const CrewDrag(this.crew);

  final Crew crew;
}

/// A drag is under way: RUNNING shows as a place to leave a crew even when
/// no loose session is running.
final crewDragActiveProvider = StateProvider<bool>((ref) => false);

/// The crew whose name is open for editing in the sidebar, by id.
final crewRenameProvider = StateProvider<String?>((ref) => null);

/// The sidebar's selection, in the order the sidebar lists it, and where a
/// Shift-click range starts.
@immutable
class SessionSelection {
  const SessionSelection({this.ids = const [], this.anchor});

  final List<String> ids;
  final String? anchor;

  bool get isEmpty => ids.isEmpty;
  bool contains(String id) => ids.contains(id);
}

final sessionSelectionProvider = StateProvider<SessionSelection>(
  (ref) => const SessionSelection(),
);

/// [sel] after a click on [id] with ⌘/Ctrl ([toggle]) or Shift ([range]).
/// [order] is every row the sidebar shows, top to bottom; [current] the
/// open session, which a first ⌘-click keeps in the selection as Finder
/// does.
SessionSelection selectWith(
  SessionSelection sel,
  String id,
  List<String> order, {
  required bool toggle,
  required bool range,
  String? current,
}) {
  int at(String s) {
    final i = order.indexOf(s);
    return i < 0 ? order.length : i;
  }

  List<String> sorted(Iterable<String> ids) =>
      ids.toSet().toList()..sort((a, b) => at(a).compareTo(at(b)));

  final seed = sel.isEmpty && current != null && current != id
      ? [current]
      : sel.ids;
  if (range) {
    final from = sel.anchor ?? current ?? id;
    final a = at(from), b = at(id);
    if (a >= order.length || b >= order.length) {
      return SessionSelection(ids: sorted([...seed, id]), anchor: from);
    }
    return SessionSelection(
      ids: order.sublist(math.min(a, b), math.max(a, b) + 1),
      anchor: from,
    );
  }
  if (!toggle) return SessionSelection(ids: [id], anchor: id);
  final ids = seed.contains(id)
      ? [
          for (final s in seed)
            if (s != id) s,
        ]
      : [...seed, id];
  return SessionSelection(ids: sorted(ids), anchor: id);
}

/// Which modifier a click on a row carries.
({bool toggle, bool range}) clickModifiers() {
  final kb = HardwareKeyboard.instance;
  return (
    toggle: kb.isMetaPressed || kb.isControlPressed,
    range: kb.isShiftPressed,
  );
}

void _toast(BuildContext context, String text) => ScaffoldMessenger.maybeOf(
  context,
)?.showSnackBar(SnackBar(content: Text(text)));

/// Runs a crew change, toasts a refusal, and asks for the list again.
Future<T?> _change<T>(
  BuildContext context,
  WidgetRef ref,
  String what,
  Future<T> Function(AgentApi api) f,
) async {
  final api = agentApi(ref);
  if (api == null) return null;
  try {
    return await f(api);
  } catch (e) {
    if (context.mounted) {
      _toast(context, "Couldn't $what: ${describeFailure(e)}");
    }
    return null;
  } finally {
    api.dispose();
    if (context.mounted) ref.invalidate(agentOverviewProvider);
  }
}

void _removed(BuildContext context, CrewChange? change) {
  if (!context.mounted) return;
  for (final c in change?.removedCrews ?? const <Crew>[]) {
    _toast(context, '${c.name} was removed: its last session left it.');
  }
}

/// Makes a crew named "New crew" of [ids], in that order, and opens its
/// name for editing with the text selected (138n, Q3).
Future<void> makeCrew(
  BuildContext context,
  WidgetRef ref,
  List<String> ids,
) async {
  final change = await _change(
    context,
    ref,
    'make a crew',
    (api) => api.createCrew('New crew', sessionIds: ids),
  );
  final made = change?.crew;
  if (made == null || !context.mounted) return;
  if (ref.read(crewPrefsProvider).value?.section ?? false) {
    ref.read(crewPrefsProvider.notifier).toggleSection();
  }
  ref.read(crewRenameProvider.notifier).state = made.id;
  if (context.mounted) _removed(context, change);
}

/// Puts [s] in [crewId] in front of [before], or last ([reorder] moves it
/// last within its own crew). Only the crew: a drop never touches the role
/// (138n).
Future<void> placeInCrew(
  BuildContext context,
  WidgetRef ref,
  AgentSession s,
  String crewId, {
  String? before,
  bool reorder = false,
}) async {
  if (before == s.id) return;
  // Joining a crew it is already in changes nothing; only a gap reorders.
  if (s.crew?.id == crewId && before == null && !reorder) return;
  final change = await _change(
    context,
    ref,
    'move ${s.displayName}',
    (api) => api.moveToCrew(s.id, crewId, before: before),
  );
  if (context.mounted) _removed(context, change);
}

/// Takes [s] out of its crew.
Future<void> leaveCrew(
  BuildContext context,
  WidgetRef ref,
  AgentSession s,
) async {
  if (s.crew == null) return;
  final change = await _change(
    context,
    ref,
    'move ${s.displayName}',
    (api) => api.moveToCrew(s.id, null),
  );
  if (context.mounted) _removed(context, change);
}

/// Moves [dragged] in front of [before] (last when null) among [crews].
Future<void> reorderCrew(
  BuildContext context,
  WidgetRef ref,
  List<Crew> crews,
  Crew dragged, {
  String? before,
}) async {
  if (before == dragged.id) return;
  final ids = [
    for (final c in crews)
      if (c.id != dragged.id) c.id,
  ];
  final at = before == null ? ids.length : ids.indexOf(before);
  ids.insert(at < 0 ? ids.length : at, dragged.id);
  if (ids.indexed.every((e) => crews[e.$1].id == e.$2)) return;
  await _change(
    context,
    ref,
    'reorder the crews',
    (api) => api.reorderCrews(ids),
  );
}

/// Where in a row a drag is: its top or bottom band is the gap beside it,
/// its middle the row itself.
enum DropZone { before, onto, after }

/// The zone at [fraction] of a row's height; [gaps] off makes the whole row
/// one target.
DropZone zoneAt(double fraction, {bool gaps = true, bool onto = true}) {
  if (!onto) return fraction < 0.5 ? DropZone.before : DropZone.after;
  if (!gaps) return DropZone.onto;
  if (fraction < 0.25) return DropZone.before;
  if (fraction > 0.75) return DropZone.after;
  return DropZone.onto;
}

/// A drop target that knows which zone of it the pointer is over, and draws
/// that: an outline onto it, a line for the gap above or below.
class ZoneDropTarget<T extends Object> extends StatefulWidget {
  const ZoneDropTarget({
    super.key,
    required this.child,
    required this.zoneOf,
    required this.onDrop,
  });

  final Widget child;

  /// The zone [data] would land in at [fraction] of the height, or null
  /// where it does nothing.
  final DropZone? Function(T data, double fraction) zoneOf;
  final void Function(T data, DropZone zone) onDrop;

  @override
  State<ZoneDropTarget<T>> createState() => _ZoneDropTargetState<T>();
}

class _ZoneDropTargetState<T extends Object> extends State<ZoneDropTarget<T>> {
  DropZone? _zone;

  // The drag's offset is the pointer: its feedback is anchored there.
  DropZone? _at(T data, Offset pointer) {
    final box = context.findRenderObject() as RenderBox?;
    if (box == null || !box.hasSize || box.size.height == 0) return null;
    final y = box.globalToLocal(pointer).dy / box.size.height;
    return widget.zoneOf(data, y.clamp(0.0, 1.0));
  }

  void _show(DropZone? z) {
    if (z != _zone && mounted) setState(() => _zone = z);
  }

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return DragTarget<T>(
      onWillAcceptWithDetails: (d) => true,
      onMove: (d) => _show(_at(d.data, d.offset)),
      onLeave: (_) => _show(null),
      onAcceptWithDetails: (d) {
        final z = _at(d.data, d.offset);
        _show(null);
        if (z != null) widget.onDrop(d.data, z);
      },
      builder: (context, candidates, _) {
        final zone = candidates.isEmpty ? null : _zone;
        final line = BorderSide(color: t.accent, width: t.bw * 2);
        return DecoratedBox(
          key: zone == null ? null : Key('drop-${zone.name}'),
          position: DecorationPosition.foreground,
          decoration: switch (zone) {
            null => const BoxDecoration(),
            DropZone.onto => BoxDecoration(
              borderRadius: BorderRadius.circular(t.rControl),
              border: Border.all(color: t.accent, width: t.bw * 1.5),
            ),
            DropZone.before => BoxDecoration(border: Border(top: line)),
            DropZone.after => BoxDecoration(border: Border(bottom: line)),
          },
          child: widget.child,
        );
      },
    );
  }
}

/// A row or header that can be picked up: its ghost is the row on
/// `surface2`, turned -2°, with the shadow, held where it was grabbed; the
/// row left behind dims to .3.
class CrewDraggable<T extends Object> extends ConsumerStatefulWidget {
  const CrewDraggable({super.key, required this.data, required this.child});

  final T data;
  final Widget child;

  @override
  ConsumerState<CrewDraggable<T>> createState() => _CrewDraggableState<T>();
}

class _CrewDraggableState<T extends Object>
    extends ConsumerState<CrewDraggable<T>> {
  Offset _grab = Offset.zero;

  void _active(bool on) {
    if (mounted) ref.read(crewDragActiveProvider.notifier).state = on;
  }

  @override
  Widget build(BuildContext context) {
    return LayoutBuilder(
      builder: (context, box) => Listener(
        onPointerDown: (e) => _grab = e.localPosition,
        child: Draggable<T>(
          data: widget.data,
          // The targets read the pointer from the drag's offset; the ghost
          // is moved back by the grab so it still sits under the hand.
          dragAnchorStrategy: pointerDragAnchorStrategy,
          onDragStarted: () => _active(true),
          onDragEnd: (_) => _active(false),
          feedback: _Ghost(
            width: box.maxWidth,
            grab: () => _grab,
            child: widget.child,
          ),
          childWhenDragging: Opacity(opacity: 0.3, child: widget.child),
          child: widget.child,
        ),
      ),
    );
  }
}

class _Ghost extends StatelessWidget {
  const _Ghost({required this.width, required this.grab, required this.child});

  final double width;

  /// Read when the drag starts, after the pointer went down.
  final Offset Function() grab;
  final Widget child;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Transform.translate(
      offset: -grab(),
      child: Transform.rotate(
        angle: -2 * math.pi / 180,
        child: Container(
          key: const Key('drag-ghost'),
          width: width,
          decoration: BoxDecoration(
            color: t.surface2,
            borderRadius: BorderRadius.circular(t.rControl),
            boxShadow: t.shadow,
          ),
          child: Material(type: MaterialType.transparency, child: child),
        ),
      ),
    );
  }
}

/// The foot bar while rows are selected: Make crew, Move to… and Esc, which
/// the Escape key also does from anywhere.
class SelectionBar extends ConsumerStatefulWidget {
  const SelectionBar({super.key, required this.overview});

  final AgentOverview overview;

  @override
  ConsumerState<SelectionBar> createState() => _SelectionBarState();
}

class _SelectionBarState extends ConsumerState<SelectionBar> {
  final _moveAnchor = GlobalKey();

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

  bool _key(KeyEvent e) {
    if (e is! KeyDownEvent || e.logicalKey != LogicalKeyboardKey.escape) {
      return false;
    }
    _clear();
    return true;
  }

  void _clear() {
    if (mounted) {
      ref.read(sessionSelectionProvider.notifier).state =
          const SessionSelection();
    }
  }

  List<String> get _ids => ref.read(sessionSelectionProvider).ids;

  Future<void> _make() async {
    final ids = _ids;
    _clear();
    await makeCrew(context, ref, ids);
  }

  Future<void> _move() async {
    final t = context.tokens;
    final crews = widget.overview.crews ?? const <Crew>[];
    final picked = await showStormPopover<(String?,)>(
      context: context,
      anchorKey: _moveAnchor,
      width: t.sp * 30,
      builder: (popover) => StormPopover(
        key: const Key('selection-move-menu'),
        padding: t.sp * 0.75,
        children: [
          PopoverItem(
            key: const Key('move-to-none'),
            label: 'Not in a crew',
            onTap: () => Navigator.of(popover).pop((null,)),
          ),
          for (final c in crews)
            PopoverItem(
              key: Key('move-to-${c.id}'),
              label: c.name,
              leading: CrewSquare(accent: c.accent, size: t.sp * 1.25),
              onTap: () => Navigator.of(popover).pop((c.id,)),
            ),
        ],
      ),
    );
    if (picked == null || !mounted) return;
    final to = picked.$1;
    final sessions = [
      for (final id in _ids) ?widget.overview.byId(id),
    ].where((s) => s.crew?.id != to).toList();
    _clear();
    final change = await _change(context, ref, 'move the sessions', (
      api,
    ) async {
      final removed = <Crew>[];
      for (final s in sessions) {
        removed.addAll((await api.moveToCrew(s.id, to)).removedCrews);
      }
      return CrewChange(removedCrews: removed);
    });
    if (mounted) _removed(context, change);
  }

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final n = ref.watch(sessionSelectionProvider).ids.length;
    return Padding(
      key: const Key('selection-bar'),
      padding: EdgeInsets.symmetric(horizontal: t.sp * 1.5, vertical: t.sp),
      child: Row(
        children: [
          Expanded(
            child: Text(
              '$n selected',
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: TextStyle(
                fontFamily: StormTokens.monoFamily,
                fontSize: t.labelSize,
                color: t.text3,
              ),
            ),
          ),
          StormButton.soft(
            key: const Key('selection-make-crew'),
            label: 'Make crew',
            onPressed: _make,
          ),
          SizedBox(width: t.sp * 0.5),
          KeyedSubtree(
            key: _moveAnchor,
            child: StormButton.text(
              key: const Key('selection-move'),
              label: 'Move to…',
              onPressed: _move,
            ),
          ),
          StormButton.text(
            key: const Key('selection-clear'),
            label: 'Esc',
            onPressed: _clear,
          ),
        ],
      ),
    );
  }
}

/// A crew's name being edited in place: "New crew" selected on open, kept
/// on Enter or when focus leaves, left as it was on Escape.
class CrewNameField extends ConsumerStatefulWidget {
  const CrewNameField({super.key, required this.crew, required this.style});

  final Crew crew;
  final TextStyle style;

  @override
  ConsumerState<CrewNameField> createState() => _CrewNameFieldState();
}

class _CrewNameFieldState extends ConsumerState<CrewNameField> {
  late final _text = TextEditingController(text: widget.crew.name)
    ..selection = TextSelection(
      baseOffset: 0,
      extentOffset: widget.crew.name.length,
    );
  late final _focus = FocusNode(onKeyEvent: _onKey);
  bool _done = false;

  @override
  void initState() {
    super.initState();
    _focus.addListener(() {
      if (!_focus.hasFocus) _commit();
    });
    // The drop or click that made the crew still holds focus elsewhere.
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) _focus.requestFocus();
    });
  }

  @override
  void dispose() {
    _text.dispose();
    _focus.dispose();
    super.dispose();
  }

  KeyEventResult _onKey(FocusNode _, KeyEvent e) {
    if (e is KeyDownEvent && e.logicalKey == LogicalKeyboardKey.escape) {
      _finish();
      return KeyEventResult.handled;
    }
    return KeyEventResult.ignored;
  }

  void _finish() {
    _done = true;
    if (mounted && ref.read(crewRenameProvider) == widget.crew.id) {
      ref.read(crewRenameProvider.notifier).state = null;
    }
  }

  Future<void> _commit() async {
    if (_done) return;
    final name = _text.text.trim();
    final c = widget.crew;
    _finish();
    if (name.isEmpty || name == c.name) return;
    await _change(
      context,
      ref,
      'rename ${c.name}',
      (api) => api.updateCrew(c.id, name: name),
    );
  }

  @override
  Widget build(BuildContext context) => TextField(
    key: Key('crew-rename-${widget.crew.id}'),
    controller: _text,
    focusNode: _focus,
    autofocus: true,
    maxLines: 1,
    style: widget.style,
    decoration: const InputDecoration.collapsed(hintText: 'Crew name'),
    onSubmitted: (_) => _commit(),
  );
}
