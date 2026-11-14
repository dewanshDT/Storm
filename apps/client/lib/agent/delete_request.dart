import 'dart:async';
import 'dart:math' as math;

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../router.dart';
import '../state/app_state.dart' show rootMessengerKey, vaultsProvider;
import '../ui/decision_strip.dart';
import '../ui/states.dart' show describeFailure;
import '../ui/surfaces.dart' show showStormSheet;
import '../ui/trash_screen.dart' show TrashRetention, trashRetentionProvider;
import 'agent_api.dart';
import 'agent_models.dart';
import 'agent_state.dart';
import 'sidebar_rules.dart' show agoWords;

/// The agent asking, as the strip names it.
class DeleteAsker {
  const DeleteAsker({
    required this.sessionId,
    required this.name,
    required this.provider,
    this.given,
    this.model,
    this.context,
  });

  final String sessionId;
  final String name;
  final String provider;

  /// The name its parent gave a child (145b).
  final String? given;
  final String? model;
  final SessionContext? context;

  factory DeleteAsker.fromJson(Map<String, dynamic> j) => DeleteAsker(
    sessionId: j['id'] as String? ?? '',
    name: (j['title'] ?? j['name'] ?? 'An agent') as String,
    provider: j['provider'] as String? ?? '',
    model: j['model'] as String?,
    context: j['context'] is Map<String, dynamic>
        ? SessionContext.fromJson(j['context'] as Map<String, dynamic>)
        : null,
  );

  /// Who the reason is from: a child by its given name, otherwise its
  /// agent, never the session's title, which its terminal sets.
  String get speaker =>
      given ?? (provider.isEmpty ? 'The agent' : providerLabel(provider));

  factory DeleteAsker.of(AgentSession s) => DeleteAsker(
    sessionId: s.id,
    name: s.displayName,
    provider: s.provider,
    given: s.spawn.isChild ? s.name : null,
    model: s.model,
    context: s.context,
  );
}

/// The routes of decision 149, owner only on the session tier.
extension DeleteApi on AgentApi {
  /// The deletions waiting, and those just answered; with [after], waits up
  /// to [wait] seconds for a change first.
  Future<(int, List<(DeleteRequest, DeleteAsker)>)> deleteRequests({
    int? after,
    int wait = 0,
  }) async {
    final json =
        await send(
              'GET',
              '/v1/agent/delete-requests',
              query: {
                if (after != null) 'after': '$after',
                if (after != null) 'wait': '$wait',
              },
            )
            as Map;
    return (
      (json['version'] as num?)?.toInt() ?? 0,
      [
        for (final r in json['requests'] as List? ?? const [])
          if (r is Map<String, dynamic>)
            (
              DeleteRequest.fromJson(r),
              DeleteAsker.fromJson(
                r['session'] is Map<String, dynamic>
                    ? r['session'] as Map<String, dynamic>
                    : const {},
              ),
            ),
      ],
    );
  }

  Future<DeleteRequest> decideDeleteRequest(
    String sessionId,
    String requestId, {
    required bool approve,
  }) async => DeleteRequest.fromJson(
    await send(
          'POST',
          '/v1/agent/sessions/${Uri.encodeComponent(sessionId)}'
              '/delete-requests/${Uri.encodeComponent(requestId)}',
          body: {'decision': approve ? 'approve' : 'decline'},
        )
        as Map<String, dynamic>,
  );
}

/// "Ideas.md", or "folder Projects/Old and its 12 notes".
String deleteItem(DeleteRequest r) {
  if (!r.isFolder) return r.path.split('/').last;
  return switch (r.notes) {
    0 => 'folder ${r.path}',
    1 => 'folder ${r.path} and its 1 note',
    final n => 'folder ${r.path} and its $n notes',
  };
}

/// "tidier wants to delete Ideas.md" (153g).
String deleteTitle(String session, DeleteRequest r) =>
    '$session wants to delete ${deleteItem(r)}';

/// The phone's one line: "Delete Ideas.md?" under a header that already
/// names the session.
String deleteShortTitle(DeleteRequest r) => 'Delete ${deleteItem(r)}?';

String _bytes(int n) {
  if (n < 1024) return '$n B';
  if (n < 1024 * 1024) return '${(n / 1024).toStringAsFixed(1)} KB';
  return '${(n / (1024 * 1024)).toStringAsFixed(1)} MB';
}

/// "Ideas/Ideas.md · 2.0 KB · modified 2h ago".
String deleteWhat(DeleteRequest r, {DateTime? now}) {
  final at = DateTime.tryParse(r.modified ?? '');
  return [
    r.path,
    if (r.isFolder) r.notes == 1 ? '1 note' : '${r.notes} notes',
    _bytes(r.size),
    if (at != null) 'modified ${_ago(at, now ?? DateTime.now())}',
  ].join(' · ');
}

String _ago(DateTime at, DateTime now) {
  final words = agoWords(at, now);
  return words == 'now' ? 'just now' : '$words ago';
}

/// "It moves to personal’s Trash and is kept for 30 days.": the vault's own
/// retention, never a constant (153g).
String deleteIfApproved(String? vault, TrashRetention keep) =>
    'It moves to ${vault ?? 'the vault'}’s Trash and is ${keep.keptFor}.';

/// The request a session's strip shows, with its place among what waits
/// (153b, oldest first). An older server lists no decisions: then it is
/// the one.
({DeleteRequest request, int index, int total})? deleteDecisionOf(
  AgentSession s,
) {
  final r = s.deleteRequest;
  if (s.ended || r == null || !r.pending) return null;
  final all = s.pendingDecisions;
  if (all.isEmpty) return (request: r, index: 1, total: 1);
  final i = all.indexWhere((d) => d.id == r.id);
  return (request: r, index: i < 0 ? 1 : i + 1, total: math.max(all.length, 1));
}

String needsYouLabel(int index, int total) => 'NEEDS YOU · $index OF $total';

/// Requests this device answered, so the watcher does not report them as
/// answered elsewhere.
final _answeredHere = <String>{};

/// Sends the person's answer (149c). Nothing else answers a request: Later
/// and Esc only put the strip aside on this device.
Future<void> answerDeleteRequest(
  ProviderContainer container,
  DeleteRequest r, {
  required bool approve,
  ScaffoldMessengerState? messenger,
}) async {
  final api = container.read(agentApiFactoryProvider)?.call();
  if (api == null) return;
  _answeredHere.add(r.id);
  try {
    await api.decideDeleteRequest(r.sessionId, r.id, approve: approve);
  } catch (e) {
    final code = e is AgentRefusal ? e.code : null;
    (messenger ?? rootMessengerKey.currentState)?.showSnackBar(
      SnackBar(
        content: Text(switch (code) {
          'request_timed_out' => 'That request timed out: nothing was deleted.',
          final c? when c.startsWith('request_') => 'It was already answered.',
          _ => describeFailure(e),
        }),
      ),
    );
  } finally {
    api.dispose();
    container.invalidate(sessionDetailProvider(r.sessionId));
    container.invalidate(agentOverviewProvider);
  }
}

/// A session's delete request as a decision strip (153g): who asks, the
/// reason word for word, what goes and where it goes. Later (and Esc) puts
/// it aside here; Decline and Move to Trash answer it.
class DeleteDecisionStrip extends ConsumerStatefulWidget {
  const DeleteDecisionStrip({
    super.key,
    required this.session,
    this.layout = DecisionLayout.strip,
    this.collapsed = false,
    this.onLater,
    this.onReview,
    this.onAnswered,
    this.escapeAllowed,
  });

  final AgentSession session;
  final DecisionLayout layout;

  /// Put aside with Later: one line with Review.
  final bool collapsed;
  final VoidCallback? onLater;
  final VoidCallback? onReview;
  final VoidCallback? onAnswered;
  final bool Function()? escapeAllowed;

  @override
  ConsumerState<DeleteDecisionStrip> createState() =>
      _DeleteDecisionStripState();
}

class _DeleteDecisionStripState extends ConsumerState<DeleteDecisionStrip> {
  bool _busy = false;

  Future<void> _answer(DeleteRequest r, {required bool approve}) async {
    setState(() => _busy = true);
    final c = ProviderScope.containerOf(context, listen: false);
    await answerDeleteRequest(
      c,
      r,
      approve: approve,
      messenger: ScaffoldMessenger.maybeOf(context),
    );
    if (mounted) setState(() => _busy = false);
    widget.onAnswered?.call();
  }

  @override
  Widget build(BuildContext context) {
    final s = widget.session;
    final d = deleteDecisionOf(s);
    if (d == null) return const SizedBox.shrink();
    final r = d.request;
    final label = needsYouLabel(d.index, d.total);
    final title = deleteTitle(s.displayName, r);
    if (widget.collapsed) {
      return DecisionStrip(
        key: const Key('delete-request-strip'),
        tone: DecisionTone.waiting,
        label: label,
        title: title,
        actions: [
          DecisionAction(
            key: const Key('delete-request-review'),
            label: 'Review',
            kind: DecisionActionKind.ghost,
            onPressed: widget.onReview,
          ),
        ],
      );
    }
    final vault = ref
        .watch(vaultsProvider)
        .value
        ?.where((v) => v.id == r.vaultId)
        .firstOrNull
        ?.name;
    final keep = ref.watch(trashRetentionProvider(r.vaultId));
    const shown = 3;
    final hidden = r.paths.length > shown ? r.notes - shown : 0;
    final rows = [
      DecisionRow(
        '${DeleteAsker.of(s).speaker} says',
        r.reason,
        key: const Key('delete-request-says'),
        quote: true,
      ),
      DecisionRow(
        'What',
        deleteWhat(r),
        key: const Key('delete-request-what'),
        more: r.isFolder
            ? [...r.paths.take(shown), if (hidden > 0) '…and $hidden more']
            : const [],
      ),
      DecisionRow(
        'If you approve',
        deleteIfApproved(vault, keep),
        key: const Key('delete-request-if-approved'),
      ),
    ];
    final later = DecisionAction(
      key: const Key('delete-request-later'),
      label: 'Later',
      kind: DecisionActionKind.text,
      onPressed: widget.onLater,
    );
    final decline = DecisionAction(
      key: const Key('delete-request-decline'),
      label: 'Decline',
      kind: DecisionActionKind.ghost,
      onPressed: _busy ? null : () => _answer(r, approve: false),
    );
    final approve = DecisionAction(
      key: const Key('delete-request-approve'),
      label: 'Move to Trash',
      kind: DecisionActionKind.primary,
      onPressed: _busy ? null : () => _answer(r, approve: true),
    );
    return DecisionStrip(
      key: const Key('delete-request-strip'),
      tone: DecisionTone.waiting,
      layout: widget.layout,
      label: label,
      title: title,
      rows: rows,
      onEscape: widget.onLater,
      escapeAllowed: widget.escapeAllowed,
      actions: widget.layout == DecisionLayout.sheet
          ? [approve, decline, later]
          : [later, decline, approve],
    );
  }
}

/// The phone's strip for a session's delete request (§7): one line under
/// the header, and Review raises the whole request in a sheet.
class PhoneDeleteStrip extends StatelessWidget {
  const PhoneDeleteStrip({super.key, required this.session});

  final AgentSession session;

  @override
  Widget build(BuildContext context) {
    final d = deleteDecisionOf(session);
    if (d == null) return const SizedBox.shrink();
    return CompactDecisionStrip(
      key: const Key('delete-request-compact'),
      label: 'NEEDS YOU',
      title: deleteShortTitle(d.request),
      onReview: () => showDeleteRequestSheet(context, session),
    );
  }
}

/// The request as a sheet, never a dialog (§7): Move to Trash, Decline and
/// Later full width. It closes itself once the request is answered here or
/// anywhere else.
Future<void> showDeleteRequestSheet(BuildContext context, AgentSession s) {
  final d = deleteDecisionOf(s);
  if (d == null) return Future.value();
  return showStormSheet<void>(
    context: context,
    title: needsYouLabel(d.index, d.total),
    heightFactor: 0.85,
    builder: (sheet) => _DeleteSheet(
      session: s,
      requestId: d.request.id,
      // Once only: an answer and the request leaving the view both close it.
      close: () {
        if (!sheet.mounted) return;
        final route = ModalRoute.of(sheet);
        if (route != null && route.isCurrent) Navigator.of(sheet).pop();
      },
    ),
  );
}

class _DeleteSheet extends ConsumerWidget {
  const _DeleteSheet({
    required this.session,
    required this.requestId,
    required this.close,
  });

  final AgentSession session;
  final String requestId;
  final VoidCallback close;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final fresh = ref.watch(sessionDetailProvider(session.id)).value;
    final s = fresh ?? session;
    final d = deleteDecisionOf(s);
    if (d == null || d.request.id != requestId) {
      WidgetsBinding.instance.addPostFrameCallback((_) => close());
      return const SizedBox.shrink();
    }
    return DeleteDecisionStrip(
      session: s,
      layout: DecisionLayout.sheet,
      onLater: close,
      onAnswered: close,
    );
  }
}

// ---- other screens and devices (153h) ------------------------------------

/// The requests this device has toasted, kept so a restart does not toast
/// them again. Device-local, like what it has seen.
const kDeleteToastedKey = 'storm.agents.delete_toasted';
const _toastedLimit = 200;

ScaffoldMessengerState? _agentToastOn;
ScaffoldFeatureController<SnackBar, SnackBarClosedReason>? _agentToast;

/// Raises [bar] in the agents' one toast slot (153h, 154-4): a newer toast
/// takes the place of the one showing, whichever watcher raised it, and is
/// never queued behind it.
ScaffoldFeatureController<SnackBar, SnackBarClosedReason>? showAgentToast(
  SnackBar bar,
) {
  final messenger = rootMessengerKey.currentState;
  if (messenger == null) return null;
  if (_agentToast != null && identical(_agentToastOn, messenger)) {
    messenger.removeCurrentSnackBar();
  }
  final toast = _agentToast = messenger.showSnackBar(bar);
  _agentToastOn = messenger;
  toast.closed.then((_) {
    if (_agentToast == toast) _agentToast = null;
  });
  return toast;
}

/// `/agents/s/<id>` → the session shown there.
String? shownSessionOf(String location) {
  final m = RegExp(r'^/agents/s/([^/?#]+)').firstMatch(location);
  return m == null ? null : Uri.decodeComponent(m.group(1)!);
}

/// Watches for agents' deletions on every screen while signed in (149c,
/// 153h). A long poll, paused in the background and stopped on a server
/// without the route. It never opens a dialog: a new request for a session
/// this device is not showing raises one toast with Review, once per
/// request, and the session's strip is where it is answered. The rail's
/// count says it too.
class DeleteRequestWatcher extends ConsumerStatefulWidget {
  const DeleteRequestWatcher({
    super.key,
    required this.location,
    required this.child,
  });

  /// Where the shell is, to know which session is on screen.
  final String location;
  final Widget child;

  @override
  ConsumerState<DeleteRequestWatcher> createState() =>
      _DeleteRequestWatcherState();
}

/// How long one poll waits for a change at the server.
const kDeleteWatchWait = 20;

class _DeleteRequestWatcherState extends ConsumerState<DeleteRequestWatcher> {
  int? _version;
  bool _running = false;
  bool _stopped = false;
  Timer? _retry;
  AppLifecycleListener? _lifecycle;

  /// Requests toasted, or shown in their session's strip, on this device.
  Set<String>? _toasted;

  /// Requests this device saw waiting, so one answered elsewhere while its
  /// session is open here is said.
  final _pending = <String>{};

  /// The toast on screen now, and its request: a newer one replaces it.
  String? _toastFor;
  ScaffoldFeatureController<SnackBar, SnackBarClosedReason>? _toast;

  @override
  void initState() {
    super.initState();
    _lifecycle = AppLifecycleListener(onResume: _poll);
    WidgetsBinding.instance.addPostFrameCallback((_) => _poll());
  }

  @override
  void dispose() {
    _stopped = true;
    _retry?.cancel();
    _lifecycle?.dispose();
    super.dispose();
  }

  bool get _foreground {
    final s = WidgetsBinding.instance.lifecycleState;
    return s == null || s == AppLifecycleState.resumed;
  }

  Future<Set<String>> _loadToasted() async {
    if (_toasted case final t?) return t;
    var loaded = <String>{};
    try {
      final prefs = await SharedPreferences.getInstance();
      loaded = {...?prefs.getStringList(kDeleteToastedKey)};
    } catch (_) {
      // No preferences store (tests, previews): memory still works.
    }
    return _toasted ??= loaded;
  }

  Future<void> _remember(String id) async {
    final toasted = await _loadToasted();
    if (!toasted.add(id)) return;
    final list = toasted.toList();
    try {
      final prefs = await SharedPreferences.getInstance();
      await prefs.setStringList(
        kDeleteToastedKey,
        list.sublist(math.max(0, list.length - _toastedLimit)),
      );
    } catch (_) {}
  }

  Future<void> _poll() async {
    if (_running || _stopped || !mounted || !_foreground) return;
    final api = ref.read(agentApiFactoryProvider)?.call();
    if (api == null) return;
    _running = true;
    _retry?.cancel();
    var again = Duration.zero;
    try {
      final (version, requests) = await api.deleteRequests(
        after: _version,
        wait: kDeleteWatchWait,
      );
      if (!mounted || _stopped) return;
      final first = _version == null;
      _version = version;
      await _show(requests, first: first);
    } on AgentRefusal catch (e) {
      // A server before decision 149 has no such route: nothing to watch.
      if (e.statusCode == 404 || e.statusCode == 403) {
        _stopped = true;
        return;
      }
      again = const Duration(seconds: 5);
    } catch (_) {
      again = const Duration(seconds: 5);
    } finally {
      api.dispose();
      _running = false;
    }
    if (!mounted || _stopped) return;
    // A floor between polls, so a server that answers at once is never
    // asked in a tight loop.
    _retry = Timer(
      again == Duration.zero ? const Duration(seconds: 1) : again,
      _poll,
    );
  }

  Future<void> _show(
    List<(DeleteRequest, DeleteAsker)> requests, {
    required bool first,
  }) async {
    final toasted = await _loadToasted();
    if (!mounted) return;
    final shown = shownSessionOf(widget.location);
    final c = ProviderScope.containerOf(context, listen: false);
    if (!first && requests.isNotEmpty) {
      // The strip and the rail count follow now, not at the next list poll.
      for (final id in {for (final (r, _) in requests) r.sessionId}) {
        c.invalidate(sessionDetailProvider(id));
      }
      c.invalidate(agentOverviewProvider);
    }
    for (final (r, _) in requests) {
      if (r.pending) continue;
      if (_toastFor == r.id) _hideToast();
      final waited = _pending.remove(r.id);
      final here = _answeredHere.remove(r.id);
      if (waited && !here && r.sessionId == shown) _answeredElsewhere(r);
    }
    for (final (r, asker) in requests) {
      if (!r.pending) continue;
      _pending.add(r.id);
      if (toasted.contains(r.id)) continue;
      unawaited(_remember(r.id));
      // Its session is on screen: the strip is already saying it.
      if (r.sessionId == shown) continue;
      _raise(r, asker);
    }
  }

  void _hideToast() {
    final toast = _toast;
    _toast = null;
    _toastFor = null;
    if (toast == null) return;
    try {
      toast.close();
    } catch (_) {}
  }

  void _raise(DeleteRequest r, DeleteAsker asker) {
    final router = GoRouter.maybeOf(context);
    final toast = showAgentToast(
      SnackBar(
        key: ValueKey('delete-toast-${r.id}'),
        content: Text('${asker.name} wants to delete ${deleteItem(r)}'),
        persist: false,
        duration: const Duration(seconds: 10),
        action: SnackBarAction(
          key: const Key('delete-toast-review'),
          label: 'Review',
          onPressed: () => router?.go(Routes.agentSession(r.sessionId)),
        ),
      ),
    );
    if (toast == null) return;
    _toast = toast;
    _toastFor = r.id;
    toast.closed.then((_) {
      if (_toast == toast) {
        _toast = null;
        _toastFor = null;
      }
    });
  }

  void _answeredElsewhere(DeleteRequest r) {
    rootMessengerKey.currentState?.showSnackBar(
      SnackBar(
        key: const Key('delete-answered-elsewhere'),
        content: Text(switch (r.state) {
          'timed_out' => 'The request to delete ${r.path} timed out.',
          _ => 'Answered on ${r.decidedBy ?? 'another device'}',
        }),
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    ref.listen(agentApiFactoryProvider, (_, next) {
      _version = null;
      if (next != null) _poll();
    });
    return widget.child;
  }
}
