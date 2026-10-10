import 'package:flutter/material.dart';
import 'package:flutter_lucide/flutter_lucide.dart';
import 'package:flutter_svg/flutter_svg.dart';

import '../ui/shell/nav_bubble.dart' show StormPill;
import '../ui/tokens.dart';
import 'agent_models.dart';

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

String _clock(DateTime t) {
  final l = t.toLocal();
  String two(int n) => n.toString().padLeft(2, '0');
  return '${two(l.hour)}:${two(l.minute)}';
}

/// A live session's age, or an ended one's status in lower case.
String sessionWhen(AgentSession s) =>
    s.ended ? s.statusLabel.toLowerCase() : shortAge(s.createdAt);

/// "build-vm · storm · 40m": the rows' sub-line; the mark says the agent.
String sessionSub(AgentSession s, String host) =>
    '$host · ${s.workspace} · ${sessionWhen(s)}';

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
        : '${s.statusLabel} ${_clock(ended)}',
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
  return '${providerLabel(s.provider)} · ${s.workspace} on $hostName · $when';
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
  });

  final AgentSession session;
  final String host;
  final VoidCallback onTap;
  final bool selected;
  final bool phone;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final s = session;
    final radius = BorderRadius.circular(phone ? 0 : t.rControl);
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
              Text(
                s.displayName,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: TextStyle(
                  fontFamily: StormTokens.sansFamily,
                  fontSize: phone ? t.uiSize : t.codeSize,
                  color: s.ended ? t.text2 : t.text,
                ),
              ),
              SizedBox(height: t.sp * (phone ? 0.25 : 0.125)),
              Text(
                sessionSub(s, host),
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: TextStyle(
                  fontFamily: StormTokens.monoFamily,
                  fontSize: t.labelSize,
                  color: t.text3,
                ),
              ),
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
      child: Material(
        color: selected ? t.surface2 : Colors.transparent,
        borderRadius: radius,
        child: InkWell(
          key: Key('session-${s.id}'),
          borderRadius: radius,
          onTap: onTap,
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

/// The note a session started from: a file icon and its title, outlined.
class NoteContextChip extends StatelessWidget {
  const NoteContextChip({super.key, required this.title});

  final String title;

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
            constraints: BoxConstraints(maxWidth: t.sp * 20),
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
  const NewSessionPill({super.key, required this.onTap});

  final VoidCallback onTap;

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
            label: 'New session',
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
                        'New session',
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
    return Text(
      text.toUpperCase(),
      style: TextStyle(
        fontFamily: StormTokens.monoFamily,
        fontSize: t.labelSize,
        letterSpacing: t.labelSize * 0.08,
        color: t.text3,
      ),
    );
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
    final mark = switch (s.provider) {
      'claude-code' => SvgPicture.asset(
        'assets/agents/claude-code.svg',
        width: glyph,
        height: glyph,
        colorFilter: const ColorFilter.mode(claudeOrange, BlendMode.srcIn),
      ),
      'opencode' => SvgPicture.asset(
        'assets/agents/opencode.svg',
        width: glyph * 0.8,
        height: glyph * 0.8,
        colorFilter: ColorFilter.mode(t.text, BlendMode.srcIn),
      ),
      'shell' => Icon(LucideIcons.square_terminal, size: glyph, color: t.text2),
      _ => Icon(LucideIcons.bot, size: glyph, color: t.text2),
    };
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

class _Badge extends StatefulWidget {
  const _Badge({
    super.key,
    required this.color,
    required this.ring,
    required this.size,
    required this.pulse,
  });

  final Color color;
  final Color ring;
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
          color: Color.lerp(widget.color, widget.ring, _pulse.value * 0.55),
          border: Border.all(color: widget.ring, width: t.bw * 2),
        ),
      ),
    );
  }
}
