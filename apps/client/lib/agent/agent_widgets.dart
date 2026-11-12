import 'package:flutter/material.dart';
import 'package:flutter/semantics.dart' show CustomSemanticsAction;
import 'package:flutter_lucide/flutter_lucide.dart';
import 'package:flutter_svg/flutter_svg.dart';

import '../ui/session_status.dart';
import '../ui/shell/nav_bubble.dart' show StormPill;
import '../ui/shell/sidebar_rows.dart' show UnseenDot;
import '../ui/tokens.dart';
import 'agent_models.dart';
import 'agent_state.dart' show AgentOverview;
import 'crew_widgets.dart' show CrewDots, RoleTagRow;
import 'sidebar_rules.dart';
import 'spawn_widgets.dart' show childWorkspaceLabel;

/// The pieces every agent surface draws a session with: the sidebar, the
/// overview, the phone list and the session itself say the same words.

/// Agents cannot run without the server, and saying so is a state, not an
/// error (the offline ground rule). One wording wherever it appears.
const kAgentsOfflineTitle = 'Agents need the server';
const kAgentsOfflineDetail = "Nothing to show until it's back.";

/// "personal / projects / storm": a note's vault and folders.
String noteCrumb(String vaultName, String? path) {
  final cut = path?.lastIndexOf('/') ?? -1;
  final folder = cut < 0 ? '' : path!.substring(0, cut);
  return [
    vaultName,
    ...folder.split('/').where((s) => s.isNotEmpty),
  ].join(' / ');
}

/// A note's title, or its file name without `.md` when it has none.
String noteTitleOf(String? title, String? path) {
  if (title != null && title.isNotEmpty) return title;
  final name = path?.split('/').last ?? '';
  return name.endsWith('.md') ? name.substring(0, name.length - 3) : name;
}

/// "now", "40m", "3h", "2d": how long ago, as the lists say it.
String shortAge(String? iso, {DateTime? now}) {
  final then = iso == null ? null : DateTime.tryParse(iso);
  if (then == null) return '';
  final d = (now ?? DateTime.now()).difference(then);
  if (d.inMinutes < 1) return 'now';
  if (d.inHours < 1) return '${d.inMinutes}m';
  if (d.inDays < 1) return '${d.inHours}h';
  return '${d.inDays}d';
}

/// "38 min", "2 h 5 min": how long a session ran.
String _ran(Duration d) {
  if (d.inMinutes < 1) return '${d.inSeconds} s';
  if (d.inHours < 1) return '${d.inMinutes} min';
  final m = d.inMinutes % 60;
  return m == 0 ? '${d.inHours} h' : '${d.inHours} h $m min';
}

/// "14:02": a moment as the agent surfaces say it, in local time.
String clockAt(DateTime t) {
  final l = t.toLocal();
  String two(int n) => n.toString().padLeft(2, '0');
  return '${two(l.hour)}:${two(l.minute)}';
}

/// The agent surfaces' text styles: a mono sub-line, sans body text, a
/// spaced mono key, and code or exact text as an agent sent it.
TextStyle agentMono(StormTokens t, {Color? color, double? size}) => TextStyle(
  fontFamily: StormTokens.monoFamily,
  fontSize: size ?? t.labelSize,
  color: color ?? t.text3,
);

TextStyle agentSans(StormTokens t, {Color? color, FontWeight? weight}) =>
    TextStyle(
      fontFamily: StormTokens.sansFamily,
      fontSize: t.codeSize,
      height: 1.4,
      fontWeight: weight,
      color: color ?? t.text,
    );

TextStyle agentKey(StormTokens t, {Color? color}) => TextStyle(
  fontFamily: StormTokens.monoFamily,
  fontSize: t.labelSize,
  letterSpacing: t.labelSize * 0.08,
  color: color ?? t.text3,
);

TextStyle agentCode(StormTokens t, {Color? color}) => TextStyle(
  fontFamily: StormTokens.monoFamily,
  fontSize: t.labelSize * 1.09,
  height: 1.45,
  color: color ?? t.text2,
);

/// A live session's age, or an ended one's status in lower case.
String sessionWhen(AgentSession s) =>
    s.ended ? s.statusLabel.toLowerCase() : shortAge(s.createdAt);

/// "build-vm · storm · haiku · 40m": the rows' sub-line; the mark says the
/// agent, and the model shows when it was chosen (145b).
String sessionSub(AgentSession s, String host) => [
  host,
  s.workspace,
  ?s.model,
  ?permissionModeWord(s.permissionMode),
  sessionWhen(s),
].join(' · ');

/// "Fix the login, Claude Code on build-vm, working": a row read aloud.
String sessionSpoken(AgentSession s, String host) => [
  s.displayName,
  '${providerLabel(s.provider)} on $host',
  s.status == 'running' && s.working ? 'working' : s.statusLabel.toLowerCase(),
].join(', ');

/// "Completed 14:02 · ran 38 min", "Failed · host restarted".
String endedLine(AgentSession s) {
  final ended = s.endedAt == null ? null : DateTime.tryParse(s.endedAt!);
  final started = DateTime.tryParse(s.startedAt ?? s.createdAt);
  return [
    ended == null || s.status == 'failed'
        ? s.statusLabel
        : '${s.statusLabel} ${clockAt(ended)}',
    ?s.endDetail,
    if (ended != null && started != null && s.status != 'failed')
      'ran ${_ran(ended.difference(started))}',
  ].join(' · ');
}

/// "Claude Code · storm on build-vm · started 40m ago", or the ended line.
String sessionMeta(AgentSession s, String hostName) {
  final age = shortAge(s.createdAt);
  final when = s.ended
      ? endedLine(s)
      : age == 'now'
      ? 'started now'
      : 'started $age ago';
  final agent = [
    providerLabel(s.provider),
    ?s.model,
    ?permissionModeWord(s.permissionMode),
  ].join(' · ');
  return '$agent · ${s.workspace} on $hostName · $when';
}

/// One session in a list: its agent's mark, its name, and a mono line under
/// it (handoff §2.6, §2.8; D15 AM44). One signal per channel (153e): the
/// outline is for what needs you, the mark's badge for what the agent is
/// doing, the sub-line for why, in words. The sidebar's rows are tighter
/// and fill when selected; the phone's are separated by hairlines.
class SessionRow extends StatefulWidget {
  const SessionRow({
    super.key,
    required this.session,
    required this.host,
    required this.onTap,
    this.selected = false,
    this.phone = false,
    this.overview,
    this.tier,
    this.now,
    this.fromParent,
    this.showHost = false,
    this.unread = false,
    this.tree,
    this.onMenu,
    this.onLongPress,
  });

  final AgentSession session;
  final String host;
  final VoidCallback onTap;
  final bool selected;
  final bool phone;

  /// Names whoever it is blocked on.
  final AgentOverview? overview;

  /// Its tier as the list sorted it; its own otherwise.
  final SidebarTier? tier;
  final DateTime? now;

  /// The parent it is listed apart from (another crew or section): "from
  /// {parent}" leads the sub-line, with no indent.
  final AgentSession? fromParent;

  /// Name the host: sessions run on more than one.
  final bool showHost;

  /// Output past what this device last showed.
  final bool unread;

  /// It started agents listed under it: their chevron and, folded, their
  /// roll-up.
  final TreeFold? tree;

  /// The ··· on hover, and a right-click, open its menu at the anchor.
  final void Function(GlobalKey anchor)? onMenu;

  /// The phone's crew and role sheet (decision 138n).
  final VoidCallback? onLongPress;

  @override
  State<SessionRow> createState() => _SessionRowState();
}

/// A tree parent's fold: how many are under it and, folded, their most
/// urgent state.
class TreeFold {
  const TreeFold({
    required this.size,
    required this.collapsed,
    required this.onToggle,
    this.rollUp,
  });

  final int size;
  final bool collapsed;
  final VoidCallback onToggle;
  final RollUp? rollUp;
}

class _SessionRowState extends State<SessionRow> {
  final _anchor = GlobalKey();
  bool _hover = false;

  Color _tone(StormTokens t, SubTone tone) => switch (tone) {
    SubTone.plain => t.text3,
    SubTone.loud => t.text,
    SubTone.quiet => t.text2,
    SubTone.danger => t.danger,
    SubTone.accent => t.accent,
  };

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final s = widget.session;
    final phone = widget.phone;
    final now = widget.now ?? DateTime.now();
    final tier = widget.tier ?? tierOf(s, now);
    final tree = widget.tree;
    final folded = tree != null && tree.collapsed && tree.size > 0;
    final roll = folded ? tree.rollUp : null;
    final rolled = roll != null && roll.tier.index < tier.index;
    final outlined = tier == SidebarTier.needsYou || (rolled && roll.outlined);
    final reason = reasonLine(s, now: now, tier: tier, o: widget.overview);
    final quietName =
        s.ended ||
        tier == SidebarTier.parked ||
        tier == SidebarTier.outOfUsage ||
        tier == SidebarTier.unknown;

    final subBase = agentMono(t);
    Widget subLine() {
      final lead = folded
          ? '${tree.size} ${tree.size == 1 ? 'agent' : 'agents'}'
          : null;
      if (rolled) {
        return _Line(
          '$lead · ${roll.words}',
          agentMono(t, color: roll.loud ? t.danger : t.text2),
        );
      }
      if (reason != null) {
        return _Line(
          [?lead, reason.text].join(' · '),
          agentMono(t, color: _tone(t, reason.tone)),
        );
      }
      return SessionSubLine(
        session: s,
        host: widget.showHost ? widget.host : null,
        lead: [
          ?lead,
          if (widget.fromParent case final p?) 'from ${p.displayName}',
        ],
        details: false,
        style: subBase,
      );
    }

    final radius = BorderRadius.circular(phone && !outlined ? 0 : t.rControl);
    final base = widget.selected ? t.surface2 : Colors.transparent;
    final fill = outlined
        ? Color.alphaBlend(
            t.danger.withValues(alpha: 0.09),
            widget.selected ? t.surface2 : (phone ? t.bg : t.surface),
          )
        : base;
    final content = Row(
      children: [
        AgentMark(
          session: s,
          size: t.sp * (phone ? 4.5 : 4),
          background: widget.selected ? t.surface2 : (phone ? t.bg : t.surface),
        ),
        SizedBox(width: t.sp * (phone ? 1.5 : 1.25)),
        Expanded(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            mainAxisSize: MainAxisSize.min,
            children: [
              RoleTagRow(
                name: s.displayName,
                role: s.role,
                launchRole: s.launchRole,
                dot: widget.unread
                    ? UnseenDot(
                        key: Key('unread-${s.id}'),
                        message: UnseenDot.sessionMessage,
                      )
                    : null,
                style: TextStyle(
                  fontFamily: StormTokens.sansFamily,
                  fontSize: phone ? t.uiSize : t.codeSize,
                  color: quietName ? t.text2 : t.text,
                ),
              ),
              SizedBox(height: t.sp * (phone ? 0.25 : 0.125)),
              KeyedSubtree(key: Key('sub-${s.id}'), child: subLine()),
            ],
          ),
        ),
        // Folded, or under the pointer: an open tree shows no chevron, as
        // the handoff draws it. A phone has no pointer, so it always shows.
        if (tree != null &&
            tree.size > 0 &&
            (tree.collapsed || _hover || phone)) ...[
          SizedBox(width: t.sp * 0.5),
          ExcludeSemantics(
            child: GestureDetector(
              key: Key('tree-chevron-${s.id}'),
              behavior: HitTestBehavior.opaque,
              onTap: tree.onToggle,
              child: SizedBox(
                width: t.sp * 2,
                height: t.sp * 2.5,
                child: Icon(
                  tree.collapsed
                      ? LucideIcons.chevron_right
                      : LucideIcons.chevron_down,
                  size: t.labelSize,
                  color: t.text3,
                ),
              ),
            ),
          ),
        ],
        if (widget.onMenu != null && _hover) ...[
          SizedBox(width: t.sp * 0.5),
          CrewDots(
            key: Key('session-menu-${s.id}'),
            label: 'Session actions',
            onTap: () => widget.onMenu!(_anchor),
          ),
        ],
      ],
    );
    final row = Semantics(
      button: true,
      selected: widget.selected,
      customSemanticsActions: tree == null || tree.size == 0
          ? null
          : {
              CustomSemanticsAction(
                label: tree.collapsed ? 'Expand' : 'Collapse',
              ): tree.onToggle,
            },
      label: [
        sessionSpoken(s, widget.host),
        if (tier == SidebarTier.needsYou) 'needs you',
        ?reason?.text,
        if (widget.unread) 'new output',
      ].join(', '),
      excludeSemantics: true,
      onTap: widget.onTap,
      onLongPress: widget.onLongPress,
      child: Material(
        key: _anchor,
        color: fill,
        shape: RoundedRectangleBorder(
          borderRadius: radius,
          side: outlined
              ? BorderSide(color: t.danger, width: t.bw)
              : BorderSide.none,
        ),
        child: InkWell(
          key: Key('session-${s.id}'),
          borderRadius: radius,
          onTap: widget.onTap,
          onLongPress: widget.onLongPress,
          child: Container(
            padding: phone
                ? EdgeInsets.symmetric(
                    vertical: t.sp * 1.5,
                    horizontal: outlined ? t.sp : 0,
                  )
                : EdgeInsets.symmetric(
                    horizontal: t.sp,
                    vertical: t.sp * 0.875,
                  ),
            decoration: phone && !outlined
                ? BoxDecoration(
                    border: Border(
                      bottom: BorderSide(color: t.border, width: t.bw),
                    ),
                  )
                : null,
            child: content,
          ),
        ),
      ),
    );
    final menu = widget.onMenu;
    if (menu == null && tree == null) return row;
    return MouseRegion(
      onEnter: (_) => setState(() => _hover = true),
      onExit: (_) => setState(() => _hover = false),
      child: menu == null
          ? row
          : GestureDetector(onSecondaryTapUp: (_) => menu(_anchor), child: row),
    );
  }
}

class _Line extends StatelessWidget {
  const _Line(this.text, this.style);

  final String text;
  final TextStyle style;

  @override
  Widget build(BuildContext context) => Text(
    text,
    maxLines: 1,
    softWrap: false,
    overflow: TextOverflow.ellipsis,
    style: style,
  );
}

/// "build-vm · storm · 40m", where only the workspace gives way: the host,
/// what leads it and the age are never cut (decision 138l). The sidebar's
/// says no model or mode (153e: "workspace · age").
class SessionSubLine extends StatelessWidget {
  const SessionSubLine({
    super.key,
    required this.session,
    required this.host,
    required this.style,
    this.lead = const [],
    this.details = true,
  });

  final AgentSession session;

  /// Null leaves the host out.
  final String? host;
  final TextStyle style;

  /// Said before the workspace: "2 agents", "from gateway-impl".
  final List<String> lead;

  /// The model and permission mode beside the age.
  final bool details;

  @override
  Widget build(BuildContext context) {
    final scaler = MediaQuery.textScalerOf(context);
    final dir = Directionality.of(context);
    // What the Text below draws with, letter spacing included.
    final drawn = DefaultTextStyle.of(context).style.merge(style);
    // A child's workspace is `children/<id>` on its host: named by what it
    // is instead, as its meta line does (134l).
    final ws = session.spawn.isChild
        ? childWorkspaceLabel(session)
        : session.workspace;
    // The model, when one was chosen (145b), stays whole with the age.
    final when = [
      if (details) ...[
        ?session.model,
        ?permissionModeWord(session.permissionMode),
      ],
      sessionWhen(session),
    ].join(' · ');
    final head = [?host, ...lead];
    return LayoutBuilder(
      builder: (context, box) {
        String line(String? w) => [...head, ?w, when].join(' · ');
        bool fits(String text) =>
            (TextPainter(
              text: TextSpan(text: text, style: drawn),
              textDirection: dir,
              textScaler: scaler,
              maxLines: 1,
            )..layout()).width <=
            box.maxWidth;
        var text = line(ws);
        if (box.hasBoundedWidth && !fits(text)) {
          var lo = 0, hi = ws.length;
          while (lo < hi) {
            final mid = (lo + hi + 1) ~/ 2;
            fits(line('${ws.substring(0, mid)}…')) ? lo = mid : hi = mid - 1;
          }
          text = line('${ws.substring(0, lo)}…');
          // No room for even a cut workspace: it goes, so the host and age
          // stay whole (the overview card at 900).
          if (lo == 0 && !fits(text)) text = line(null);
        }
        return Text(
          text,
          maxLines: 1,
          softWrap: false,
          overflow: TextOverflow.ellipsis,
          style: style,
        );
      },
    );
  }
}

/// The note a session started from: a file icon and its title, outlined.
class NoteContextChip extends StatelessWidget {
  const NoteContextChip({super.key, required this.title, this.maxWidth});

  final String title;

  /// The title's room; 160 by default.
  final double? maxWidth;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Container(
      padding: EdgeInsets.symmetric(horizontal: t.sp, vertical: t.sp * 0.25),
      decoration: BoxDecoration(
        borderRadius: BorderRadius.circular(t.rControl),
        border: Border.all(color: t.border, width: t.bw),
      ),
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          Icon(LucideIcons.file, size: t.labelSize, color: t.text2),
          SizedBox(width: t.sp * 0.625),
          ConstrainedBox(
            constraints: BoxConstraints(maxWidth: maxWidth ?? t.sp * 20),
            child: Text(
              title,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: TextStyle(
                fontFamily: StormTokens.sansFamily,
                fontSize: t.labelSize * 1.09,
                color: t.text2,
              ),
            ),
          ),
        ],
      ),
    );
  }
}

/// *＋ New session*, the phone list's one labelled primary in the pill.
class NewSessionPill extends StatelessWidget {
  const NewSessionPill({
    super.key,
    required this.onTap,
    this.label = 'New session',
  });

  final VoidCallback onTap;

  /// "New session in crew" on a phone's crew page.
  final String label;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return SafeArea(
      minimum: EdgeInsets.only(bottom: t.sp * 3.25),
      child: Align(
        alignment: Alignment.bottomCenter,
        child: StormPill(
          child: Semantics(
            button: true,
            label: label,
            excludeSemantics: true,
            onTap: onTap,
            child: Material(
              color: t.accent,
              shape: const StadiumBorder(),
              child: InkWell(
                key: const Key('new-session-pill'),
                customBorder: const StadiumBorder(),
                onTap: onTap,
                child: Container(
                  height: t.sp * 6,
                  padding: EdgeInsets.symmetric(horizontal: t.sp * 2.75),
                  child: Row(
                    mainAxisSize: MainAxisSize.min,
                    children: [
                      Icon(
                        LucideIcons.plus,
                        size: t.bodySize,
                        color: t.onAccent,
                      ),
                      SizedBox(width: t.sp),
                      Text(
                        label,
                        style: TextStyle(
                          fontFamily: StormTokens.sansFamily,
                          fontSize: t.bodySize,
                          fontWeight: FontWeight.w500,
                          color: t.onAccent,
                        ),
                      ),
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

/// A heading in the Agents pane's own rhythm: mono, spaced, `text3`.
class AgentsLabel extends StatelessWidget {
  const AgentsLabel(this.text, {super.key});

  final String text;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Text(text.toUpperCase(), style: agentKey(t));
  }
}

/// The agent's mark on a rounded tile, what the agent is doing a badge on
/// its corner (D15 AM44, 153e): filled `accent` and pulsing while it works,
/// an `accent` ring while idle, `text3` while starting, `danger` once
/// failed. Parked, out of usage and unknown have none; an ended session's
/// tile is faded.
class AgentMark extends StatelessWidget {
  const AgentMark({
    super.key,
    required this.session,
    this.size,
    this.background,
  });

  final AgentSession session;
  final double? size;

  /// What the badge's ring cuts out of: the row behind the tile.
  final Color? background;

  static const claudeOrange = Color(0xFFD97757);

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final s = session;
    final side = size ?? t.sp * 4;
    final glyph = side * 0.6;
    final mark = ProviderGlyph(s.provider, size: glyph);
    final outOfUsage =
        s.spawn.attention?.kind == 'usage_limit' ||
        s.spawn.needsAttention == 'usage_limit';
    final idle = s.status == 'running' && !s.working && !outOfUsage;
    final badge = switch (s.status) {
      'running' when outOfUsage => null,
      'running' => t.accent,
      'creating' || 'starting' => t.text3,
      'failed' => t.danger,
      _ => null,
    };
    final dot = side * 0.34;
    return SizedBox(
      key: Key('mark-${s.id}'),
      width: side,
      height: side,
      child: Stack(
        clipBehavior: Clip.none,
        children: [
          Opacity(
            opacity: s.ended ? 0.5 : 1,
            child: Container(
              width: side,
              height: side,
              alignment: Alignment.center,
              decoration: BoxDecoration(
                color: t.surface2,
                borderRadius: BorderRadius.circular(t.rControl),
              ),
              child: mark,
            ),
          ),
          if (badge != null)
            Positioned(
              right: -dot * 0.25,
              bottom: -dot * 0.25,
              child: _Badge(
                key: Key('badge-${s.id}'),
                color: badge,
                ring: background ?? t.bg,
                hollow: idle,
                size: dot,
                pulse: s.status == 'running' && s.working,
              ),
            ),
        ],
      ),
    );
  }
}

/// An agent's own mark, or a generic one for an agent without one.
class ProviderGlyph extends StatelessWidget {
  const ProviderGlyph(this.provider, {super.key, required this.size});

  final String provider;
  final double size;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    Widget svg(String name, double side, Color color) => SvgPicture.asset(
      'assets/agents/$name.svg',
      width: side,
      height: side,
      colorFilter: ColorFilter.mode(color, BlendMode.srcIn),
    );
    return switch (provider) {
      'claude-code' => svg('claude-code', size, AgentMark.claudeOrange),
      'opencode' => svg('opencode', size * 0.8, t.text),
      'codex' => svg('codex', size * 0.9, t.text),
      'cursor' => svg('cursor', size * 0.9, t.text),
      'command-code' => svg('command-code', size * 0.8, t.text),
      'shell' => Icon(LucideIcons.square_terminal, size: size, color: t.text2),
      _ => Icon(LucideIcons.bot, size: size, color: t.text2),
    };
  }
}

/// The session's status as a bare dot, in the [AgentMark] badge's colours:
/// pulsing while the agent works, danger once failed; otherwise the
/// status's own colour (the phone session header). Never amber, which is
/// for tags (153e).
class SessionHeaderDot extends StatelessWidget {
  const SessionHeaderDot({super.key, required this.session, this.size});

  final AgentSession session;
  final double? size;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final s = session;
    final color = switch (s.status) {
      'creating' || 'starting' || 'unknown' => t.text3,
      'failed' => t.danger,
      _ => sessionStatusLook(s.status, t).color,
    };
    return _Badge(
      key: const Key('session-dot'),
      color: color,
      fade: t.bg,
      size: size ?? t.sp * 0.875,
      pulse: s.status == 'running' && s.working,
    );
  }
}

class _Badge extends StatefulWidget {
  const _Badge({
    super.key,
    required this.color,
    this.ring,
    this.fade,
    this.hollow = false,
    required this.size,
    required this.pulse,
  });

  final Color color;

  /// Cut out of what is behind the badge; none for a bare dot.
  final Color? ring;

  /// A ring of [color] around [ring]'s colour: idle.
  final bool hollow;

  /// What a pulse fades towards, when there is no [ring].
  final Color? fade;
  final double size;
  final bool pulse;

  @override
  State<_Badge> createState() => _BadgeState();
}

class _BadgeState extends State<_Badge> with SingleTickerProviderStateMixin {
  late final _pulse = AnimationController(
    vsync: this,
    duration: const Duration(milliseconds: 1100),
  );

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    _sync();
  }

  @override
  void didUpdateWidget(_Badge old) {
    super.didUpdateWidget(old);
    _sync();
  }

  void _sync() {
    final animate = widget.pulse && !MediaQuery.disableAnimationsOf(context);
    if (animate && !_pulse.isAnimating) {
      _pulse.repeat(reverse: true);
    } else if (!animate && _pulse.isAnimating) {
      _pulse
        ..stop()
        ..value = 0;
    }
  }

  @override
  void dispose() {
    _pulse.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return AnimatedBuilder(
      animation: _pulse,
      builder: (context, _) => Container(
        width: widget.size,
        height: widget.size,
        decoration: widget.hollow
            ? BoxDecoration(
                shape: BoxShape.circle,
                color: widget.ring,
                border: Border.all(color: widget.color, width: t.bw * 2),
              )
            : BoxDecoration(
                shape: BoxShape.circle,
                color: Color.lerp(
                  widget.color,
                  widget.ring ?? widget.fade,
                  _pulse.value * 0.55,
                ),
                border: widget.ring == null
                    ? null
                    : Border.all(color: widget.ring!, width: t.bw * 2),
              ),
      ),
    );
  }
}
