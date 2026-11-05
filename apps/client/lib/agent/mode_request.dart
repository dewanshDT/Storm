import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../ui/controls.dart';
import '../ui/states.dart' show describeFailure;
import '../ui/tokens.dart';
import 'agent_api.dart' show AgentRefusal;
import 'agent_models.dart';
import 'agent_state.dart';
import 'agent_widgets.dart' show agentKey, agentMono, agentSans;
import 'spawn_api.dart';

/// What the person is told auto does before they approve it (147c).
const kAutoExplained =
    'Auto lets Claude Code run routine actions, shell commands included, '
    'without asking; its own safety check reviews each one first. Storm '
    'restarts the session in Auto and keeps the conversation.';

/// "Asks for Auto" on a session's row while its request waits (147c).
class ModeAskTag extends StatelessWidget {
  const ModeAskTag({super.key});

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Container(
      key: const Key('mode-ask-tag'),
      padding: EdgeInsets.symmetric(
        horizontal: t.sp * 0.75,
        vertical: t.sp * 0.125,
      ),
      decoration: BoxDecoration(
        color: t.amberSoft,
        borderRadius: BorderRadius.circular(999),
      ),
      child: Text(
        'Asks for Auto',
        maxLines: 1,
        overflow: TextOverflow.ellipsis,
        style: agentMono(t, color: t.amber),
      ),
    );
  }
}

/// The one card Storm keeps (decision 147c): an agent asks to switch its
/// session to auto, which widens what it may do without asking, so only the
/// person decides. The reason is the agent's own text, shown as plain data.
/// Approve switches the session (a restart in auto that keeps the
/// conversation) and follows it there; Deny answers the agent `denied`.
class ModeRequestCard extends ConsumerStatefulWidget {
  const ModeRequestCard({super.key, required this.session, this.now});

  final AgentSession session;

  /// The clock, for tests.
  final DateTime Function()? now;

  @override
  ConsumerState<ModeRequestCard> createState() => _ModeRequestCardState();
}

class _ModeRequestCardState extends ConsumerState<ModeRequestCard> {
  bool _busy = false;

  /// This device approved it: follow the switch once it has somewhere to go.
  bool _follow = false;
  bool _followed = false;
  Timer? _poll;
  Timer? _tick;

  @override
  void initState() {
    super.initState();
    // The expiry counts down while the card is shown.
    _tick = Timer.periodic(const Duration(seconds: 30), (_) {
      if (mounted) setState(() {});
    });
  }

  @override
  void dispose() {
    _poll?.cancel();
    _tick?.cancel();
    super.dispose();
  }

  Future<void> _decide(ModeRequest r, bool approve) async {
    final api = agentApi(ref);
    if (api == null) return;
    setState(() => _busy = true);
    final c = ProviderScope.containerOf(context, listen: false);
    final id = widget.session.id;
    try {
      await api.decideModeRequest(id, r.id, approve: approve);
      if (approve) {
        _follow = true;
        // The old session ends, so its record stops refreshing itself.
        var left = 90;
        _poll = Timer.periodic(const Duration(seconds: 1), (timer) {
          if (--left <= 0) timer.cancel();
          c.invalidate(sessionDetailProvider(id));
          c.invalidate(agentOverviewProvider);
        });
      }
    } catch (e) {
      if (!mounted) return;
      final code = e is AgentRefusal ? e.code : null;
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(
          content: Text(switch (code) {
            'request_expired' => 'That request expired: the agent was told.',
            'request_denied' ||
            'request_approved' => 'It was already answered.',
            _ => describeFailure(e),
          }),
        ),
      );
    } finally {
      api.dispose();
      c.invalidate(sessionDetailProvider(id));
      if (mounted) setState(() => _busy = false);
    }
  }

  String _expiry(ModeRequest r) {
    final at = r.expiresAt;
    if (at == null) return '';
    final left = at.difference((widget.now ?? DateTime.now)());
    if (left.isNegative) return 'Expiring now.';
    final minutes = (left.inSeconds / 60).ceil();
    return minutes <= 1
        ? 'Expires in under a minute.'
        : 'Expires in $minutes minutes.';
  }

  @override
  Widget build(BuildContext context) {
    final s = widget.session;
    final r = s.modeRequest;
    if (r == null || (!r.pending && !r.approved)) {
      return const SizedBox.shrink();
    }
    final to = r.switchedTo;
    if (r.approved && to != null && _follow && !_followed && to != s.id) {
      _followed = true;
      _poll?.cancel();
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (mounted) openAgentSession(context, to);
      });
    }
    final t = context.tokens;
    final name = s.displayName;
    final title = r.pending
        ? '$name asks to switch to Auto'
        : r.switchFailed
        ? 'Auto was approved, but $name could not be restarted in it'
        : to == null
        ? 'Restarting $name in Auto…'
        : 'Restarted in Auto';
    return Semantics(
      container: true,
      liveRegion: true,
      child: Container(
        key: const Key('mode-request-card'),
        padding: EdgeInsets.symmetric(
          horizontal: t.sp * 2,
          vertical: t.sp * 1.5,
        ),
        decoration: BoxDecoration(
          color: t.surface2,
          borderRadius: BorderRadius.circular(t.rControl),
          border: Border.all(
            color: r.pending ? t.amber : t.border,
            width: t.bw,
          ),
        ),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Text(
              title,
              key: const Key('mode-request-title'),
              style: agentSans(t, weight: FontWeight.w600),
            ),
            if (r.pending) ...[
              SizedBox(height: t.sp),
              Text('THE AGENT SAYS', style: agentKey(t)),
              SizedBox(height: t.sp * 0.5),
              // The agent's words: plain text, never a link or markup.
              Container(
                padding: EdgeInsets.all(t.sp),
                decoration: BoxDecoration(
                  color: t.surface,
                  borderRadius: BorderRadius.circular(t.rControl),
                ),
                child: Text(
                  r.reason.isEmpty ? '(no reason given)' : r.reason,
                  key: const Key('mode-request-reason'),
                  style: agentMono(t, color: t.text2),
                ),
              ),
              SizedBox(height: t.sp),
              Text(
                '$kAutoExplained ${_expiry(r)}',
                key: const Key('mode-request-note'),
                style: agentSans(t, color: t.text2),
              ),
              SizedBox(height: t.sp * 1.25),
              Wrap(
                alignment: WrapAlignment.end,
                spacing: t.sp,
                runSpacing: t.sp * 0.5,
                children: [
                  StormButton.outline(
                    key: const Key('mode-request-deny'),
                    label: 'Deny',
                    onPressed: _busy ? null : () => _decide(r, false),
                  ),
                  StormButton.primary(
                    key: const Key('mode-request-approve'),
                    label: 'Approve Auto',
                    onPressed: _busy ? null : () => _decide(r, true),
                  ),
                ],
              ),
            ] else if (to != null && to != s.id) ...[
              SizedBox(height: t.sp),
              Align(
                alignment: Alignment.centerRight,
                child: StormButton.primary(
                  key: const Key('mode-request-open'),
                  label: 'Open it',
                  onPressed: () => openAgentSession(context, to),
                ),
              ),
            ],
          ],
        ),
      ),
    );
  }
}
