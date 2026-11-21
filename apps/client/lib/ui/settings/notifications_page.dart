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
import 'package:url_launcher/url_launcher.dart';

import '../../agent/agent_api.dart' show AgentRefusal;
import '../../agent/agent_models.dart' show providerLabel;
import '../../agent/agent_state.dart' show agentApi, agentOverviewProvider;
import '../../agent/agent_widgets.dart' show clockAt;
import '../../agent/desktop_notifications.dart';
import '../../agent/desktop_platform.dart';
import '../../agent/device_notification_prefs_state.dart';
import '../../agent/notifications_models.dart';
import '../../agent/notifications_state.dart';
import '../../agent/ntfy_topic.dart';
import '../../agent/this_device_local_prefs.dart';
import '../../agent/unified_push.dart';
import '../../agent/web_notifications.dart';
import '../clipboard_copy.dart';
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

  /// 161-9: "This device" on a phone links here ("set up ntfy below")
  /// rather than duplicating the section.
  final _ntfySectionKey = GlobalKey();

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
        _ThisDeviceSection(ntfySectionKey: _ntfySectionKey),
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

        _NtfySection(
          key: _ntfySectionKey,
          state: state,
          prefs: prefs,
          onApply: _apply,
        ),

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

/// "ntfy (optional)" (decision 161, slice 161-9): an account-wide sink,
/// independent of any one device — the phone may be closed. Its own
/// preview setting defaults to "Session name only" and never carries agent
/// text unless set to "Full text" (with the warning below), because
/// ntfy.sh stores everything it forwards as plain text.
class _NtfySection extends StatefulWidget {
  const _NtfySection({
    super.key,
    required this.state,
    required this.prefs,
    required this.onApply,
  });

  final NotificationsState state;
  final NotificationPrefs prefs;
  final Future<void> Function(Map<String, dynamic> patch) onApply;

  @override
  State<_NtfySection> createState() => _NtfySectionState();
}

class _NtfySectionState extends State<_NtfySection> {
  final _url = TextEditingController();
  final _token = TextEditingController();
  String? _urlEditedFor;
  bool _testing = false;
  NtfyTestResult? _testResult;

  @override
  void dispose() {
    _url.dispose();
    _token.dispose();
    super.dispose();
  }

  // Neither method below touches `_urlEditedFor`: that field means "the
  // server value this controller was last synced *from*" (build()'s own
  // guard), not "what I just asked to save". Setting it here would make
  // the very next build see a mismatch against the still-stale `prefs`
  // and immediately overwrite the field back to the old value, before
  // the PUT in flight has had any chance to land.

  Future<void> _generate() async {
    final url = randomNtfyUrl();
    setState(() => _url.text = url);
    await widget.onApply({'ntfy_url': url});
  }

  Future<void> _submitUrl(String value) async {
    await widget.onApply({'ntfy_url': value.trim()});
  }

  Future<void> _submitToken(String value) async {
    await widget.onApply({'ntfy_token': value});
    // Never kept in the field, or re-shown, once sent: the hint
    // ("Set") is what confirms it landed.
    _token.clear();
  }

  Future<void> _copyUrl() async {
    final ok = await copyToClipboard(_url.text);
    if (!mounted) return;
    settingsToast(context, ok ? 'Copied.' : 'Could not copy.');
  }

  Future<void> _test() async {
    setState(() {
      _testing = true;
      _testResult = null;
    });
    try {
      final result = await widget.state.sendTestNtfy();
      if (!mounted) return;
      setState(() {
        _testing = false;
        _testResult = result;
      });
    } catch (e) {
      if (!mounted) return;
      setState(() {
        _testing = false;
        _testResult = NtfyTestResult(sent: false, error: describeFailure(e));
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final prefs = widget.prefs;
    if (_urlEditedFor != prefs.ntfyUrl) {
      _url.text = prefs.ntfyUrl ?? '';
      _urlEditedFor = prefs.ntfyUrl;
    }
    return Padding(
      padding: EdgeInsets.only(bottom: t.sp * 1.5),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          const GroupLabel('ntfy (optional)', bottom: 10),
          Text(
            'Install the ntfy app (Android/iPhone), add this topic — Storm '
            'publishes there independent of whether this device is open.',
            style: TextStyle(fontSize: t.labelSize, color: t.text3),
          ),
          SizedBox(height: t.sp),
          SettingsRow(
            label: 'Topic URL',
            sub:
                'ntfy.sh stores this as plain text; self-host your own '
                'server for more privacy.',
            trailing: SizedBox(
              width: t.sp * 22,
              child: StormInput(
                key: const Key('ntfy-url'),
                controller: _url,
                hintText: 'https://ntfy.sh/storm-…',
                onSubmitted: (v) => unawaited(_submitUrl(v)),
                suffix: IconButton(
                  key: const Key('ntfy-copy'),
                  tooltip: 'Copy',
                  icon: Icon(Icons.copy, size: t.bodySize),
                  onPressed: _url.text.isEmpty
                      ? null
                      : () => unawaited(_copyUrl()),
                ),
              ),
            ),
          ),
          SettingsButtonRow(
            child: StormButton.outline(
              key: const Key('ntfy-generate'),
              label: 'Generate private topic',
              onPressed: () => unawaited(_generate()),
            ),
          ),
          SettingsRow(
            label: 'Token',
            sub: 'Optional — only if your topic needs one.',
            trailing: SizedBox(
              width: t.sp * 16,
              child: StormInput(
                key: const Key('ntfy-token'),
                controller: _token,
                obscureText: true,
                hintText: prefs.ntfyTokenSet ? 'Set' : 'Bearer token',
                onSubmitted: (v) => unawaited(_submitToken(v)),
              ),
            ),
          ),
          SizedBox(height: t.sp * 0.75),
          SettingsRow(
            label: 'Show',
            divider: false,
            trailing: ChoiceChips<String>(
              key: const Key('ntfy-preview'),
              options: [
                for (final level in kDevicePreviewLevels)
                  ChoiceOption(level, _kPreviewLabels[level] ?? level),
              ],
              selected: prefs.ntfyPreview,
              onSelected: (v) => unawaited(widget.onApply({'ntfy_preview': v})),
            ),
          ),
          if (prefs.ntfyPreview == 'full')
            Padding(
              padding: EdgeInsets.only(top: t.sp * 0.5),
              child: Text(
                'Full text sends an agent\'s own words to ntfy as plain '
                'text.',
                style: TextStyle(fontSize: t.labelSize, color: t.amber),
              ),
            ),
          SizedBox(height: t.sp * 1.5),
          SettingsButtonRow(
            child: StormButton.outline(
              key: const Key('ntfy-test'),
              label: _testing ? 'Testing…' : 'Test',
              onPressed: !prefs.ntfyEnabled || _testing
                  ? null
                  : () => unawaited(_test()),
            ),
          ),
          if (_testResult != null)
            Padding(
              padding: EdgeInsets.only(top: t.sp * 0.5),
              child: Text(
                _testResult!.sent
                    ? 'Sent — check your phone.'
                    : 'Failed: ${_testResult!.error ?? 'unknown error'}',
                key: const Key('ntfy-test-result'),
                style: TextStyle(
                  fontSize: t.labelSize,
                  color: _testResult!.sent ? t.text3 : t.danger,
                ),
              ),
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
  const _ThisDeviceSection({required this.ntfySectionKey});

  /// 161-9: the mobile branch's "set up ntfy below" scrolls here.
  final GlobalKey ntfySectionKey;

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
    // Web notifications round: its own section, not this one — the
    // secure-context gate, the "Enable notifications" button and its
    // three-way permission state have no desktop equivalent, and forcing
    // them through this widget's desktop-flavoured labels (`defaultTargetPlatform`
    // is the host OS even under `kIsWeb`, never a "web" value of its own)
    // would read wrong on at least one of the two.
    if (kIsWeb) return const _ThisDeviceWebSection();
    // 161-8, building on 161-9's own mobile section rather than replacing
    // it: Android gets the real thing (UnifiedPush, device-specific,
    // end-to-end encrypted), aware of the account-wide ntfy sink as a
    // fallback/complement; iOS has neither OS path yet, so it keeps
    // 161-9's plain "set up ntfy" section unchanged.
    if (isAndroidPlatform) {
      return _ThisDeviceAndroidSection(ntfySectionKey: widget.ntfySectionKey);
    }
    if (!isDesktopPlatform) {
      return _ThisDeviceMobileSection(ntfySectionKey: widget.ntfySectionKey);
    }
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

/// Android and iOS (decision 161, slice 161-9): honest about what this
/// slice actually does. The operator's own report was the reason for
/// this section existing at all — Android showed notifications as "on"
/// while nothing ever arrived outside the open app, because the OS was
/// never asked and nothing here raises a system notification. Rather
/// than imply OS delivery (or show nothing, as before), this says
/// plainly that delivery is in-app only, and points at ntfy, which does
/// reach a closed phone.
class _ThisDeviceMobileSection extends ConsumerWidget {
  const _ThisDeviceMobileSection({required this.ntfySectionKey});

  final GlobalKey ntfySectionKey;

  void _scrollToNtfy(BuildContext context) {
    final target = ntfySectionKey.currentContext;
    if (target != null) {
      unawaited(
        Scrollable.ensureVisible(
          target,
          duration: const Duration(milliseconds: 300),
        ),
      );
    }
  }

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final t = context.tokens;
    final prefs = ref.watch(notificationsStateProvider)?.prefs;
    final viaNtfy = prefs?.ntfyEnabled ?? false;
    return Padding(
      padding: EdgeInsets.only(bottom: t.sp * 1.5),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          const GroupLabel('This device', bottom: 10),
          SettingsRow(
            key: const Key('device-delivery-mobile'),
            label: 'Delivery',
            sub: viaNtfy
                ? 'Via ntfy (account-wide)'
                : 'Only inside the app while it\'s open. For '
                      'notifications when Storm is closed, set up ntfy '
                      'below.',
            divider: false,
            trailing: viaNtfy
                ? null
                : TextAction(
                    key: const Key('device-set-up-ntfy'),
                    label: 'Set up ntfy',
                    onTap: () => _scrollToNtfy(context),
                  ),
          ),
          SizedBox(height: t.sp * 3),
          const SettingsDivider(),
          SizedBox(height: t.sp * 1.5),
        ],
      ),
    );
  }
}

/// Web notifications round: the browser `Notification` API, raised from
/// the page itself while a tab is open (never Web Push, which stays
/// 161-6/7). Its own section rather than a branch of
/// [_ThisDeviceSectionState] above — the secure-context gate and the
/// "Enable notifications" button's own gesture requirement have no
/// desktop equivalent.
class _ThisDeviceWebSection extends ConsumerStatefulWidget {
  const _ThisDeviceWebSection();

  @override
  ConsumerState<_ThisDeviceWebSection> createState() =>
      _ThisDeviceWebSectionState();
}

class _ThisDeviceWebSectionState extends ConsumerState<_ThisDeviceWebSection> {
  WebNotificationPermission _permission = WebNotificationPermission.notAsked;
  bool _enabling = false;

  @override
  void initState() {
    super.initState();
    _permission = webNotificationPermission();
  }

  Future<void> _enable(DeviceNotificationPrefsState deviceState) async {
    setState(() => _enabling = true);
    try {
      final result = await requestWebNotificationPermission();
      if (!mounted) return;
      setState(() {
        _permission = result;
        _enabling = false;
      });
      if (result == WebNotificationPermission.allowed) {
        await _guard(context, () => deviceState.setEnabled(true));
      }
    } catch (e) {
      if (!mounted) return;
      setState(() => _enabling = false);
      settingsToast(context, describeFailure(e));
    }
  }

  String get _permissionLabel => switch (_permission) {
    WebNotificationPermission.insecureOrigin => 'Notifications need HTTPS',
    WebNotificationPermission.notAsked => 'Not asked yet',
    WebNotificationPermission.denied => 'Blocked in browser settings',
    WebNotificationPermission.allowed => 'Allowed',
  };

  @override
  Widget build(BuildContext context) {
    final deviceState = ref.watch(deviceNotificationPrefsProvider);
    final local = ref.watch(thisDeviceLocalPrefsProvider);
    final notifier = ref.watch(webNotifierProvider);
    if (deviceState == null) return const SizedBox.shrink();
    return ListenableBuilder(
      listenable: Listenable.merge([deviceState, local, ?notifier]),
      builder: (context, _) => _body(context, deviceState, local),
    );
  }

  Widget _body(
    BuildContext context,
    DeviceNotificationPrefsState deviceState,
    ThisDeviceLocalPrefs local,
  ) {
    final t = context.tokens;
    if (_permission == WebNotificationPermission.insecureOrigin) {
      return Padding(
        padding: EdgeInsets.only(bottom: t.sp * 1.5),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            const GroupLabel('This device', bottom: 10),
            SettingsRow(
              label: 'Notifications',
              sub:
                  '$_permissionLabel — open Storm via the relay, or '
                  'another HTTPS address, to turn them on.',
            ),
            SizedBox(height: t.sp * 3),
            const SettingsDivider(),
            SizedBox(height: t.sp * 1.5),
          ],
        ),
      );
    }
    final prefs = deviceState.prefs;
    final ready =
        _permission == WebNotificationPermission.allowed && prefs.enabled;
    return Padding(
      padding: EdgeInsets.only(bottom: t.sp * 1.5),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          const GroupLabel('This device', bottom: 10),
          if (!ready)
            SettingsRow(
              label: 'Notifications on this device',
              sub: _permissionLabel,
              trailing: _permission == WebNotificationPermission.denied
                  ? null
                  : StormButton.outline(
                      key: const Key('web-enable-notifications'),
                      label: _enabling ? 'Enabling…' : 'Enable notifications',
                      onPressed: _enabling
                          ? null
                          : () => unawaited(_enable(deviceState)),
                    ),
            )
          else ...[
            SettingsRow(
              label: 'Notifications on this device',
              sub: _permissionLabel,
              trailing: StormToggle(
                key: const Key('device-notify-enabled'),
                value: prefs.enabled,
                onChanged: (v) =>
                    _guard(context, () => deviceState.setEnabled(v)),
              ),
            ),
            SettingsRow(label: 'Delivery', sub: 'Only while this tab is open'),
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
                sub: prefs.classOn(kind) && _onWebClassCount(prefs) <= 1
                    ? 'Use "Notifications on this device" to turn off the last one'
                    : null,
                divider: kind != kDeviceNotificationClasses.last,
                trailing: StormToggle(
                  key: Key('device-class-$kind'),
                  value: prefs.classOn(kind),
                  enabled:
                      !(prefs.classOn(kind) && _onWebClassCount(prefs) <= 1),
                  onChanged: (v) => _guard(
                    context,
                    () => deviceState.setClasses(_withWebClass(prefs, kind, v)),
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
            SizedBox(height: t.sp * 1.5),
            SettingsButtonRow(
              child: StormButton.outline(
                key: const Key('device-send-test'),
                label: 'Send a test notification',
                onPressed: () async {
                  try {
                    final n = await deviceState.sendTest();
                    unawaited(ref.read(webNotifierProvider)?.forceShow(n));
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
          ],
          SizedBox(height: t.sp * 3),
          const SettingsDivider(),
          SizedBox(height: t.sp * 1.5),
        ],
      ),
    );
  }

  int _onWebClassCount(DeviceNotificationPrefs prefs) => prefs.classes.isEmpty
      ? kDeviceNotificationClasses.length
      : kDeviceNotificationClasses.where(prefs.classOn).length;

  List<String> _withWebClass(
    DeviceNotificationPrefs prefs,
    String kind,
    bool on,
  ) {
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

/// Android closed-app push (decision 161, slice 161-8): UnifiedPush, with
/// ntfy as the distributor Storm tells people to install. Builds on
/// [_ThisDeviceMobileSection]'s own delivery row rather than replacing it
/// (161-9 shipped first): the account-wide ntfy sink below still reaches
/// this device with no setup here at all, so the Delivery row and this
/// section's own "Set up ntfy" fallback both still apply when UnifiedPush
/// itself isn't on. Its own section rather than a branch of
/// [_ThisDeviceSectionState] — the distributor pick, the install-ntfy
/// state and the permission flow have no desktop or web equivalent, same
/// reasoning as [_ThisDeviceWebSection].
class _ThisDeviceAndroidSection extends ConsumerStatefulWidget {
  const _ThisDeviceAndroidSection({required this.ntfySectionKey});

  final GlobalKey ntfySectionKey;

  @override
  ConsumerState<_ThisDeviceAndroidSection> createState() =>
      _ThisDeviceAndroidSectionState();
}

class _ThisDeviceAndroidSectionState
    extends ConsumerState<_ThisDeviceAndroidSection> {
  void _scrollToNtfy(BuildContext context) {
    final target = widget.ntfySectionKey.currentContext;
    if (target != null) {
      unawaited(
        Scrollable.ensureVisible(
          target,
          duration: const Duration(milliseconds: 300),
        ),
      );
    }
  }

  @override
  Widget build(BuildContext context) {
    final deviceState = ref.watch(deviceNotificationPrefsProvider);
    final local = ref.watch(thisDeviceLocalPrefsProvider);
    final push = ref.watch(unifiedPushControllerProvider);
    final viaNtfy =
        ref.watch(notificationsStateProvider)?.prefs?.ntfyEnabled ?? false;
    if (deviceState == null) return const SizedBox.shrink();
    return ListenableBuilder(
      listenable: Listenable.merge([deviceState, local, ?push]),
      builder: (context, _) =>
          _body(context, deviceState, local, push, viaNtfy),
    );
  }

  /// "Build on its Android delivery row": a working UnifiedPush
  /// registration wins (it's this device's own, richer and encrypted to
  /// it alone); short of that, the account-wide ntfy sink already
  /// reaching this device is real delivery too, same wording
  /// [_ThisDeviceMobileSection] uses for it.
  String _deliveryLabel(
    DeviceNotificationPrefs prefs,
    UnifiedPushController? push,
    bool viaNtfy,
  ) {
    final pushOn = push?.status == UnifiedPushStatus.on;
    if (pushOn) {
      final via = 'Push via ${push?.distributorLabel ?? "a distributor"}';
      final health = prefs.push;
      if (health == null || health.failures == 0) return '$via · working';
      return health.lastOk == null
          ? '$via · never delivered (${health.failures} failed)'
          : '$via · ${health.failures} failed since its last delivery';
    }
    if (viaNtfy) return 'Via ntfy (account-wide)';
    return 'Only inside the app while it\'s open';
  }

  Future<void> _install(String url) async {
    unawaited(launchUrl(Uri.parse(url), mode: LaunchMode.externalApplication));
  }

  Widget _body(
    BuildContext context,
    DeviceNotificationPrefsState deviceState,
    ThisDeviceLocalPrefs local,
    UnifiedPushController? push,
    bool viaNtfy,
  ) {
    final t = context.tokens;
    final prefs = deviceState.prefs;
    final status = push?.status ?? UnifiedPushStatus.unavailable;
    final on = status == UnifiedPushStatus.on;
    return Padding(
      padding: EdgeInsets.only(bottom: t.sp * 1.5),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          const GroupLabel('This device', bottom: 10),
          SettingsRow(
            key: const Key('device-notify-enabled'),
            label: 'Notifications on this device',
            sub: status == UnifiedPushStatus.enabling ? 'Enabling…' : null,
            trailing: on
                ? StormToggle(
                    value: true,
                    onChanged: (v) {
                      if (!v) unawaited(push?.disable());
                    },
                  )
                : StormButton.outline(
                    key: const Key('android-enable-notifications'),
                    label: status == UnifiedPushStatus.enabling
                        ? 'Enabling…'
                        : 'Enable notifications',
                    onPressed: status == UnifiedPushStatus.enabling
                        ? null
                        : () => unawaited(push?.enable()),
                  ),
          ),
          SettingsRow(
            // Same key 161-9's own mobile delivery row uses
            // (`_ThisDeviceMobileSection`) — this row fills the identical
            // role for Android, just with a real push state to report
            // once one exists, so a test or a person looking for "the
            // delivery row" finds the same name either way.
            key: const Key('device-delivery-mobile'),
            label: 'Delivery',
            sub: _deliveryLabel(prefs, push, viaNtfy),
            trailing: on || viaNtfy
                ? null
                : TextAction(
                    key: const Key('device-set-up-ntfy'),
                    label: 'Set up ntfy',
                    onTap: () => _scrollToNtfy(context),
                  ),
          ),
          if (status == UnifiedPushStatus.distributorMissing)
            SettingsRow(
              label: 'No push app installed',
              sub:
                  'Install ntfy to get notifications when Storm is closed. '
                  'Open it once after installing, then try Enable again.',
              trailing: Wrap(
                spacing: t.sp,
                children: [
                  TextAction(
                    key: const Key('android-install-ntfy-play'),
                    label: 'Play Store',
                    onTap: () => unawaited(
                      _install(
                        'https://play.google.com/store/apps/details?id=io.heckel.ntfy',
                      ),
                    ),
                  ),
                  TextAction(
                    key: const Key('android-install-ntfy-fdroid'),
                    label: 'F-Droid',
                    onTap: () => unawaited(
                      _install('https://f-droid.org/packages/io.heckel.ntfy/'),
                    ),
                  ),
                ],
              ),
            ),
          if (status == UnifiedPushStatus.permissionDenied)
            SettingsRow(
              label: 'Notifications are blocked',
              sub: 'Blocked in Android settings.',
              trailing: TextAction(
                key: const Key('android-open-system-settings'),
                label: 'Open system settings',
                onTap: () => unawaited(push?.openSystemSettings()),
              ),
            ),
          if (status == UnifiedPushStatus.error)
            SettingsRow(
              label: 'Could not enable notifications',
              sub: push?.errorMessage ?? 'Something went wrong — try again.',
            ),
          if (on) ...[
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
                sub: prefs.classOn(kind) && _onAndroidClassCount(prefs) <= 1
                    ? 'Use "Notifications on this device" to turn off the last one'
                    : null,
                divider: kind != kDeviceNotificationClasses.last,
                trailing: StormToggle(
                  key: Key('device-class-$kind'),
                  value: prefs.classOn(kind),
                  enabled:
                      !(prefs.classOn(kind) &&
                          _onAndroidClassCount(prefs) <= 1),
                  onChanged: (v) => _guard(
                    context,
                    () => deviceState.setClasses(
                      _withAndroidClass(prefs, kind, v),
                    ),
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
            SizedBox(height: t.sp * 1.5),
            SettingsButtonRow(
              child: StormButton.outline(
                key: const Key('device-send-test'),
                label: 'Send a test notification',
                onPressed: () async {
                  try {
                    final n = await deviceState.sendTest();
                    unawaited(push?.forceShow(n, deviceState.prefs));
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
          ],
          SizedBox(height: t.sp * 3),
          const SettingsDivider(),
          SizedBox(height: t.sp * 1.5),
        ],
      ),
    );
  }

  int _onAndroidClassCount(DeviceNotificationPrefs prefs) =>
      prefs.classes.isEmpty
      ? kDeviceNotificationClasses.length
      : kDeviceNotificationClasses.where(prefs.classOn).length;

  List<String> _withAndroidClass(
    DeviceNotificationPrefs prefs,
    String kind,
    bool on,
  ) {
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
