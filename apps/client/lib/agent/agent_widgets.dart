import 'package:flutter/material.dart';
import 'package:flutter_lucide/flutter_lucide.dart';
import 'package:flutter_svg/flutter_svg.dart';

import '../ui/session_status.dart';
import '../ui/shell/nav_bubble.dart' show StormPill;
import '../ui/tokens.dart';
import 'agent_models.dart';
import 'crew_widgets.dart' show RoleTagRow;
import 'spawn_widgets.dart'
    show AttentionTag, NeedsYouBadge, childWorkspaceLabel;

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
/// it. The sidebar's rows are tighter and fill when selected; the phone's
/// are separated by hairlines (handoff §2.6, §2.8; D15 AM44).
class SessionRow extends StatelessWidget {
  const SessionRow({
    super.key,
    required this.session,
    required this.host,
    required this.onTap,
    this.selected = false,
    this.phone = false,
    this.lineage,
    this.onLongPress,
    this.waiting,
  });

  final AgentSession session;
  final String host;
  final VoidCallback onTap;
  final bool selected;
  final bool phone;

  /// A spawned child's "↳ from …" in place of the sub-line (134l).
  final String? lineage;

  /// One of its children waiting at a permission prompt (146c).
  final AgentSession? waiting;

  /// The phone's crew and role sheet (decision 138n).
  final VoidCallback? onLongPress;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final s = session;
    final radius = BorderRadius.circular(phone ? 0 : t.rControl);
    final subStyle = agentMono(t);
    final content = Row(
      children: [
        AgentMark(
          session: s,
          size: t.sp * (phone ? 4.5 : 4),
          background: selected ? t.surface2 : (phone ? t.bg : t.surface),
        ),
        SizedBox(width: t.sp * (phone ? 1.5 : 1.25)),
        Expanded(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            mainAxisSize: MainAxisSize.min,
            children: [
              Row(
                children: [
                  Flexible(
                    child: RoleTagRow(
                      name: s.displayName,
                      role: s.role,
                      launchRole: s.launchRole,
                      style: TextStyle(
                        fontFamily: StormTokens.sansFamily,
                        fontSize: phone ? t.uiSize : t.codeSize,
                        color: s.ended ? t.text2 : t.text,
                      ),
                    ),
                  ),
                  if (!s.ended && s.spawn.needsAttention != null) ...[
                    SizedBox(width: t.sp * 0.75),
                    Flexible(child: AttentionTag(kind: s.spawn.needsAttention)),
                  ] else if (waiting case final w?) ...[
                    SizedBox(width: t.sp * 0.75),
                    Flexible(child: NeedsYouBadge(waiting: w)),
                  ],
                ],
              ),
              SizedBox(height: t.sp * (phone ? 0.25 : 0.125)),
              if (lineage case final line?)
                Text(
                  line,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: subStyle,
                )
              else
                SessionSubLine(session: s, host: host, style: subStyle),
            ],
          ),
        ),
      ],
    );
    return Semantics(
      button: true,
      selected: selected,
      label: sessionSpoken(s, host),
      excludeSemantics: true,
      onTap: onTap,
      onLongPress: onLongPress,
      child: Material(
        color: selected ? t.surface2 : Colors.transparent,
        borderRadius: radius,
        child: InkWell(
          key: Key('session-${s.id}'),
          borderRadius: radius,
          onTap: onTap,
          onLongPress: onLongPress,
          child: Container(
            padding: phone
                ? EdgeInsets.symmetric(vertical: t.sp * 1.5)
                : EdgeInsets.symmetric(
                    horizontal: t.sp,
                    vertical: t.sp * 0.875,
                  ),
            decoration: phone
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
  }
}

/// "build-vm · storm · 40m", where only the workspace gives way: the host
/// and the age are never cut (decision 138l).
class SessionSubLine extends StatelessWidget {
  const SessionSubLine({
    super.key,
    required this.session,
    required this.host,
    required this.style,
  });

  final AgentSession session;
  final String host;
  final TextStyle style;

  @override
  Widget build(BuildContext context) {
    final scaler = MediaQuery.textScalerOf(context);
    final dir = Directionality.of(context);
    // What the Text below draws with, letter spacing included.
    final drawn = DefaultTextStyle.of(context).style.merge(style);
    // A child's workspace is `children/<id>` on its host: named by what it
    // is instead, as its meta line does (134l).
    final ws = session.spawn.isChild
        ? childWorkspaceLabel(session.spawn.childWorkspace)
        : session.workspace;
    // The model, when one was chosen (145b), stays whole with the age.
    final when = [
      ?session.model,
      ?permissionModeWord(session.permissionMode),
      sessionWhen(session),
    ].join(' · ');
    return LayoutBuilder(
      builder: (context, box) {
        String line(String w) => '$host · $w · $when';
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
          if (lo == 0 && !fits(text)) text = '$host · $when';
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

/// The agent's mark on a rounded tile, the session's status a badge on its
/// corner (D15 AM44): pulsing while the agent works, amber while unknown,
/// danger once failed; an ended session's tile is faded.
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
    final badge = switch (s.status) {
      'running' when s.working => t.accent,
      'creating' || 'starting' => t.text3,
      'unknown' => t.amber,
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
/// pulsing while the agent works, amber while unknown, danger once failed;
/// otherwise the status's own colour (the phone session header).
class SessionHeaderDot extends StatelessWidget {
  const SessionHeaderDot({super.key, required this.session, this.size});

  final AgentSession session;
  final double? size;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final s = session;
    final color = switch (s.status) {
      'creating' || 'starting' => t.text3,
      'unknown' => t.amber,
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
    required this.size,
    required this.pulse,
  });

  final Color color;

  /// Cut out of what is behind the badge; none for a bare dot.
  final Color? ring;

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
        decoration: BoxDecoration(
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
