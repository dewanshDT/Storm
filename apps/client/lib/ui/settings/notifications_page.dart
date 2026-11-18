/// Settings › Notifications (decision 161, slice 161-3). The account-level
/// sections only; "This device" at the top is reserved for another slice.
library;

import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../agent/agent_api.dart' show AgentRefusal;
import '../../agent/agent_models.dart' show providerLabel;
import '../../agent/agent_state.dart' show agentApi, agentOverviewProvider;
import '../../agent/agent_widgets.dart' show clockAt;
import '../../agent/notifications_models.dart';
import '../../agent/notifications_state.dart';
import '../controls.dart';
import '../states.dart' show describeFailure;
import '../tokens.dart';
import '../widgets.dart' show StormInput;
import 'settings_widgets.dart';

const _kKindLabels = {
  'needs_you': 'Needs you',
  'delete_request': 'Delete requests',
  'finished': 'Finished its turn',
  'error': 'Hit an error',
  'usage_limit': 'Usage limit',
  'host_offline': 'Host offline',
  'child_finished': 'Child finished',
  'stopped_working': 'Stopped working',
};

/// Presets for `finished`/`host_offline`'s threshold, in seconds.
const _kThresholdPresets = [0, 30, 120, 600];

String _thresholdLabel(int s) => switch (s) {
  0 => 'Immediately',
  30 => 'After 30s',
  120 => 'After 2m',
  600 => 'After 10m',
  _ => '${s}s',
};

const _kSessionLimitPresets = [5, 20, 50, 100, 200];
const _kRetentionPresets = [7, 30, 90, 365];

/// "14:02", or "3/4 14:02" for a moment not today; "you turn them back on"
/// for [NotificationPrefs.pausedUntil]'s far-future sentinel
/// (`PAUSE_INDEFINITELY`).
/// Runs [action], showing any failure rather than swallowing it.
Future<void> _guard(
  BuildContext context,
  Future<void> Function() action,
) async {
  try {
    await action();
  } catch (e) {
    if (context.mounted) settingsToast(context, describeFailure(e));
  }
}

String friendlyUntil(DateTime until) {
  if (until.year >= 9999) return 'you turn them back on';
  final local = until.toLocal();
  final now = DateTime.now();
  final sameDay =
      local.year == now.year &&
      local.month == now.month &&
      local.day == now.day;
  return sameDay
      ? clockAt(until)
      : '${local.month}/${local.day} ${clockAt(until)}';
}

class NotificationsPage extends ConsumerWidget {
  const NotificationsPage({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final state = ref.watch(notificationsStateProvider);
    if (state == null) {
      return const SettingsPage(
        title: 'Notifications',
        intro: 'Sign in to change notification settings.',
        children: [],
      );
    }
    return ListenableBuilder(
      listenable: state,
      builder: (context, _) => _NotificationsPageBody(state: state),
    );
  }
}

class _NotificationsPageBody extends ConsumerStatefulWidget {
  const _NotificationsPageBody({required this.state});

  final NotificationsState state;

  @override
  ConsumerState<_NotificationsPageBody> createState() =>
      _NotificationsPageBodyState();
}

class _NotificationsPageBodyState
    extends ConsumerState<_NotificationsPageBody> {
  final _tz = TextEditingController();
  String? _tzEditedFor;

  @override
  void dispose() {
    _tz.dispose();
    super.dispose();
  }

  NotificationsState get _state => widget.state;

  Future<void> _apply(Map<String, dynamic> patch) async {
    try {
      await _state.putPrefs(patch);
    } catch (e) {
      if (!mounted) return;
      if (e is AgentRefusal && e.code == 'prefs_version') {
        await _state.reloadPrefs();
        if (mounted) {
          settingsToast(context, 'Someone changed this elsewhere — reloaded.');
        }
      } else {
        settingsToast(context, describeFailure(e));
      }
    }
  }

  Map<String, dynamic> _eventPatch(
    NotificationPrefs prefs,
    String kind, {
    bool? on,
    int? thresholdS,
  }) {
    final current = prefs.events[kind];
    return {
      'events': {
        kind: {
          'on': on ?? current?.on ?? defaultEventOn(kind),
          if ((thresholdS ?? current?.thresholdS) != null)
            'threshold_s': thresholdS ?? current?.thresholdS,
        },
      },
    };
  }

  Future<void> _toggleAgentDefault(bool value, {required bool children}) async {
    if (!value) {
      final ok = await confirmSetting(
        context,
        title: 'Also turn off for running sessions?',
        body: children
            ? 'Sessions started by agents that can already notify keep doing '
                  'so unless you turn it off for them too.'
            : 'Sessions that can already notify keep doing so unless you '
                  'turn it off for them too.',
        action: 'Turn off',
        cancelLabel: 'Keep them on',
        confirmKey: const Key('notify-default-off-confirm'),
      );
      if (ok) {
        final api = agentApi(ref);
        if (api != null) {
          try {
            await api.turnNotifyOff(children ? 'children' : 'all');
          } catch (e) {
            if (mounted) settingsToast(context, describeFailure(e));
          } finally {
            api.dispose();
          }
        }
      }
    }
    await _apply({
      children ? 'children_default_on' : 'agent_default_on': value,
    });
  }

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final state = _state;
    if (state.unsupported) {
      return const SettingsPage(
        title: 'Notifications',
        intro: 'Not available on this server yet.',
        children: [],
      );
    }
    final prefs = state.prefs;
    if (state.loading || prefs == null) {
      return const SettingsPage(
        title: 'Notifications',
        intro: 'Loading…',
        children: [SettingsMuted('Loading…')],
      );
    }
    if (_tzEditedFor != prefs.quietTz) {
      _tz.text = prefs.quietTz;
      _tzEditedFor = prefs.quietTz;
    }

    return SettingsPage(
      title: 'Notifications',
      intro: 'What Storm tells you about, and when.',
      children: [
        const GroupLabel('What Storm tells you about', bottom: 10),
        for (final kind in kNotificationEventKinds)
          _EventRow(
            kind: kind,
            pref: prefs.events[kind],
            onToggle: (v) => _apply(_eventPatch(prefs, kind, on: v)),
            onThreshold: !const {'finished', 'host_offline'}.contains(kind)
                ? null
                : (v) => _apply(_eventPatch(prefs, kind, thresholdS: v)),
          ),

        GroupLabel('Messages from agents', top: t.sp * 3),
        SettingsRow(
          label: 'New sessions can notify you',
          trailing: StormToggle(
            key: const Key('notify-agent-default'),
            value: prefs.agentDefaultOn,
            onChanged: (v) => _toggleAgentDefault(v, children: false),
          ),
        ),
        SettingsRow(
          label: 'Agents started by agents can notify',
          trailing: StormToggle(
            key: const Key('notify-children-default'),
            value: prefs.childrenDefaultOn,
            onChanged: (v) => _toggleAgentDefault(v, children: true),
          ),
        ),
        SettingsRow(
          label: 'Limit per session',
          divider: false,
          trailing: ChoiceChips<int>(
            options: [
              for (final n in _kSessionLimitPresets) ChoiceOption(n, '$n/hr'),
            ],
            selected: prefs.perSessionLimit,
            onSelected: (v) => _apply({'per_session_limit': v}),
          ),
        ),
        SizedBox(height: t.sp * 1.5),
        _MutesList(state: state),

        GroupLabel('Quiet hours', top: t.sp * 3),
        SettingsRow(
          label: 'Quiet hours',
          trailing: StormToggle(
            key: const Key('notify-quiet-on'),
            value: prefs.quietOn,
            onChanged: (v) => _apply({'quiet_on': v}),
          ),
        ),
        SettingsRow(
          label: 'Start',
          trailing: _TimeField(
            key: const Key('notify-quiet-start'),
            value: prefs.quietStart,
            onChanged: (v) => _apply({'quiet_start': v}),
          ),
        ),
        SettingsRow(
          label: 'End',
          trailing: _TimeField(
            key: const Key('notify-quiet-end'),
            value: prefs.quietEnd,
            onChanged: (v) => _apply({'quiet_end': v}),
          ),
        ),
        SettingsRow(
          label: 'Time zone',
          sub:
              'An IANA zone, e.g. "America/New_York" or "UTC". Storm has no '
              'reliable way to detect this device\'s zone, so it keeps '
              'whatever was last saved.',
          trailing: SizedBox(
            width: t.sp * 16,
            child: StormInput(
              key: const Key('notify-quiet-tz'),
              controller: _tz,
              onSubmitted: (v) => _apply({'quiet_tz': v.trim()}),
            ),
          ),
        ),
        SettingsRow(
          label: 'Delete requests: still notify',
          trailing: StormToggle(
            key: const Key('notify-quiet-allow-delete'),
            value: prefs.quietAllowDeleteRequests,
            onChanged: (v) => _apply({'quiet_allow_delete_requests': v}),
          ),
        ),
        SettingsRow(
          label: 'Summary when quiet hours end',
          divider: false,
          trailing: StormToggle(
            key: const Key('notify-summary-on'),
            value: prefs.summaryOn,
            onChanged: (v) => _apply({'summary_on': v}),
          ),
        ),
        SizedBox(height: t.sp * 1.5),
        _PauseControl(state: state, prefs: prefs),

        GroupLabel('Inbox', top: t.sp * 3),
        SettingsRow(
          label: 'Keep for',
          divider: false,
          trailing: ChoiceChips<int>(
            options: [
              for (final d in _kRetentionPresets)
                ChoiceOption(d, d == 1 ? '1 day' : '$d days'),
            ],
            selected: prefs.retentionDays,
            onSelected: (v) => _apply({'retention_days': v}),
          ),
        ),
        SettingsButtonRow(
          child: StormButton.danger(
            key: const Key('notify-clear-inbox'),
            label: 'Clear inbox',
            onPressed: () async {
              final ok = await confirmSetting(
                context,
                title: 'Clear the inbox?',
                body:
                    'Every notice is removed for every device. This '
                    'cannot be undone.',
                action: 'Clear',
                confirmKey: const Key('notify-clear-confirm'),
              );
              if (!ok || !context.mounted) return;
              try {
                await state.clear();
              } catch (e) {
                if (context.mounted) settingsToast(context, describeFailure(e));
              }
            },
          ),
        ),

        GroupLabel('Send a test notification', top: t.sp * 3),
        SettingsButtonRow(
          child: StormButton.outline(
            key: const Key('notify-send-test'),
            label: 'Send test',
            onPressed: () async {
              try {
                await state.sendTest();
                if (context.mounted) {
                  settingsToast(context, 'Sent — check this device.');
                }
              } catch (e) {
                if (context.mounted) settingsToast(context, describeFailure(e));
              }
            },
          ),
        ),
      ],
    );
  }
}

class _EventRow extends StatelessWidget {
  const _EventRow({
    required this.kind,
    required this.pref,
    required this.onToggle,
    this.onThreshold,
  });

  final String kind;
  final EventPref? pref;
  final ValueChanged<bool> onToggle;
  final ValueChanged<int>? onThreshold;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final on = pref?.on ?? defaultEventOn(kind);
    final threshold = pref?.thresholdS;
    return Padding(
      padding: EdgeInsets.only(bottom: onThreshold == null ? 0 : t.sp),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          SettingsRow(
            label: _kKindLabels[kind] ?? kind,
            divider: onThreshold == null,
            trailing: StormToggle(
              key: Key('notify-event-$kind'),
              value: on,
              onChanged: onToggle,
            ),
          ),
          if (onThreshold != null && on) ...[
            SizedBox(height: t.sp * 0.75),
            ChoiceChips<int>(
              key: Key('notify-event-$kind-threshold'),
              options: [
                for (final s in _kThresholdPresets)
                  ChoiceOption(s, _thresholdLabel(s)),
              ],
              selected: threshold,
              onSelected: onThreshold,
            ),
            SizedBox(height: t.sp),
            Container(height: t.bw, color: t.border),
          ],
        ],
      ),
    );
  }
}

class _TimeField extends StatelessWidget {
  const _TimeField({super.key, required this.value, required this.onChanged});

  final String value;
  final ValueChanged<String> onChanged;

  @override
  Widget build(BuildContext context) {
    final parts = value.split(':');
    final initial = TimeOfDay(
      hour: parts.isNotEmpty ? int.tryParse(parts[0]) ?? 0 : 0,
      minute: parts.length > 1 ? int.tryParse(parts[1]) ?? 0 : 0,
    );
    return TextAction(
      label: value,
      onTap: () async {
        final picked = await showTimePicker(
          context: context,
          initialTime: initial,
        );
        if (picked == null) return;
        final hh = picked.hour.toString().padLeft(2, '0');
        final mm = picked.minute.toString().padLeft(2, '0');
        onChanged('$hh:$mm');
      },
    );
  }
}

class _MutesList extends ConsumerWidget {
  const _MutesList({required this.state});

  final NotificationsState state;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final t = context.tokens;
    if (state.mutes.isEmpty) return const SizedBox.shrink();
    final overview = ref.watch(agentOverviewProvider).value;
    String untilWords(String? until, {required String whenForever}) {
      if (until == null) return whenForever;
      final parsed = DateTime.tryParse(until);
      return parsed == null
          ? 'until it expires'
          : 'until ${friendlyUntil(parsed)}';
    }

    String label(NotificationMute m) {
      if (m.scope == 'provider') {
        final until = untilWords(m.until, whenForever: 'always');
        return 'All ${providerLabel(m.target)} sessions · $until';
      }
      final name = overview?.byId(m.target)?.displayName ?? m.target;
      final until = untilWords(m.until, whenForever: 'until the session ends');
      return '$name · $until';
    }

    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        for (final m in state.mutes)
          SettingsRow(
            key: Key('notify-mute-${m.id}'),
            label: label(m),
            trailing: TextAction(
              key: Key('notify-unmute-${m.id}'),
              label: 'Unmute',
              onTap: () => _guard(context, () => state.deleteMute(m.id)),
            ),
          ),
        SizedBox(height: t.sp * 0.5),
      ],
    );
  }
}

class _PauseControl extends StatelessWidget {
  const _PauseControl({required this.state, required this.prefs});

  final NotificationsState state;
  final NotificationPrefs prefs;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final pausedUntil = prefs.pausedUntil;
    final parsed = pausedUntil == null ? null : DateTime.tryParse(pausedUntil);
    // "Paused" means an end strictly in the future, not merely a parsable
    // one: a past `paused_until` the server has not yet cleared is not
    // still paused (item 7 of the review).
    final paused = parsed != null && parsed.isAfter(DateTime.now());
    if (paused) {
      return SettingsRow(
        label: 'Paused until ${friendlyUntil(parsed)}',
        divider: false,
        trailing: TextAction(
          key: const Key('notify-resume'),
          label: 'Resume now',
          onTap: () => _guard(context, state.unpause),
        ),
      );
    }
    return SettingsRow(
      label: 'Pause',
      divider: false,
      trailing: Wrap(
        spacing: t.sp,
        children: [
          TextAction(
            key: const Key('notify-pause-1h'),
            label: '1 hour',
            onTap: () => _guard(
              context,
              () => state.pause(
                until: DateTime.now()
                    .toUtc()
                    .add(const Duration(hours: 1))
                    .toIso8601String(),
              ),
            ),
          ),
          TextAction(
            key: const Key('notify-pause-tomorrow'),
            label: 'Until tomorrow',
            onTap: () {
              final local = DateTime.now();
              final tomorrow = DateTime(
                local.year,
                local.month,
                local.day + 1,
                8,
              );
              unawaited(
                _guard(
                  context,
                  () => state.pause(until: tomorrow.toUtc().toIso8601String()),
                ),
              );
            },
          ),
          TextAction(
            key: const Key('notify-pause-indefinite'),
            label: 'Until I turn them back on',
            onTap: () => _guard(context, () => state.pause()),
          ),
        ],
      ),
    );
  }
}
