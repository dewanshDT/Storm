import 'package:flutter/material.dart';
import 'package:flutter_lucide/flutter_lucide.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../ui/icons.dart';
import '../state/health.dart' show relativeTime;
import '../ui/shell/nav_bubble.dart' show PrimaryCircle, StormPill;
import '../ui/tokens.dart';
import '../ui/widgets.dart';
import 'agent_models.dart';
import 'agent_state.dart' show agentTitlesProvider;

/// The pieces every agent surface draws a session with (decision 78).
///
/// The dashboard band, the phone list and the wide sidebar all show the same
/// row, so a session reads the same wherever it is met.

/// "Claude Code in storm": what is running, and where.
String sessionTitle(AgentSession? s) =>
    s == null ? 'Session' : '${providerLabel(s.provider)} in ${s.workspace}';

/// Agents cannot run without the server, and saying so is a state, not an
/// error (the offline ground rule). One wording wherever it appears.
const kAgentsOfflineTitle = 'Agents need the server';
const kAgentsOfflineDetail = "Nothing to show until it's back.";

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

/// The row every agent list is made of: a lead column (a dot or an icon), a
/// title over one line of detail, and whatever trails. The band's idle line
/// and a session use the same frame, so their titles share one left edge.
class AgentRow extends StatelessWidget {
  const AgentRow({
    super.key,
    required this.title,
    required this.onTap,
    this.lead,
    this.detail,
    this.trailing,
    this.muted = false,
    this.selected = false,
    this.dense = false,
  });

  final String title;
  final VoidCallback onTap;
  final Widget? lead;
  final String? detail;
  final Widget? trailing;

  /// An ended session's title steps back a shade.
  final bool muted;
  final bool selected;

  /// The sidebar's tighter rhythm.
  final bool dense;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
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
                // The same column whether or not there is a lead, so titles
                // line up down the list.
                SizedBox(
                  width: t.sp * 2.75,
                  child: lead == null
                      ? null
                      : Align(alignment: Alignment.centerLeft, child: lead),
                ),
                Expanded(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Text(
                        title,
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: TextStyle(
                          fontFamily: StormTokens.sansFamily,
                          fontSize: dense ? t.codeSize : t.bodySize,
                          fontWeight: FontWeight.w600,
                          color: muted ? t.text2 : t.text,
                        ),
                      ),
                      if (detail != null) ...[
                        SizedBox(height: t.sp * 0.25),
                        Text(
                          detail!,
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                          style: TextStyle(
                            fontFamily: StormTokens.sansFamily,
                            fontSize: dense ? t.labelSize : t.codeSize,
                            color: t.text3,
                          ),
                        ),
                      ],
                    ],
                  ),
                ),
                if (trailing != null) ...[SizedBox(width: t.sp), trailing!],
              ],
            ),
          ),
        ),
      ),
    );
  }
}

/// The name an agent gave its conversation, from a raw terminal title, or
/// null when the title says nothing a list does not already say.
///
/// Claude Code prefixes its title with a status glyph (`✳`, or a braille
/// spinner while working) and both CLIs start on their own product name, so
/// leading symbols are dropped and a bare product name counts as no name.
String? agentChosenTitle(String? raw) {
  if (raw == null) return null;
  final name = raw.replaceFirst(RegExp(r'^[^\p{L}\p{N}]+', unicode: true), '');
  final trimmed = name.trim();
  const generic = {'claude code', 'claude', 'opencode', 'open code'};
  if (trimmed.isEmpty || generic.contains(trimmed.toLowerCase())) return null;
  return trimmed;
}

/// A provider's mark. Lucide has no brand logos, so each gets the glyph
/// nearest its own identity: Claude Code titles itself with an asterisk.
IconData providerIcon(String provider) => switch (provider) {
  'claude-code' => LucideIcons.asterisk,
  'opencode' => LucideIcons.square_code,
  'shell' => LucideIcons.square_terminal,
  _ => LucideIcons.bot,
};

/// The title a session is shown under: the agent's own name for it when
/// this device has seen one, else "Claude Code in storm".
String sessionDisplayTitle(AgentSession? s, Map<String, String> titles) =>
    (s == null ? null : titles[s.id]) ?? sessionTitle(s);

/// One session: its agent, its name, host and age, and its status at the
/// end of the row.
class AgentSessionRow extends ConsumerWidget {
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
  final bool dense;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final t = context.tokens;
    final s = session;
    final dot = sessionDot(s);
    final titles = ref.watch(agentTitlesProvider);
    // Running says itself with the dot; anything else is said in words,
    // because an ended session has no dot (see sessionDot).
    final words = s.status == 'running' ? null : s.statusLabel;
    return AgentRow(
      title: sessionDisplayTitle(s, titles),
      // The same age wording as Recently opened, directly below the band.
      detail:
          '$hostName · '
          '${relativeTime(DateTime.tryParse(s.createdAt)?.toLocal())}',
      lead: Tooltip(
        message: providerLabel(s.provider),
        child: Icon(
          providerIcon(s.provider),
          size: dense ? t.bodySize : t.headingSize,
          color: s.ended ? t.text3 : t.text2,
        ),
      ),
      trailing: words != null
          ? Text(
              words,
              style: TextStyle(
                fontFamily: StormTokens.sansFamily,
                fontSize: t.labelSize,
                color: s.status == 'failed' ? t.danger : t.text3,
              ),
            )
          // Colour alone is not a status: the dot carries its word too.
          : Semantics(
              label: s.statusLabel,
              child: StatusDot(status: dot ?? DotStatus.synced),
            ),
      muted: s.ended,
      selected: selected,
      dense: dense,
      onTap: onTap,
    );
  }
}

/// *New session*, as a pill floating at the bottom of the phone list.
///
/// The nav bubble's own [StormPill] and [PrimaryCircle], because the Agents
/// space is a *space*, a peer of the vault screens, and should read as part of
/// the same app (decision 78). Settings-shaped screens (Hosts, MCP keys) keep
/// their extended FAB: that is the pattern for a page you visit to configure
/// something.
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
        child: StormPill(
          padding: EdgeInsets.zero,
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
                    const PrimaryCircle(glyph: StormGlyph.plus),
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
