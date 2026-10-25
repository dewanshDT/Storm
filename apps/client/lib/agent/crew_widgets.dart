import 'dart:math' as math;

import 'package:flutter/material.dart';
import 'package:flutter_lucide/flutter_lucide.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../ui/accents.dart';
import '../ui/settings/settings_widgets.dart' show promptForText;
import '../ui/states.dart' show describeFailure;
import '../ui/surfaces.dart';
import '../ui/tokens.dart';
import '../ui/widgets.dart' show PopoverItem, PopoverTone;
import 'agent_api.dart';
import 'agent_models.dart';
import 'agent_state.dart';
import 'agent_widgets.dart';
import 'crew_prefs.dart';

/// The pieces crews are drawn with (decision 138, plan §4.2): one role tag,
/// one crew square, chip, header and menu wherever a crew appears.

/// The palette in the crew menu's order (138a).
const kCrewPalette = [
  Accent.coral,
  Accent.peach,
  Accent.clay,
  Accent.sand,
  Accent.sage,
  Accent.mint,
  Accent.sky,
  Accent.lavender,
  Accent.blossom,
];

/// Running members before ended ones, each in crew order.
List<AgentSession> liveFirst(List<AgentSession> members) => [
  ...members.where((s) => !s.ended),
  ...members.where((s) => s.ended),
];

/// "2 running" open, "2 · 1" (running · ended) collapsed.
String crewCount(List<AgentSession> members, {required bool collapsed}) {
  final running = members.where((s) => !s.ended).length;
  return collapsed
      ? '$running · ${members.length - running}'
      : '$running running';
}

/// "3 running · 1 ended": the overview card's summary.
String crewSummary(List<AgentSession> members) {
  final running = members.where((s) => !s.ended).length;
  final ended = members.length - running;
  return [
    if (running > 0) '$running running',
    if (ended > 0) '$ended ended',
  ].join(' · ');
}

/// What a role tag says: the note's title, lower case. A deleted note keeps
/// the launch title when it is the same note.
String roleTagText(RoleRef role, {RoleRef? launchRole}) {
  final title =
      role.title ?? (role.sameNote(launchRole) ? launchRole!.title : null);
  return (title ?? 'deleted role').toLowerCase();
}

/// [text]'s width as a `Text` with [style] draws it: on top of the ambient
/// style, whose letter spacing a bare measurement would miss.
double _textWidth(BuildContext context, String text, TextStyle style) =>
    (TextPainter(
      text: TextSpan(
        text: text,
        style: DefaultTextStyle.of(context).style.merge(style),
      ),
      textDirection: Directionality.of(context),
      textScaler: MediaQuery.textScalerOf(context),
      maxLines: 1,
    )..layout()).width;

TextStyle _roleStyle(StormTokens t, {required bool deleted}) => TextStyle(
  fontFamily: StormTokens.monoFamily,
  fontSize: t.labelSize,
  height: 1.45,
  color: t.text2,
  decoration: deleted ? TextDecoration.lineThrough : null,
  decorationColor: t.text2,
);

/// The role tag's width bounds: 5ch to 15ch of its text, plus its padding
/// and outline.
({double min, double max, double chrome}) _roleBounds(BuildContext context) {
  final t = context.tokens;
  final ch = _textWidth(context, '0', _roleStyle(t, deleted: false));
  final chrome = t.sp * 0.625 * 2 + t.bw * 2;
  return (min: ch * 5 + chrome, max: ch * 15 + chrome, chrome: chrome);
}

/// A session's role (138d): neutral and solid whatever the note says, about
/// 14ch at most, struck through once its note is deleted.
class RoleTag extends StatelessWidget {
  const RoleTag({super.key, required this.role, this.launchRole});

  final RoleRef role;

  /// The session's launch role, for a deleted label's last known title.
  final RoleRef? launchRole;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final bounds = _roleBounds(context);
    return Opacity(
      opacity: role.deleted ? 0.6 : 1,
      child: Container(
        key: Key('role-tag-${role.noteId}'),
        constraints: BoxConstraints(minWidth: bounds.min, maxWidth: bounds.max),
        padding: EdgeInsets.symmetric(horizontal: t.sp * 0.625),
        decoration: BoxDecoration(
          borderRadius: BorderRadius.circular(t.rControl * 0.5),
          border: Border.all(color: t.border, width: t.bw),
        ),
        child: Text(
          roleTagText(role, launchRole: launchRole),
          maxLines: 1,
          softWrap: false,
          overflow: TextOverflow.ellipsis,
          style: _roleStyle(t, deleted: role.deleted),
        ),
      ),
    );
  }
}

/// What a [RoleTagRow] takes when nothing has to give way.
double roleTagRowWidth(
  BuildContext context, {
  required String name,
  required TextStyle style,
  RoleRef? role,
  RoleRef? launchRole,
}) {
  final nameW = _textWidth(context, name, style).ceilToDouble();
  if (role == null) return nameW;
  final t = context.tokens;
  final bounds = _roleBounds(context);
  final tag = _textWidth(
    context,
    roleTagText(role, launchRole: launchRole),
    _roleStyle(t, deleted: role.deleted),
  );
  return nameW +
      t.sp * 0.75 +
      (tag + bounds.chrome).ceilToDouble().clamp(bounds.min, bounds.max);
}

/// A name and, when there is one, its role tag. The tag gives way first,
/// down to 5ch, and only then the name (handoff §3: shrink 1000 against 1).
class RoleTagRow extends StatelessWidget {
  const RoleTagRow({
    super.key,
    required this.name,
    required this.role,
    required this.style,
    this.launchRole,
  });

  final String name;
  final RoleRef? role;
  final RoleRef? launchRole;
  final TextStyle style;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final label = Text(
      name,
      maxLines: 1,
      softWrap: false,
      overflow: TextOverflow.ellipsis,
      style: style,
    );
    final r = role;
    if (r == null) return label;
    final gap = t.sp * 0.75;
    final bounds = _roleBounds(context);
    final nameW = _textWidth(context, name, style).ceilToDouble();
    final tagText = _textWidth(
      context,
      roleTagText(r, launchRole: launchRole),
      _roleStyle(t, deleted: r.deleted),
    );
    final tagW = (tagText + bounds.chrome).ceilToDouble().clamp(
      bounds.min,
      bounds.max,
    );
    return LayoutBuilder(
      builder: (context, box) {
        var tag = tagW, nameRoom = nameW;
        final room = box.maxWidth;
        if (box.hasBoundedWidth && nameW + gap + tagW > room) {
          final floor = math.min(bounds.min, math.max(0.0, room - gap));
          tag = (room - gap - nameW).clamp(floor, math.max(floor, tagW));
          nameRoom = (room - gap - tag).clamp(0.0, nameW);
        }
        return Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            SizedBox(width: nameRoom, child: label),
            SizedBox(width: gap),
            SizedBox(
              width: tag,
              child: Align(
                alignment: AlignmentDirectional.centerStart,
                child: RoleTag(role: r, launchRole: launchRole),
              ),
            ),
          ],
        );
      },
    );
  }
}

/// A crew's colour: its palette word's tile in this theme (`Accent.tile`,
/// 138l), with the handoff's size/3 corner.
class CrewSquare extends StatelessWidget {
  const CrewSquare({super.key, required this.accent, required this.size});

  final Accent accent;
  final double size;

  @override
  Widget build(BuildContext context) => Container(
    width: size,
    height: size,
    decoration: BoxDecoration(
      color: accent.tile(context.tokens),
      borderRadius: BorderRadius.circular(size / 3),
    ),
  );
}

/// A 9px square and the crew's name, ellipsized past 220.
class CrewChip extends StatelessWidget {
  const CrewChip({super.key, required this.crew});

  final Crew crew;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        CrewSquare(accent: crew.accent, size: t.sp * 1.125),
        SizedBox(width: t.sp * 0.75),
        ConstrainedBox(
          constraints: BoxConstraints(maxWidth: t.sp * 27.5),
          child: Text(
            crew.name,
            key: Key('crew-chip-${crew.id}'),
            maxLines: 1,
            softWrap: false,
            overflow: TextOverflow.ellipsis,
            style: TextStyle(
              fontFamily: StormTokens.sansFamily,
              fontSize: t.labelSize * 1.09,
              color: t.text2,
            ),
          ),
        ),
      ],
    );
  }
}

/// A crew in the sidebar: chevron, square, name and count; a ··· on hover
/// and right-click both open [showCrewMenu] (138n, Q4).
class CrewHeader extends StatefulWidget {
  const CrewHeader({
    super.key,
    required this.crew,
    required this.members,
    required this.collapsed,
    required this.onToggle,
    required this.onMenu,
  });

  final Crew crew;
  final List<AgentSession> members;
  final bool collapsed;
  final VoidCallback onToggle;

  /// Opens the menu, anchored to the given key.
  final void Function(GlobalKey anchor) onMenu;

  @override
  State<CrewHeader> createState() => _CrewHeaderState();
}

class _CrewHeaderState extends State<CrewHeader> {
  final _anchor = GlobalKey();
  bool _hover = false;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final c = widget.crew;
    final count = crewCount(widget.members, collapsed: widget.collapsed);
    final radius = BorderRadius.circular(t.rControl);
    return MouseRegion(
      onEnter: (_) => setState(() => _hover = true),
      onExit: (_) => setState(() => _hover = false),
      child: GestureDetector(
        onSecondaryTapUp: (_) => widget.onMenu(_anchor),
        child: Semantics(
          button: true,
          expanded: !widget.collapsed,
          label: '${c.name}, crew, $count',
          excludeSemantics: true,
          onTap: widget.onToggle,
          child: Material(
            key: _anchor,
            color: _hover ? t.surface2 : Colors.transparent,
            borderRadius: radius,
            child: InkWell(
              key: Key('crew-${c.id}'),
              borderRadius: radius,
              onTap: widget.onToggle,
              child: Padding(
                padding: EdgeInsets.symmetric(
                  horizontal: t.sp,
                  vertical: t.sp * 0.875,
                ),
                child: Row(
                  children: [
                    SizedBox(
                      width: t.sp * 1.5,
                      child: Icon(
                        widget.collapsed
                            ? LucideIcons.chevron_right
                            : LucideIcons.chevron_down,
                        size: t.labelSize,
                        color: t.text3,
                      ),
                    ),
                    SizedBox(width: t.sp),
                    CrewSquare(accent: c.accent, size: t.sp * 1.25),
                    SizedBox(width: t.sp),
                    Expanded(
                      child: Text(
                        c.name,
                        key: Key('crew-name-${c.id}'),
                        maxLines: 1,
                        softWrap: false,
                        overflow: TextOverflow.ellipsis,
                        style: TextStyle(
                          fontFamily: StormTokens.sansFamily,
                          fontSize: t.codeSize,
                          fontWeight: FontWeight.w500,
                          color: t.text,
                        ),
                      ),
                    ),
                    SizedBox(width: t.sp),
                    Text(
                      count,
                      key: Key('crew-count-${c.id}'),
                      style: TextStyle(
                        fontFamily: StormTokens.monoFamily,
                        fontSize: t.labelSize,
                        color: t.text3,
                      ),
                    ),
                    if (_hover) ...[
                      SizedBox(width: t.sp * 0.5),
                      _Dots(
                        key: Key('crew-menu-${c.id}'),
                        onTap: () => widget.onMenu(_anchor),
                      ),
                    ],
                  ],
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }
}

class _Dots extends StatelessWidget {
  const _Dots({super.key, required this.onTap, this.card = false});

  final VoidCallback onTap;

  /// The overview card's: bare, in `text3`.
  final bool card;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final radius = BorderRadius.circular(t.rControl * 0.6);
    return Semantics(
      button: true,
      label: 'Crew actions',
      excludeSemantics: true,
      onTap: onTap,
      child: Material(
        color: card ? Colors.transparent : t.surface,
        borderRadius: radius,
        child: InkWell(
          borderRadius: radius,
          onTap: onTap,
          child: SizedBox(
            width: t.sp * 2.75,
            height: t.sp * 2.5,
            child: Icon(
              LucideIcons.ellipsis,
              size: t.codeSize,
              color: card ? t.text3 : t.text2,
            ),
          ),
        ),
      ),
    );
  }
}

/// A crew's members behind its solid left edge (handoff §3).
class CrewMembers extends StatelessWidget {
  const CrewMembers({super.key, required this.children});

  final List<Widget> children;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Container(
      margin: EdgeInsets.only(left: t.sp * 1.625),
      padding: EdgeInsets.only(left: t.sp * 0.75),
      decoration: BoxDecoration(
        border: Border(
          left: BorderSide(color: t.border, width: t.bw),
        ),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          for (final (i, c) in children.indexed) ...[
            if (i > 0) SizedBox(height: t.sp * 0.25),
            c,
          ],
        ],
      ),
    );
  }
}

/// What the crew menu was closed with.
enum CrewAction { openPage, rename, collapse, newSession, dismissEnded, endAll }

/// The crew menu's contents (handoff §3): 220 wide on `surface2`. An item
/// with no handler is drawn disabled; Open crew page, New session in crew and
/// End all running wait for their screens (138-4).
class CrewMenu extends StatelessWidget {
  const CrewMenu({
    super.key,
    required this.crew,
    required this.collapsed,
    required this.ended,
    required this.running,
    this.enabled = const {
      CrewAction.rename,
      CrewAction.collapse,
      CrewAction.dismissEnded,
    },
  });

  final Crew crew;
  final bool collapsed;
  final int ended;
  final int running;
  final Set<CrewAction> enabled;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    Widget item(
      CrewAction a,
      String label, {
      int? count,
      PopoverTone tone = PopoverTone.normal,
    }) {
      final on = enabled.contains(a) && (count == null || count > 0);
      return PopoverItem(
        key: Key('crew-action-${a.name}'),
        label: label,
        tone: on ? tone : PopoverTone.muted,
        trailing: count == null
            ? null
            : Text(
                '$count',
                style: TextStyle(
                  fontFamily: StormTokens.monoFamily,
                  fontSize: t.labelSize,
                  color: on && tone == PopoverTone.danger ? t.danger : t.text3,
                ),
              ),
        onTap: on ? () => Navigator.pop(context, a) : null,
      );
    }

    return StormPopover(
      key: const Key('crew-menu'),
      width: t.sp * 27.5,
      padding: t.sp * 0.75,
      children: [
        item(CrewAction.openPage, 'Open crew page'),
        item(CrewAction.rename, 'Rename'),
        Padding(
          padding: EdgeInsets.fromLTRB(
            t.sp * 0.75,
            t.sp * 0.75,
            t.sp * 0.75,
            t.sp,
          ),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text(
                'Colour',
                style: TextStyle(
                  fontFamily: StormTokens.sansFamily,
                  fontSize: t.codeSize,
                  color: t.text,
                ),
              ),
              SizedBox(height: t.sp * 0.875),
              CrewSwatches(
                selected: crew.accent,
                onSelected: (a) => Navigator.pop(context, a),
              ),
            ],
          ),
        ),
        item(CrewAction.collapse, collapsed ? 'Expand' : 'Collapse'),
        const PopoverDivider(),
        item(CrewAction.newSession, 'New session in crew'),
        item(CrewAction.dismissEnded, 'Dismiss ended', count: ended),
        item(
          CrewAction.endAll,
          'End all running…',
          count: running,
          tone: PopoverTone.danger,
        ),
      ],
    );
  }
}

/// The nine palette swatches, each its `Accent.tile` in this theme.
class CrewSwatches extends StatelessWidget {
  const CrewSwatches({
    super.key,
    required this.selected,
    required this.onSelected,
  });

  final Accent selected;
  final ValueChanged<Accent> onSelected;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final side = t.sp * 1.875;
    return Wrap(
      spacing: t.sp * 0.625,
      runSpacing: t.sp * 0.625,
      children: [
        for (final a in kCrewPalette)
          Semantics(
            button: true,
            selected: a == selected,
            label: a.label,
            excludeSemantics: true,
            onTap: () => onSelected(a),
            child: GestureDetector(
              onTap: () => onSelected(a),
              child: Container(
                key: Key('crew-swatch-${a.name}'),
                width: side,
                height: side,
                decoration: BoxDecoration(
                  shape: BoxShape.circle,
                  color: a.tile(t),
                  border: a == selected
                      ? Border.all(color: t.accent, width: t.bw * 1.5)
                      : null,
                ),
              ),
            ),
          ),
      ],
    );
  }
}

/// Opens the crew menu at [anchor] and does what was picked: rename,
/// recolour, collapse and dismiss ended are wired; the rest are drawn
/// disabled until their screens exist.
Future<void> showCrewMenu(
  BuildContext context,
  WidgetRef ref, {
  required GlobalKey anchor,
  required Crew crew,
  required AgentOverview overview,
}) async {
  final t = context.tokens;
  final members = overview.membersOf(crew);
  final collapsed =
      ref.read(crewPrefsProvider).value?.collapsed(crew.id) ?? false;
  final picked = await showStormPopover<Object>(
    context: context,
    anchorKey: anchor,
    width: t.sp * 27.5,
    builder: (_) => CrewMenu(
      crew: crew,
      collapsed: collapsed,
      ended: members.where((s) => s.ended).length,
      running: members.where((s) => !s.ended).length,
    ),
  );
  if (picked == null || !context.mounted) return;
  void toast(String text) => ScaffoldMessenger.maybeOf(
    context,
  )?.showSnackBar(SnackBar(content: Text(text)));

  Future<void> call(String what, Future<void> Function(AgentApi api) f) async {
    final api = agentApi(ref);
    if (api == null) return;
    try {
      await f(api);
    } catch (e) {
      toast("Couldn't $what: ${describeFailure(e)}");
    } finally {
      api.dispose();
      ref.invalidate(agentOverviewProvider);
    }
  }

  switch (picked) {
    case Accent a when a != crew.accent:
      await call(
        'recolour ${crew.name}',
        (api) => api.updateCrew(crew.id, color: a.name),
      );
    case CrewAction.rename:
      final name = await promptForText(
        context,
        title: 'Rename crew',
        hint: 'Crew name',
        initial: crew.name,
      );
      final trimmed = name?.trim() ?? '';
      if (trimmed.isEmpty || trimmed == crew.name || !context.mounted) return;
      await call(
        'rename ${crew.name}',
        (api) => api.updateCrew(crew.id, name: trimmed),
      );
    case CrewAction.collapse:
      ref.read(crewPrefsProvider.notifier).toggleCrew(crew.id);
    case CrewAction.dismissEnded:
      Crew? removed;
      await call('dismiss every ended session in ${crew.name}', (api) async {
        for (final s in members.where((s) => s.ended)) {
          removed = await api.dismiss(s.id) ?? removed;
        }
      });
      if (removed != null) {
        toast('${removed!.name} was removed: its last session was dismissed.');
      }
    default:
      break;
  }
}

/// A crew on the overview (handoff §3): open, its members' rows; collapsed,
/// their stacked marks and names.
class CrewCard extends ConsumerStatefulWidget {
  const CrewCard({
    super.key,
    required this.crew,
    required this.overview,
    required this.collapsed,
    required this.onOpenSession,
  });

  final Crew crew;
  final AgentOverview overview;
  final bool collapsed;
  final ValueChanged<AgentSession> onOpenSession;

  @override
  ConsumerState<CrewCard> createState() => _CrewCardState();
}

class _CrewCardState extends ConsumerState<CrewCard> {
  final anchor = GlobalKey();

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final CrewCard(:crew, :overview, :collapsed, :onOpenSession) = widget;
    final members = liveFirst(overview.membersOf(crew));
    void menu() => showCrewMenu(
      context,
      ref,
      anchor: anchor,
      crew: crew,
      overview: overview,
    );
    void toggle() => ref.read(crewPrefsProvider.notifier).toggleCrew(crew.id);
    return GestureDetector(
      onSecondaryTapUp: (_) => menu(),
      child: Container(
        key: Key('crew-card-${crew.id}'),
        padding: EdgeInsets.symmetric(
          horizontal: t.sp * 2.25,
          vertical: t.sp * 1.5,
        ),
        decoration: BoxDecoration(
          color: t.surface,
          borderRadius: BorderRadius.circular(t.rCard),
          border: Border.all(color: t.border, width: t.bw),
        ),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Semantics(
              button: true,
              expanded: !collapsed,
              label: '${crew.name}, crew, ${crewSummary(members)}',
              excludeSemantics: true,
              onTap: toggle,
              child: GestureDetector(
                key: Key('crew-card-header-${crew.id}'),
                behavior: HitTestBehavior.opaque,
                onTap: toggle,
                child: Row(
                  key: anchor,
                  children: [
                    // Its own row, so the name has all the room the summary
                    // leaves rather than sharing it with a spacer.
                    Expanded(
                      child: Row(
                        children: [
                          CrewSquare(accent: crew.accent, size: t.sp * 1.5),
                          SizedBox(width: t.sp * 1.125),
                          Flexible(
                            child: Text(
                              crew.name,
                              maxLines: 1,
                              softWrap: false,
                              overflow: TextOverflow.ellipsis,
                              style: TextStyle(
                                fontFamily: StormTokens.sansFamily,
                                fontSize: t.uiSize * 1.05,
                                fontWeight: FontWeight.w600,
                                color: t.text,
                              ),
                            ),
                          ),
                          SizedBox(width: t.sp * 1.125),
                          Text(
                            crewSummary(members),
                            style: TextStyle(
                              fontFamily: StormTokens.monoFamily,
                              fontSize: t.labelSize * 1.09,
                              color: t.text3,
                            ),
                          ),
                        ],
                      ),
                    ),
                    _Dots(
                      key: Key('crew-card-menu-${crew.id}'),
                      card: true,
                      onTap: menu,
                    ),
                  ],
                ),
              ),
            ),
            SizedBox(height: t.sp * 0.75),
            if (collapsed)
              _Stacked(members: members)
            else
              for (final s in members)
                CrewCardRow(
                  session: s,
                  host: overview.hostName(s.hostId),
                  onTap: () => onOpenSession(s),
                ),
          ],
        ),
      ),
    );
  }
}

/// One member on an open crew card: mark 28, name and role, the sub-line,
/// its context note and what it wrote.
class CrewCardRow extends StatelessWidget {
  const CrewCardRow({
    super.key,
    required this.session,
    required this.host,
    required this.onTap,
  });

  final AgentSession session;
  final String host;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final s = session;
    final radius = BorderRadius.circular(t.rControl);
    return Semantics(
      button: true,
      label: sessionSpoken(s, host),
      excludeSemantics: true,
      onTap: onTap,
      child: Material(
        color: Colors.transparent,
        borderRadius: radius,
        child: InkWell(
          key: Key('crew-row-${s.id}'),
          borderRadius: radius,
          hoverColor: t.surface2,
          onTap: onTap,
          child: Padding(
            padding: EdgeInsets.symmetric(
              horizontal: t.sp,
              vertical: t.sp * 0.875,
            ),
            child: Row(
              children: [
                AgentMark(session: s, size: t.sp * 3.5),
                SizedBox(width: t.sp * 1.25),
                Expanded(
                  child: LayoutBuilder(
                    builder: (context, box) {
                      final nameStyle = TextStyle(
                        fontFamily: StormTokens.sansFamily,
                        fontSize: t.uiSize,
                        color: s.ended ? t.text2 : t.text,
                      );
                      final subStyle = TextStyle(
                        fontFamily: StormTokens.monoFamily,
                        fontSize: t.labelSize * 1.09,
                        color: t.text3,
                      );
                      final gap = t.sp * 1.25;
                      // The name and role first; the sub-line takes the
                      // rest, and keeps at least 40% when both can't fit.
                      final natural = roleTagRowWidth(
                        context,
                        name: s.displayName,
                        style: nameStyle,
                        role: s.role,
                        launchRole: s.launchRole,
                      );
                      final sub = _textWidth(
                        context,
                        sessionSub(s, host),
                        subStyle,
                      );
                      final nameW = math.min(
                        natural,
                        math.max(box.maxWidth * 0.6, box.maxWidth - gap - sub),
                      );
                      return Row(
                        children: [
                          SizedBox(
                            width: math.min(nameW, box.maxWidth),
                            child: RoleTagRow(
                              name: s.displayName,
                              role: s.role,
                              launchRole: s.launchRole,
                              style: nameStyle,
                            ),
                          ),
                          SizedBox(width: gap),
                          Expanded(
                            child: SessionSubLine(
                              session: s,
                              host: host,
                              style: subStyle,
                            ),
                          ),
                        ],
                      );
                    },
                  ),
                ),
                if (s.context != null) ...[
                  SizedBox(width: t.sp),
                  NoteContextChip(title: s.context!.title, maxWidth: t.sp * 15),
                ],
                if (s.wroteCount > 0) ...[
                  SizedBox(width: t.sp * 1.25),
                  Text(
                    'wrote ${s.wroteCount}',
                    style: TextStyle(
                      fontFamily: StormTokens.monoFamily,
                      fontSize: t.labelSize,
                      color: t.accent,
                    ),
                  ),
                ],
              ],
            ),
          ),
        ),
      ),
    );
  }
}

/// A collapsed card's members: 24px marks overlapping by 8, then their
/// names.
class _Stacked extends StatelessWidget {
  const _Stacked({required this.members});

  final List<AgentSession> members;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final side = t.sp * 3;
    final step = side - t.sp;
    return Row(
      children: [
        SizedBox(
          width: members.isEmpty ? 0 : side + step * (members.length - 1),
          height: side,
          child: Stack(
            children: [
              for (final (i, s) in members.indexed)
                Positioned(
                  left: step * i,
                  child: AgentMark(
                    session: s,
                    size: side,
                    background: t.surface,
                  ),
                ),
            ],
          ),
        ),
        SizedBox(width: t.sp),
        Expanded(
          child: Text(
            'Collapsed · ${members.map((s) => s.displayName).join(', ')}',
            maxLines: 1,
            softWrap: false,
            overflow: TextOverflow.ellipsis,
            style: TextStyle(
              fontFamily: StormTokens.sansFamily,
              fontSize: t.codeSize,
              color: t.text3,
            ),
          ),
        ),
      ],
    );
  }
}
