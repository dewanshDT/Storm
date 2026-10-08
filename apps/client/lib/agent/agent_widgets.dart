import 'package:flutter/material.dart';
import 'package:flutter_lucide/flutter_lucide.dart';

import '../ui/session_status.dart';
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

/// "storm · Claude Code · 40m": the sidebar and phone rows' sub-line.
String sessionSub(AgentSession s) =>
    '${s.workspace} · ${providerLabel(s.provider)} · ${sessionWhen(s)}';

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

/// One session in a list: its dot, its name, and a mono line under it. The
/// sidebar's rows are tighter and fill when selected; the phone's are
/// separated by hairlines (handoff §2.6, §2.8).
class SessionRow extends StatelessWidget {
  const SessionRow({
    super.key,
    required this.session,
    required this.onTap,
    this.selected = false,
    this.phone = false,
  });

  final AgentSession session;
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
        SessionStatusDot(status: s.status),
        SizedBox(width: t.sp * (phone ? 1.5 : 1.25)),
        Expanded(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            mainAxisSize: MainAxisSize.min,
            children: [
              Text(
                s.name,
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
                sessionSub(s),
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
      label: '${s.name}, ${s.statusLabel}',
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
