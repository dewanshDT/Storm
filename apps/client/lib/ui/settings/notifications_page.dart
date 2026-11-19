/// Settings › Notifications (decisions 161-3 and 161-4): the account-level
/// sections, and "This device" at the top.
///
/// Two different stores, deliberately: anything that changes what the
/// server *sends* ([DeviceNotificationPrefsState], over
/// `/v1/notifications/device`) is on the server, because a closed app
/// cannot filter a push it never received; anything that only changes how
/// *this* device renders what arrives ([ThisDeviceLocalPrefs] — sound,
/// macOS's keep-running toggle) stays local.
library;

import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../agent/agent_api.dart' show AgentRefusal;
import '../../agent/agent_models.dart' show providerLabel;
import '../../agent/agent_state.dart' show agentApi, agentOverviewProvider;
import '../../agent/agent_widgets.dart' show clockAt;
import '../../agent/desktop_notifications.dart';
import '../../agent/desktop_platform.dart';
import '../../agent/device_notification_prefs_state.dart';
import '../../agent/notifications_models.dart';
import '../../agent/notifications_state.dart';
import '../../agent/this_device_local_prefs.dart';
import '../controls.dart';
import '../states.dart' show describeFailure;
import '../tokens.dart';
import '../widgets.dart' show StormInput;
import 'settings_widgets.dart';

const _kDeviceClassLabels = {
  'needs_you': 'Needs you',
  'delete_requests': 'Delete requests',
  'finished': 'Finished',
  'errors': 'Errors',
  'agent_messages': 'Agent messages',
  'hosts': 'Hosts',
};

const _kPreviewLabels = {
  'full': 'Full text',
  'session': 'Session name only',
  'storm': '"Storm" only',
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

/// "14:02", or "3/4 14:02" for a moment not today; "you turn them back on"
/// for [NotificationPrefs.pausedUntil]'s far-future sentinel
/// (`PAUSE_INDEFINITELY`).
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
        const _ThisDeviceSection(),
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
            label: kNotificationKindLabels[kind] ?? kind,
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
            const SettingsDivider(),
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

class _PauseControl extends StatefulWidget {
  const _PauseControl({required this.state, required this.prefs});

  final NotificationsState state;
  final NotificationPrefs prefs;

  @override
  State<_PauseControl> createState() => _PauseControlState();
}

class _PauseControlState extends State<_PauseControl> {
  Timer? _expiry;
  DateTime? _scheduledFor;

  /// Nothing pushes a change when a pause simply elapses (item N8 of the
  /// second review) — a timer to its own end is what turns this row back
  /// into "Pause" without waiting for some unrelated rebuild.
  void _scheduleExpiry(DateTime? until) {
    if (_scheduledFor == until) return;
    _expiry?.cancel();
    _scheduledFor = until;
    if (until == null) return;
    final delay = until.difference(DateTime.now());
    if (delay.isNegative) return;
    _expiry = Timer(delay, () {
      if (mounted) setState(() {});
    });
  }

  @override
  void dispose() {
    _expiry?.cancel();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final state = widget.state;
    final prefs = widget.prefs;
    final pausedUntil = prefs.pausedUntil;
    final parsed = pausedUntil == null ? null : DateTime.tryParse(pausedUntil);
    _scheduleExpiry(parsed);
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

/// "This device" (decision 161, slice 161-4): nothing on phones or web,
/// where there is no OS notification to configure and no background
/// stream to keep open.
class _ThisDeviceSection extends ConsumerStatefulWidget {
  const _ThisDeviceSection();

  @override
  ConsumerState<_ThisDeviceSection> createState() => _ThisDeviceSectionState();
}

class _ThisDeviceSectionState extends ConsumerState<_ThisDeviceSection> {
  DesktopPermissionStatus? _permission;

  @override
  void initState() {
    super.initState();
    if (isDesktopPlatform) unawaited(_checkPermission());
  }

  Future<void> _checkPermission() async {
    final notifier = ref.read(desktopNotifierProvider);
    final status = await notifier?.permissionStatus();
    if (!mounted) return;
    setState(() => _permission = status);
  }

  String get _permissionLabel => switch (_permission) {
    null => 'Checking…',
    DesktopPermissionStatus.unavailable => 'No notification service',
    DesktopPermissionStatus.notAsked => 'Not asked yet',
    DesktopPermissionStatus.denied => 'Blocked in system settings',
    DesktopPermissionStatus.allowed ||
    DesktopPermissionStatus.notApplicable => 'Allowed',
  };

  // Review item G: once a show has actually failed, the delivery row says
  // so too — not just the OS permission row below it, which only answers
  // once `_checkPermission` has run.
  String _deliveryStatus(DesktopNotifier? notifier) {
    if (notifier?.unavailable ?? false) return 'No notification service';
    return switch (defaultTargetPlatform) {
      TargetPlatform.macOS => 'While Storm runs',
      _ => 'Only while Storm is open',
    };
  }

  @override
  Widget build(BuildContext context) {
    if (!isDesktopPlatform) return const SizedBox.shrink();
    final deviceState = ref.watch(deviceNotificationPrefsProvider);
    final local = ref.watch(thisDeviceLocalPrefsProvider);
    final notifier = ref.watch(desktopNotifierProvider);
    if (deviceState == null) return const SizedBox.shrink();
    return ListenableBuilder(
      listenable: Listenable.merge([deviceState, local, ?notifier]),
      builder: (context, _) => _body(context, deviceState, local, notifier),
    );
  }

  Widget _body(
    BuildContext context,
    DeviceNotificationPrefsState deviceState,
    ThisDeviceLocalPrefs local,
    DesktopNotifier? notifier,
  ) {
    final t = context.tokens;
    // Review item 12: never "Loading…" — [DeviceNotificationPrefsState]
    // seeds [prefs] with this platform's own default the moment it is
    // constructed, so the section is usable immediately and only quietly
    // updates once the real row answers.
    final prefs = deviceState.prefs;
    return Padding(
      padding: EdgeInsets.only(bottom: t.sp * 1.5),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          const GroupLabel('This device', bottom: 10),
          SettingsRow(
            label: 'Notifications on this device',
            trailing: StormToggle(
              key: const Key('device-notify-enabled'),
              value: prefs.enabled,
              onChanged: (v) =>
                  _guard(context, () => deviceState.setEnabled(v)),
            ),
          ),
          SettingsRow(label: 'Delivery', sub: _deliveryStatus(notifier)),
          SettingsRow(
            label: 'OS permission',
            sub: _permissionLabel,
            trailing: _permission == DesktopPermissionStatus.denied
                ? TextAction(
                    key: const Key('device-open-system-settings'),
                    label: 'Open system settings',
                    onTap: () => unawaited(
                      ref
                          .read(desktopNotifierProvider)
                          ?.openSystemNotificationSettings(),
                    ),
                  )
                : null,
          ),
          SettingsRow(
            label: 'Show',
            divider: false,
            trailing: ChoiceChips<String>(
              key: const Key('device-preview'),
              options: [
                for (final level in kDevicePreviewLevels)
                  ChoiceOption(level, _kPreviewLabels[level] ?? level),
              ],
              selected: prefs.preview,
              onSelected: (v) =>
                  _guard(context, () => deviceState.setPreview(v)),
            ),
          ),
          SizedBox(height: t.sp * 1.5),
          GroupLabel('Send to this device', top: t.sp),
          for (final kind in kDeviceNotificationClasses)
            SettingsRow(
              label: _kDeviceClassLabels[kind] ?? kind,
              // The last class still on can't be turned off here (review
              // item 7): `classes: []` means *every* class, so removing
              // the only name left would turn every other class back on
              // instead of leaving none — the opposite of this toggle's
              // own meaning. The main switch above is the only way to
              // turn everything off.
              sub: prefs.classOn(kind) && _onClassCount(prefs) <= 1
                  ? 'Use "Notifications on this device" to turn off the last one'
                  : null,
              divider: kind != kDeviceNotificationClasses.last,
              trailing: StormToggle(
                key: Key('device-class-$kind'),
                value: prefs.classOn(kind),
                enabled: !(prefs.classOn(kind) && _onClassCount(prefs) <= 1),
                onChanged: (v) => _guard(
                  context,
                  () => deviceState.setClasses(_withClass(prefs, kind, v)),
                ),
              ),
            ),
          SizedBox(height: t.sp * 1.5),
          SettingsRow(
            label: 'Sound',
            divider: false,
            trailing: StormToggle(
              key: const Key('device-sound'),
              value: local.soundOn,
              onChanged: (v) => local.setSoundOn(v),
            ),
          ),
          if (defaultTargetPlatform == TargetPlatform.macOS)
            SettingsRow(
              label: 'Keep Storm running when its window is closed',
              divider: false,
              trailing: StormToggle(
                key: const Key('device-keep-running'),
                value: local.keepRunningWhenClosed,
                onChanged: (v) => local.setKeepRunningWhenClosed(v),
              ),
            ),
          SizedBox(height: t.sp * 1.5),
          SettingsButtonRow(
            child: StormButton.outline(
              key: const Key('device-send-test'),
              label: 'Send a test notification',
              onPressed: () async {
                try {
                  final n = await deviceState.sendTest();
                  // Raises the OS notification itself, bypassing the
                  // deliver hint and presence (review item 13): the
                  // person is looking right at this page, so the normal
                  // rule would otherwise show nothing or only a toast,
                  // proving neither permission nor the OS path.
                  unawaited(ref.read(desktopNotifierProvider)?.forceShow(n));
                  if (context.mounted) {
                    settingsToast(context, 'Sent — check this device.');
                  }
                } catch (e) {
                  if (context.mounted) {
                    settingsToast(context, describeFailure(e));
                  }
                }
              },
            ),
          ),
          SizedBox(height: t.sp * 3),
          const SettingsDivider(),
          SizedBox(height: t.sp * 1.5),
        ],
      ),
    );
  }

  /// How many of [kDeviceNotificationClasses] are currently on — `[]`
  /// counts as every one of them.
  int _onClassCount(DeviceNotificationPrefs prefs) => prefs.classes.isEmpty
      ? kDeviceNotificationClasses.length
      : kDeviceNotificationClasses.where(prefs.classOn).length;

  /// [prefs.classes] with [kind] added or removed. `[]` ("every class")
  /// is expanded to the full explicit list first — there is no way to
  /// toggle one class off a set that names none, only off a set that
  /// names all of them.
  List<String> _withClass(DeviceNotificationPrefs prefs, String kind, bool on) {
    final current = prefs.classes.isEmpty
        ? List<String>.from(kDeviceNotificationClasses)
        : List<String>.from(prefs.classes);
    if (on) {
      if (!current.contains(kind)) current.add(kind);
    } else {
      current.remove(kind);
    }
    return current;
  }
}
