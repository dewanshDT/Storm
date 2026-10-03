import 'package:flutter/material.dart';

import '../ui/icons.dart';
import '../ui/tokens.dart';
import '../ui/widgets.dart';
import 'agent_models.dart';

/// The pieces every agent surface draws a session with (decision 78).
///
/// The dashboard band, the phone list and the wide sidebar all show the same
/// row, so a session reads the same wherever it is met.

/// "Claude Code in storm": what is running, and where.
String sessionTitle(AgentSession? s) =>
    s == null ? 'Session' : '${providerLabel(s.provider)} in ${s.workspace}';

/// How long ago, coarsely: a list is scanned, not read.
String sessionAge(String iso) {
  final at = DateTime.tryParse(iso);
  if (at == null) return '';
  final d = DateTime.now().toUtc().difference(at.toUtc());
  if (d.inMinutes < 1) return 'just now';
  if (d.inHours < 1) return '${d.inMinutes} min ago';
  if (d.inDays < 1) return '${d.inHours} h ago';
  return '${d.inDays} d ago';
}

/// A live session's dot, in the one dot vocabulary: green is good, amber is
/// work in progress, grey is out of reach. Ended sessions get no dot — their
/// status is said in words instead, because a red dot already means "a server
/// that failed to prove who it is" and must not mean two things.
DotStatus? sessionDot(AgentSession s) => switch (s.status) {
  'running' => DotStatus.synced,
  'creating' || 'starting' => DotStatus.syncing,
  'unknown' => DotStatus.offline,
  _ => null,
};

/// One session: dot, title, host and age, and — once it has ended — how.
class AgentSessionRow extends StatelessWidget {
  const AgentSessionRow({
    super.key,
    required this.session,
    required this.hostName,
    required this.onTap,
    this.selected = false,
    this.dense = false,
  });

  final AgentSession session;
  final String hostName;
  final VoidCallback onTap;
  final bool selected;

  /// The sidebar's tighter rhythm.
  final bool dense;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final s = session;
    final dot = sessionDot(s);
    // A live session that is not plainly running says what it is instead.
    final trailing = s.ended || s.status != 'running' ? s.statusLabel : null;

    return Padding(
      padding: EdgeInsets.only(bottom: t.sp * (dense ? 0.25 : 0.5)),
      child: Material(
        color: selected ? t.accentSoft : Colors.transparent,
        borderRadius: BorderRadius.circular(t.rControl),
        child: InkWell(
          borderRadius: BorderRadius.circular(t.rControl),
          onTap: onTap,
          child: Padding(
            padding: EdgeInsets.symmetric(
              horizontal: t.sp * (dense ? 1 : 0.5),
              vertical: t.sp * (dense ? 0.9 : 1.25),
            ),
            child: Row(
              children: [
                // The same column whether or not there is a dot, so titles
                // line up down the list.
                SizedBox(
                  width: t.sp * 2.75,
                  child: dot == null
                      ? null
                      : Align(
                          alignment: Alignment.centerLeft,
                          child: StatusDot(status: dot),
                        ),
                ),
                Expanded(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Text(
                        sessionTitle(s),
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: TextStyle(
                          fontFamily: StormTokens.sansFamily,
                          fontSize: dense ? t.codeSize : t.bodySize,
                          fontWeight: FontWeight.w600,
                          color: s.ended ? t.text2 : t.text,
                        ),
                      ),
                      SizedBox(height: t.sp * 0.25),
                      Text(
                        '$hostName · ${sessionAge(s.createdAt)}',
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: TextStyle(
                          fontFamily: StormTokens.sansFamily,
                          fontSize: dense ? t.labelSize : t.codeSize,
                          color: t.text3,
                        ),
                      ),
                    ],
                  ),
                ),
                if (trailing != null) ...[
                  SizedBox(width: t.sp),
                  Text(
                    trailing,
                    style: TextStyle(
                      fontFamily: StormTokens.sansFamily,
                      fontSize: t.labelSize,
                      color: s.status == 'failed' ? t.danger : t.text3,
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

/// *New session*, as a pill floating at the bottom of the phone list.
///
/// The nav bubble's grammar — a pill is an action bar, the primary action a
/// filled circle inside it — because the Agents space is a *space*, a peer of
/// the vault screens, and should read as part of the same app (decision 78).
/// Settings-shaped screens (Hosts, MCP keys) keep their extended FAB: that is
/// the pattern for a page you visit to configure something.
class NewSessionPill extends StatelessWidget {
  const NewSessionPill({super.key, required this.onTap});

  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return SafeArea(
      minimum: EdgeInsets.only(bottom: t.sp * 2.5),
      child: Align(
        alignment: Alignment.bottomCenter,
        // Drawn exactly as the nav bubble is — surface, border, shadow on the
        // container — with a transparent Material inside only for the ripple.
        child: Container(
          decoration: BoxDecoration(
            color: t.surface.withValues(alpha: 0.96),
            borderRadius: BorderRadius.circular(999),
            border: Border.all(color: t.border, width: t.bw),
            boxShadow: t.shadow,
          ),
          child: Material(
            type: MaterialType.transparency,
            child: InkWell(
              key: const Key('new-session-pill'),
              customBorder: const StadiumBorder(),
              onTap: onTap,
              child: Padding(
                padding: EdgeInsets.fromLTRB(
                  t.sp * 0.625,
                  t.sp * 0.625,
                  t.sp * 2.5,
                  t.sp * 0.625,
                ),
                child: Row(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    Container(
                      width: t.sp * 5.5,
                      height: t.sp * 5.5,
                      alignment: Alignment.center,
                      decoration: BoxDecoration(
                        color: t.accent,
                        shape: BoxShape.circle,
                      ),
                      child: StormIcon(
                        StormGlyph.plus,
                        size: t.sp * 2.75,
                        color: t.onAccent,
                      ),
                    ),
                    SizedBox(width: t.sp * 1.5),
                    Text(
                      'New session',
                      style: TextStyle(
                        fontFamily: StormTokens.sansFamily,
                        fontSize: t.bodySize,
                        fontWeight: FontWeight.w600,
                        color: t.text,
                      ),
                    ),
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
