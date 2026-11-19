/// Settings › About & health's "Send problem reports" switch (decision
/// 162k, the server's `feedback.enabled`). The same `GET /v1/feedback` the
/// report sheet itself reads to decide whether Send works — flip this off
/// and the sheet says reporting is turned off and refuses to send.
library;

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../state/feedback_state.dart'
    show feedbackApi, feedbackStatusProvider;
import '../controls.dart' show SettingsRow, StormToggle;

class FeedbackToggleRow extends ConsumerWidget {
  const FeedbackToggleRow({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final status = ref.watch(feedbackStatusProvider).value;
    final enabled = status?.enabled ?? true;
    return SettingsRow(
      key: const Key('feedback-enabled'),
      label: 'Send problem reports',
      sub: status == null
          ? 'Loading…'
          : enabled
          ? 'Lets "Report a problem" send through your own server.'
          : 'Off: the report sheet says so, and nothing is sent.',
      trailing: StormToggle(
        key: const Key('feedback-enabled-toggle'),
        value: enabled,
        onChanged: status == null ? null : (on) => _set(ref, on),
      ),
    );
  }

  Future<void> _set(WidgetRef ref, bool on) async {
    final api = feedbackApi(ref);
    if (api == null) return;
    try {
      await api.setEnabled(on);
    } finally {
      api.dispose();
      ref.invalidate(feedbackStatusProvider);
    }
  }
}
