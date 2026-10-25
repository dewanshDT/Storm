import 'dart:math' as math;

import 'package:flutter/material.dart';
import 'package:flutter/semantics.dart' show CustomSemanticsAction;
import 'package:flutter_lucide/flutter_lucide.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_riverpod/legacy.dart';
import 'package:go_router/go_router.dart';

import '../router.dart';
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
import 'agents_screen.dart' show launchAgentSession;
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
  const CrewChip({super.key, required this.crew, this.maxWidth});

  final Crew crew;

  /// All the room it has, square included, where that is under 220.
  final double? maxWidth;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final room = maxWidth == null
        ? t.sp * 27.5
        : math.min(t.sp * 27.5, math.max(0.0, maxWidth! - t.sp * 1.875));
    return Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        CrewSquare(accent: crew.accent, size: t.sp * 1.125),
        SizedBox(width: t.sp * 0.75),
        ConstrainedBox(
          constraints: BoxConstraints(maxWidth: room),
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
/// and right-click both open [showCrewMenu] (138n, Q4). The header opens the
/// crew page and the chevron folds it.
class CrewHeader extends StatefulWidget {
  const CrewHeader({
    super.key,
    required this.crew,
    required this.members,
    required this.collapsed,
    required this.onToggle,
    required this.onOpen,
    required this.onMenu,
    this.selected = false,
    this.nameField,
  });

  final Crew crew;
  final List<AgentSession> members;

  /// Drawn in place of the name while it is being edited.
  final Widget? nameField;
  final bool collapsed;
  final VoidCallback onToggle;
  final VoidCallback onOpen;

  /// Its crew page is showing.
  final bool selected;

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
          selected: widget.selected,
          expanded: !widget.collapsed,
          label: '${c.name}, crew, $count',
          excludeSemantics: true,
          onTap: widget.onOpen,
          customSemanticsActions: {
            CustomSemanticsAction(
              label: widget.collapsed ? 'Expand' : 'Collapse',
            ): widget.onToggle,
          },
          child: Material(
            key: _anchor,
            color: widget.selected || _hover ? t.surface2 : Colors.transparent,
            borderRadius: radius,
            child: InkWell(
              key: Key('crew-${c.id}'),
              borderRadius: radius,
              onTap: widget.onOpen,
              child: Padding(
                padding: EdgeInsets.symmetric(
                  horizontal: t.sp,
                  vertical: t.sp * 0.875,
                ),
                // As tall as the hover ··· so it showing never moves a row.
                child: ConstrainedBox(
                  constraints: BoxConstraints(minHeight: t.sp * 2.5),
                  child: Row(
                    children: [
                      GestureDetector(
                        key: Key('crew-chevron-${c.id}'),
                        behavior: HitTestBehavior.opaque,
                        onTap: widget.onToggle,
                        child: SizedBox(
                          width: t.sp * 1.5,
                          child: Icon(
                            widget.collapsed
                                ? LucideIcons.chevron_right
                                : LucideIcons.chevron_down,
                            size: t.labelSize,
                            color: t.text3,
                          ),
                        ),
                      ),
                      SizedBox(width: t.sp),
                      CrewSquare(accent: c.accent, size: t.sp * 1.25),
                      SizedBox(width: t.sp),
                      Expanded(
                        child:
                            widget.nameField ??
                            Text(
                              c.name,
                              key: Key('crew-name-${c.id}'),
                              maxLines: 1,
                              softWrap: false,
                              overflow: TextOverflow.ellipsis,
                              style: crewHeaderNameStyle(t),
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
                        CrewDots(
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
      ),
    );
  }
}

TextStyle crewHeaderNameStyle(StormTokens t) => TextStyle(
  fontFamily: StormTokens.sansFamily,
  fontSize: t.codeSize,
  fontWeight: FontWeight.w500,
  color: t.text,
);

/// A crew in the phone list (4j): square, name, count and a chevron that
/// folds it. A tap opens the crew page; a long-press, its actions sheet.
class PhoneCrewHeader extends StatelessWidget {
  const PhoneCrewHeader({
    super.key,
    required this.crew,
    required this.members,
    required this.collapsed,
    required this.onToggle,
    required this.onOpen,
    required this.onActions,
  });

  final Crew crew;
  final List<AgentSession> members;
  final bool collapsed;
  final VoidCallback onToggle;
  final VoidCallback onOpen;
  final VoidCallback onActions;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final c = crew;
    final count = crewCount(members, collapsed: collapsed);
    return Semantics(
      button: true,
      expanded: !collapsed,
      label: '${c.name}, crew, $count',
      excludeSemantics: true,
      onTap: onOpen,
      onLongPress: onActions,
      customSemanticsActions: {
        CustomSemanticsAction(label: collapsed ? 'Expand' : 'Collapse'):
            onToggle,
      },
      child: InkWell(
        key: Key('crew-${c.id}'),
        onTap: onOpen,
        onLongPress: onActions,
        child: Container(
          padding: EdgeInsets.symmetric(vertical: t.sp * 1.5),
          decoration: BoxDecoration(
            border: Border(
              bottom: BorderSide(color: t.border, width: t.bw),
            ),
          ),
          child: Row(
            children: [
              CrewSquare(accent: c.accent, size: t.sp * 1.5),
              SizedBox(width: t.sp * 1.25),
              Expanded(
                child: Text(
                  c.name,
                  key: Key('crew-name-${c.id}'),
                  maxLines: 1,
                  softWrap: false,
                  overflow: TextOverflow.ellipsis,
                  style: TextStyle(
                    fontFamily: StormTokens.sansFamily,
                    fontSize: t.uiSize * 1.05,
                    fontWeight: FontWeight.w500,
                    color: t.text,
                  ),
                ),
              ),
              SizedBox(width: t.sp * 1.25),
              Text(
                count,
                key: Key('crew-count-${c.id}'),
                style: TextStyle(
                  fontFamily: StormTokens.monoFamily,
                  fontSize: t.labelSize,
                  color: t.text3,
                ),
              ),
              GestureDetector(
                key: Key('crew-chevron-${c.id}'),
                behavior: HitTestBehavior.opaque,
                onTap: onToggle,
                child: SizedBox(
                  width: t.sp * 4,
                  height: t.sp * 3,
                  child: Align(
                    alignment: Alignment.centerRight,
                    child: Icon(
                      collapsed
                          ? LucideIcons.chevron_right
                          : LucideIcons.chevron_down,
                      size: t.labelSize,
                      color: t.text3,
                    ),
                  ),
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

/// The crew actions button: 22×20 on `surface` in the sidebar, bare on a
/// card, outlined on the crew page.
class CrewDots extends StatelessWidget {
  const CrewDots({
    super.key,
    required this.onTap,
    this.card = false,
    this.page = false,
  });

  final VoidCallback onTap;

  /// The overview card's: bare, in `text3`.
  final bool card;

  /// The crew page's: a 36 square with a hairline.
  final bool page;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final radius = BorderRadius.circular(t.rControl * (page ? 1 : 0.6));
    return Semantics(
      button: true,
      label: 'Crew actions',
      excludeSemantics: true,
      onTap: onTap,
      child: Material(
        color: card || page ? Colors.transparent : t.surface,
        shape: RoundedRectangleBorder(
          borderRadius: radius,
          side: page
              ? BorderSide(color: t.border, width: t.bw)
              : BorderSide.none,
        ),
        child: InkWell(
          borderRadius: radius,
          onTap: onTap,
          child: SizedBox(
            width: t.sp * (page ? 5.25 : 2.75),
            height: t.sp * (page ? 4.5 : 2.5),
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
  const CrewMembers({super.key, required this.children, this.phone = false});

  final List<Widget> children;

  /// The phone list's: 5 in, 12 of padding, rows with their own hairlines.
  final bool phone;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Container(
      margin: EdgeInsets.only(left: t.sp * (phone ? 0.625 : 1.625)),
      padding: EdgeInsets.only(left: t.sp * (phone ? 1.5 : 0.75)),
      decoration: BoxDecoration(
        border: Border(
          left: BorderSide(color: t.border, width: t.bw),
        ),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          for (final (i, c) in children.indexed) ...[
            if (i > 0 && !phone) SizedBox(height: t.sp * 0.25),
            c,
          ],
        ],
      ),
    );
  }
}

/// What the crew menu was closed with.
enum CrewAction { openPage, rename, collapse, newSession, dismissEnded, endAll }

/// The crew menu's contents (handoff §3): 220 wide on `surface2`. A counted
/// item with nothing to count is drawn disabled.
class CrewMenu extends StatelessWidget {
  const CrewMenu({
    super.key,
    required this.crew,
    required this.collapsed,
    required this.ended,
    required this.running,
    this.sheet = false,
    this.onPage = false,
  });

  final Crew crew;
  final bool collapsed;
  final int ended;
  final int running;

  /// The phone's: the same items in a sheet, without the popover's card.
  final bool sheet;

  /// Opened on the crew's own page, which needs no Open crew page.
  final bool onPage;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    Widget item(
      CrewAction a,
      String label, {
      int? count,
      PopoverTone tone = PopoverTone.normal,
    }) {
      final on = count == null || count > 0;
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

    final children = [
      if (!onPage) item(CrewAction.openPage, 'Open crew page'),
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
    ];
    if (sheet) {
      return Column(
        key: const Key('crew-menu'),
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: children,
      );
    }
    return StormPopover(
      key: const Key('crew-menu'),
      width: t.sp * 27.5,
      padding: t.sp * 0.75,
      children: children,
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

/// The crew whose End all running banner is open (138k), by id.
final crewEndAllProvider = StateProvider<String?>((ref) => null);

/// Opens the crew menu at [anchor] and does what was picked.
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
  await runCrewAction(context, ref, picked, crew: crew, overview: overview);
}

/// The crew actions as a sheet, on a phone (handoff §3 Phone: ··· opens
/// the actions as a sheet).
Future<void> showCrewSheet(
  BuildContext context,
  WidgetRef ref, {
  required Crew crew,
  required AgentOverview overview,
  bool onPage = false,
}) async {
  final members = overview.membersOf(crew);
  final collapsed =
      ref.read(crewPrefsProvider).value?.collapsed(crew.id) ?? false;
  final picked = await showStormSheet<Object>(
    context: context,
    title: crew.name,
    heightFactor: 0.86,
    builder: (_) => CrewMenu(
      crew: crew,
      collapsed: collapsed,
      ended: members.where((s) => s.ended).length,
      running: members.where((s) => !s.ended).length,
      sheet: true,
      onPage: onPage,
    ),
  );
  if (picked == null || !context.mounted) return;
  await runCrewAction(context, ref, picked, crew: crew, overview: overview);
}

/// Does what the crew menu or sheet was closed with.
Future<void> runCrewAction(
  BuildContext context,
  WidgetRef ref,
  Object picked, {
  required Crew crew,
  required AgentOverview overview,
}) async {
  final members = overview.membersOf(crew);
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
    case CrewAction.openPage:
      context.go(Routes.agentCrew(crew.id));
    case CrewAction.newSession:
      await launchAgentSession(context, ref, crew: crew);
    case CrewAction.endAll:
      ref.read(crewEndAllProvider.notifier).state = crew.id;
      context.go(Routes.agentCrew(crew.id));
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
    void open() => context.go(Routes.agentCrew(crew.id));
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
              onTap: open,
              child: GestureDetector(
                key: Key('crew-card-header-${crew.id}'),
                behavior: HitTestBehavior.opaque,
                onTap: open,
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
                    CrewDots(
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
    this.showContext = true,
  });

  final AgentSession session;
  final String host;
  final VoidCallback onTap;

  /// Its context note's chip; the crew page lists those under Started from.
  final bool showContext;

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
                if (showContext && s.context != null) ...[
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

/// "PROJECTS / STORM / AGENTS" for `projects/storm/agents`.
String roleFolderLabel(String folder) =>
    folder.split('/').where((p) => p.isNotEmpty).join(' / ');

/// Kit's roles by folder, in the server's order (`agents` first).
Map<String, List<RoleNote>> roleGroups(List<RoleNote> roles) {
  final groups = <String, List<RoleNote>>{};
  for (final r in roles) {
    (groups[r.folder] ??= []).add(r);
  }
  return groups;
}

/// The role picker (138e, handoff §3): None, then each role note's title with
/// its one-line summary under it, grouped by folder. [onPick] gets null for
/// None.
class RoleMenu extends StatelessWidget {
  const RoleMenu({
    super.key,
    required this.roles,
    required this.selected,
    required this.onPick,
    this.noneLabel = 'None',
    this.noneDetail = 'A plain agent',
    this.room,
  });

  /// The height the window leaves it at its anchor (see [popoverRoom]).
  final double? room;

  final List<RoleNote> roles;

  /// The note shown as chosen; null is None.
  final ({String vaultId, String noteId})? selected;
  final ValueChanged<RoleNote?> onPick;
  final String noneLabel;
  final String noneDetail;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final picked = selected;
    return StormPopover(
      key: const Key('role-menu'),
      padding: t.sp * 0.75,
      children: [
        ConstrainedBox(
          constraints: BoxConstraints(
            maxHeight: math.min(
              t.sp * 52.5,
              (room ?? double.infinity) - t.sp * 3,
            ),
          ),
          child: SingleChildScrollView(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              mainAxisSize: MainAxisSize.min,
              children: [
                PopoverItem(
                  key: const Key('role-none'),
                  label: noneLabel,
                  subtitle: noneDetail,
                  selected: picked == null,
                  onTap: () => onPick(null),
                ),
                for (final MapEntry(key: folder, value: notes) in roleGroups(
                  roles,
                ).entries) ...[
                  Padding(
                    padding: EdgeInsets.fromLTRB(
                      t.sp * 0.75,
                      t.sp * 1.25,
                      t.sp * 0.75,
                      t.sp * 0.5,
                    ),
                    child: AgentsLabel(roleFolderLabel(folder)),
                  ),
                  for (final r in notes)
                    PopoverItem(
                      key: Key('role-${r.noteId}'),
                      label: r.title,
                      subtitle: r.summary,
                      selected:
                          picked != null &&
                          picked.vaultId == r.vaultId &&
                          picked.noteId == r.noteId,
                      onTap: () => onPick(r),
                    ),
                ],
              ],
            ),
          ),
        ),
      ],
    );
  }
}

/// How tall a popover at [anchor] can be before it leaves the window: the
/// room below it, or above it where [placePopover] flips it up.
double popoverRoom(BuildContext context, GlobalKey anchor) {
  final height = MediaQuery.sizeOf(context).height;
  final box = anchor.currentContext?.findRenderObject() as RenderBox?;
  if (box == null) return height / 2;
  final top = box.localToGlobal(Offset.zero).dy;
  final below = height - top - box.size.height;
  return below < height * 0.3 ? top : below;
}

/// The crew's hosts, as its page header names them (138o), in member order.
List<String> crewHosts(AgentOverview o, List<AgentSession> members) => [
  ...{for (final s in members) o.hostName(s.hostId)},
];

/// CREWS, with its chevron; folded, "CREWS · n" (138m).
class CrewsSectionLabel extends StatelessWidget {
  const CrewsSectionLabel({
    super.key,
    required this.count,
    required this.collapsed,
    required this.onTap,
    this.phone = false,
  });

  final int count;
  final bool collapsed;
  final VoidCallback onTap;

  /// The phone list's: flush with the rows.
  final bool phone;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Semantics(
      button: true,
      expanded: !collapsed,
      label: 'Crews, $count',
      excludeSemantics: true,
      onTap: onTap,
      child: GestureDetector(
        key: const Key('crews-section'),
        behavior: HitTestBehavior.opaque,
        onTap: onTap,
        child: Padding(
          padding: phone
              ? EdgeInsets.only(top: t.sp * 1.75, bottom: t.sp * 0.5)
              : EdgeInsets.fromLTRB(t.sp, t.sp * 1.5, t.sp, t.sp * 0.75),
          child: Row(
            children: [
              Expanded(
                child: AgentsLabel(collapsed ? 'Crews · $count' : 'Crews'),
              ),
              Icon(
                collapsed
                    ? LucideIcons.chevron_right
                    : LucideIcons.chevron_down,
                size: t.labelSize,
                color: t.text3,
              ),
            ],
          ),
        ),
      ),
    );
  }
}
