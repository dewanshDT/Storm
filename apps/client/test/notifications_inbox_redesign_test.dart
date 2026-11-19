/// Decision 161's inbox redesign: proof that the new [NotificationsPanel]
/// (header, filter, day sections, grouping, kind icons, held tags, agent
/// bodies, keyboard nav and the fixed badge count) actually renders, plus
/// PNGs for visual review — no committed goldens exist in this repo, so
/// these are saved for the coordinator to look at rather than compared
/// pixel-for-pixel.
library;

import 'dart:async';
import 'dart:io';
import 'dart:ui' as ui;

import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';

import 'package:storm/agent/agent_api.dart';
import 'package:storm/agent/agent_events.dart' show AgentNotificationPushed;
import 'package:storm/agent/notifications_models.dart';
import 'package:storm/agent/notifications_state.dart';
import 'package:storm/agent/notifications_ui.dart';
import 'package:storm/ui/theme.dart';
import 'package:storm/ui/tokens.dart' show StormPreset;

import 'agent_fakes.dart';
import 'notifications_test.dart' show FakeNotifications;
import 'shell_harness.dart';

const _reportsDir =
    '/var/lib/storm-runtime/workspaces/scratch/reports/161-inbox-redesign';

/// Widget tests render unknown fonts as tofu boxes; loading the real faces
/// is what makes a saved PNG worth looking at rather than a grid of
/// rectangles.
Future<void> _loadFonts() async {
  Future<void> load(String family, List<String> assets) async {
    final loader = FontLoader(family);
    for (final asset in assets) {
      loader.addFont(rootBundle.load(asset));
    }
    await loader.load();
  }

  await load('IBMPlexSans', [
    'assets/fonts/IBMPlexSans-Regular.ttf',
    'assets/fonts/IBMPlexSans-Medium.ttf',
    'assets/fonts/IBMPlexSans-SemiBold.ttf',
  ]);
  await load('IBMPlexMono', [
    'assets/fonts/IBMPlexMono-Regular.ttf',
    'assets/fonts/IBMPlexMono-Medium.ttf',
  ]);
  // The kind icons (Lucide) are a package font, keyed by Flutter's own
  // "packages/<pkg>/<family>" convention, or every icon falls back to tofu.
  await load('packages/flutter_lucide/lucide', [
    'packages/flutter_lucide/lib/fonts/lucide.ttf',
  ]);
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  setUpAll(_loadFonts);

  /// Answers every notifications endpoint the panel can call — a plain
  /// "items":[] for the backfill is not enough once a row's own actions
  /// (mark read, mute) make further calls.
  AgentApi emptyApi() => AgentApi(
    baseUrl: 'http://x',
    token: 't',
    client: MockClient((req) async {
      final path = req.url.path;
      if (path.endsWith('/v1/notifications/mutes')) {
        return http.Response('[]', 200);
      }
      if (path.endsWith('/v1/notifications/prefs')) {
        return http.Response(
          '{"version":1,"events":{},"quiet_on":false,"quiet_start":"22:00",'
          '"quiet_end":"08:00","quiet_tz":"UTC",'
          '"quiet_allow_delete_requests":false,"summary_on":true,'
          '"agent_default_on":true,"children_default_on":false,'
          '"per_session_limit":20,"retention_days":30,'
          '"retention_rows":1000}',
          200,
        );
      }
      if (path.endsWith('/v1/notifications/read')) {
        return http.Response('{"read":[]}', 200);
      }
      return http.Response('{"items":[],"cursor":0}', 200);
    }),
  );

  AgentNotification notif({
    required String id,
    required int seq,
    required String kind,
    required String title,
    String source = 'storm',
    String? sessionId,
    String? sessionTitle,
    String? provider,
    String? body,
    bool read = false,
    bool silent = false,
    String? silentReason,
    required DateTime created,
  }) => AgentNotification(
    id: id,
    seq: seq,
    created: created.toUtc().toIso8601String(),
    source: source,
    kind: kind,
    sessionId: sessionId,
    provider: provider,
    sessionTitle: sessionTitle,
    title: title,
    body: body,
    urgency: 'normal',
    readAt: read ? DateTime.now().toUtc().toIso8601String() : null,
    silent: silent,
    silentReason: silentReason,
  );

  /// A realistic mix: 2 unread needs-you, a finished, an error, an agent
  /// message with a body, a terminal notice, one held by quiet hours, and
  /// a 3-notice group (same session, consecutive).
  List<AgentNotification> sampleItems() {
    final now = DateTime.now();
    return [
      notif(
        id: 'n1',
        seq: 1,
        kind: 'needs_you',
        title: 'tidier needs you',
        sessionId: 'ags_1',
        sessionTitle: 'tidier',
        provider: 'claude-code',
        created: now,
      ),
      notif(
        id: 'n2',
        seq: 2,
        kind: 'needs_you',
        title: 'deploy bot needs you',
        sessionId: 'ags_2',
        sessionTitle: 'deploy bot',
        provider: 'opencode',
        created: now,
      ),
      notif(
        id: 'n3',
        seq: 3,
        kind: 'finished',
        title: 'build-vm finished its turn',
        sessionId: 'ags_3',
        sessionTitle: 'build-vm',
        provider: 'claude-code',
        read: true,
        created: now,
      ),
      notif(
        id: 'n4',
        seq: 4,
        kind: 'error',
        title: 'release-prep hit an error',
        sessionId: 'ags_4',
        sessionTitle: 'release-prep',
        provider: 'codex',
        created: now,
      ),
      notif(
        id: 'n5',
        seq: 5,
        kind: 'agent_message',
        source: 'agent',
        title: "Fixed the flaky test, PR's up for review.",
        body: 'Also updated the changelog.',
        sessionId: 'ags_5',
        sessionTitle: 'fix-flake',
        provider: 'claude-code',
        created: now,
      ),
      notif(
        id: 'n6',
        seq: 6,
        kind: 'terminal_notice',
        source: 'terminal',
        title: 'tests failed on the second run',
        sessionId: 'ags_6',
        sessionTitle: 'ci-watch',
        provider: 'codex',
        created: now,
      ),
      notif(
        id: 'n7',
        seq: 7,
        kind: 'finished',
        title: 'late-night finished its turn',
        sessionId: 'ags_7',
        sessionTitle: 'late-night',
        provider: 'opencode',
        silent: true,
        silentReason: 'quiet',
        created: now,
      ),
      // A 3-notice group: same session, consecutive, newest first.
      notif(
        id: 'n10',
        seq: 10,
        kind: 'needs_you',
        title: 'task loop needs you',
        sessionId: 'ags_9',
        sessionTitle: 'task loop',
        provider: 'claude-code',
        created: now,
      ),
      notif(
        id: 'n9',
        seq: 9,
        kind: 'error',
        title: 'task loop hit an error',
        sessionId: 'ags_9',
        sessionTitle: 'task loop',
        provider: 'claude-code',
        created: now,
      ),
      notif(
        id: 'n8',
        seq: 8,
        kind: 'finished',
        title: 'task loop finished its turn',
        sessionId: 'ags_9',
        sessionTitle: 'task loop',
        provider: 'claude-code',
        read: true,
        created: now,
      ),
    ];
  }

  /// Settles the state's own backfill (an empty page over a fake client),
  /// then applies [items] as live pushes — never through the network, so
  /// the test owns their order and fields exactly.
  Future<NotificationsState> buildState(
    WidgetTester tester,
    List<AgentNotification> items,
  ) async {
    final state = NotificationsState(emptyApi, null);
    for (var i = 0; i < 8; i++) {
      await tester.pump(Duration.zero);
    }
    expect(state.loading, isFalse);
    for (final n in items) {
      state.applyEvent(AgentNotificationPushed(n));
    }
    return state;
  }

  final boundaryKey = GlobalKey();

  /// [NotificationsPanel] paints its own surface now (coordinator review
  /// item 1), so this only needs to size it and give it something to sit
  /// on — a mid-tone backdrop makes the panel's own edge visible in the
  /// saved PNG, which a same-coloured background would hide.
  Widget harness(
    NotificationsState state,
    ThemeData theme, {
    bool phone = false,
  }) => ProviderScope(
    overrides: [notificationsStateProvider.overrideWithValue(state)],
    child: MaterialApp(
      theme: theme,
      debugShowCheckedModeBanner: false,
      // `Material`: a popover is normally hosted inside `showStormPopover`'s
      // own transparent `Material`, and a sheet inside `Scaffold`'s. Bare
      // content has nothing for its `InkWell`s to paint onto.
      home: Material(
        color: const Color(0xFF808080),
        child: phone
            ? Column(
                mainAxisAlignment: MainAxisAlignment.end,
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  RepaintBoundary(
                    key: boundaryKey,
                    child: const NotificationsPanel(phone: true),
                  ),
                ],
              )
            : Center(
                child: RepaintBoundary(
                  key: boundaryKey,
                  child: SizedBox(
                    width: 380,
                    child: const NotificationsPanel(),
                  ),
                ),
              ),
      ),
    ),
  );

  /// Best-effort: [_reportsDir] exists for the coordinator's own review in
  /// this sandbox, but is neither present nor writable on CI's runner —
  /// which must not fail a test over a developer convenience the
  /// `expect`s above it already proved the substance of.
  Future<void> writePng(ui.Image image, String name) async {
    try {
      final bytes = await image.toByteData(format: ui.ImageByteFormat.png);
      final file = File('$_reportsDir/$name.png')..createSync(recursive: true);
      await file.writeAsBytes(bytes!.buffer.asUint8List());
    } on FileSystemException {
      // Not this environment's job.
    }
  }

  Future<void> save(
    WidgetTester tester,
    String name, {
    bool settle = true,
  }) async {
    if (settle) await tester.pumpAndSettle();
    await tester.runAsync(() async {
      final boundary =
          boundaryKey.currentContext!.findRenderObject()
              as RenderRepaintBoundary;
      final image = await boundary.toImage(pixelRatio: 2.0);
      await writePng(image, name);
    });
  }

  /// A whole-window shot — no key needed: [captureImage] walks up from the
  /// given element to the nearest repaint boundary, which the app's own
  /// root provides.
  Future<void> saveWindow(WidgetTester tester, String name) async {
    await tester.pumpAndSettle();
    await tester.runAsync(() async {
      final image = await captureImage(
        tester.element(find.byType(MaterialApp).first),
      );
      await writePng(image, name);
    });
  }

  group('NotificationsPanel (decision 161 redesign)', () {
    for (final (label, theme) in [
      ('storm-dark', StormTheme.from(StormPreset.stormDark)),
      ('storm-light', StormTheme.from(StormPreset.stormLight)),
      ('slowflow-earth', StormTheme.from(StormPreset.slowflowEarth)),
    ]) {
      testWidgets('renders the realistic mix — $label', (tester) async {
        tester.view.physicalSize = const Size(480, 1000);
        tester.view.devicePixelRatio = 1.0;
        addTearDown(tester.view.reset);
        final state = await buildState(tester, sampleItems());
        await tester.pumpWidget(harness(state, theme));
        await tester.pumpAndSettle();

        // Header: title, unread pill (every notice but the read "task
        // loop finished" and the silent "late-night" one, item 10).
        expect(find.text('Notifications'), findsOneWidget);
        expect(find.byKey(const Key('notify-mark-all-read')), findsOneWidget);
        expect(find.byKey(const Key('notify-settings')), findsOneWidget);
        expect(find.byKey(const Key('notify-overflow')), findsOneWidget);

        // Day section and the held tag, readable words rather than a
        // wire code (item 7).
        expect(find.text('TODAY'), findsOneWidget);
        expect(find.text('During quiet hours'), findsOneWidget);

        // The agent's own words, quoted and marked quietly (item 6).
        expect(find.text('From the agent'), findsOneWidget);
        expect(
          find.text(
            "Fixed the flaky test, PR's up for review.\nAlso updated the changelog.",
          ),
          findsOneWidget,
        );
        expect(find.text('From the agent, via its terminal'), findsOneWidget);

        // The three "task loop" notices fold into one group row (item 5);
        // its own rows are not shown collapsed.
        expect(find.byKey(const ValueKey('grp:n10')), findsOneWidget);
        expect(find.byKey(const Key('notify-row-n10')), findsNothing);
        expect(find.byKey(const Key('notify-row-n9')), findsNothing);
        expect(find.byKey(const Key('notify-row-n8')), findsNothing);
        expect(find.textContaining('3 updates'), findsOneWidget);

        await save(tester, 'collapsed-$label');

        // Expand the group: its three rows now show, each on its own.
        await tester.tap(find.byKey(const ValueKey('grp:n10')));
        await tester.pumpAndSettle();
        expect(find.byKey(const Key('notify-row-n10')), findsOneWidget);
        expect(find.byKey(const Key('notify-row-n9')), findsOneWidget);
        expect(find.byKey(const Key('notify-row-n8')), findsOneWidget);

        if (label == 'storm-light') {
          await save(tester, 'expanded-$label');
        }
      });
    }

    testWidgets('the bell badge excludes the silent notice (item 10)', (
      tester,
    ) async {
      final state = await buildState(tester, sampleItems());
      // 7 of the 10 sample notices actually notified: "build-vm" and the
      // group's "task loop finished" are already read, and "late-night"
      // is held by quiet hours (silent) — none of the three should count.
      expect(notifyingUnreadCount(state), 7);
    });

    testWidgets('the Unread filter hides read and leaves the rest', (
      tester,
    ) async {
      tester.view.physicalSize = const Size(480, 1000);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);
      final state = await buildState(tester, sampleItems());
      await tester.pumpWidget(
        harness(state, StormTheme.from(StormPreset.stormDark)),
      );
      await tester.pumpAndSettle();
      await tester.tap(find.text('Unread'));
      await tester.pumpAndSettle();
      // The read "build-vm finished" row is gone from the flat list; the
      // group (which still has unread members) stays.
      expect(find.byKey(const Key('notify-row-n3')), findsNothing);
      expect(find.byKey(const ValueKey('grp:n10')), findsOneWidget);
    });

    testWidgets('keyboard: down focuses the first row, enter opens it', (
      tester,
    ) async {
      tester.view.physicalSize = const Size(480, 1000);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);
      final state = await buildState(tester, [
        notif(
          id: 'k1',
          seq: 1,
          kind: 'needs_you',
          title: 'alpha needs you',
          sessionId: 'ags_k',
          sessionTitle: 'alpha',
          provider: 'claude-code',
          created: DateTime.now(),
        ),
      ]);
      var opened = false;
      await tester.pumpWidget(
        ProviderScope(
          overrides: [notificationsStateProvider.overrideWithValue(state)],
          child: MaterialApp(
            theme: StormTheme.from(StormPreset.stormDark),
            home: Scaffold(
              body: NotificationsPanel(onOpened: () => opened = true),
            ),
          ),
        ),
      );
      await tester.pumpAndSettle();
      await tester.sendKeyEvent(LogicalKeyboardKey.arrowDown);
      await tester.pumpAndSettle();
      await tester.sendKeyEvent(LogicalKeyboardKey.enter);
      await tester.pumpAndSettle();
      expect(opened, isTrue);
      expect(
        state.items.firstWhere((n) => n.id == 'k1').unread,
        isFalse,
        reason: 'Enter opens the focused row and marks it read',
      );
    });

    testWidgets('the empty state shows a quiet icon and a settings link', (
      tester,
    ) async {
      tester.view.physicalSize = const Size(480, 1000);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);
      final state = await buildState(tester, const []);
      await tester.pumpWidget(
        harness(state, StormTheme.from(StormPreset.stormDark)),
      );
      await tester.pumpAndSettle();
      expect(find.text("You're all caught up"), findsOneWidget);
      expect(find.text('Notification settings'), findsOneWidget);
      await save(tester, 'empty-storm-dark');
    });

    testWidgets('the loading state shows a spinner', (tester) async {
      tester.view.physicalSize = const Size(480, 1000);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);
      // A `Completer` the test never completes within its own body, rather
      // than a timed delay: `flutter_test` flags a `Timer` still pending at
      // teardown, but not an unresolved `Future`, so this never needs one
      // cancelled.
      final gate = Completer<http.Response>();
      AgentApi stuckApi() => AgentApi(
        baseUrl: 'http://x',
        token: 't',
        client: MockClient((req) async => gate.future),
      );
      final state = NotificationsState(stuckApi, null);
      addTearDown(state.dispose);
      await tester.pumpWidget(
        harness(state, StormTheme.from(StormPreset.stormDark)),
      );
      await tester.pump();
      expect(find.byType(CircularProgressIndicator), findsOneWidget);
      // `settle: false`: the spinner animates forever, which would make
      // `pumpAndSettle` inside `save` spin until it gives up.
      await save(tester, 'loading-storm-dark', settle: false);
      gate.complete(http.Response('{"items":[],"cursor":0}', 200));
      await tester.pump(Duration.zero);
    });

    testWidgets('the error state offers a retry', (tester) async {
      tester.view.physicalSize = const Size(480, 1000);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);
      AgentApi failingApi() => AgentApi(
        baseUrl: 'http://x',
        token: 't',
        client: MockClient((req) async => http.Response('nope', 500)),
      );
      final state = NotificationsState(failingApi, null);
      for (var i = 0; i < 8; i++) {
        await tester.pump(Duration.zero);
      }
      expect(state.backfillFailed, isTrue);
      await tester.pumpWidget(
        harness(state, StormTheme.from(StormPreset.stormDark)),
      );
      // Zero-duration pumps only: `pumpAndSettle` steps fake time forward
      // and would run past the state's own 3-second retry, firing a
      // second failing backfill that schedules a timer `dispose` below
      // never gets a chance to see, let alone cancel.
      for (var i = 0; i < 4; i++) {
        await tester.pump(Duration.zero);
      }
      expect(find.text("Couldn't load notifications."), findsOneWidget);
      await save(tester, 'error-storm-dark', settle: false);
      // `addTearDown` runs after `flutter_test`'s own pending-timer check,
      // too late to cancel the state's 3-second retry — it must go here.
      state.dispose();
    });

    testWidgets('the phone sheet at 390×844', (tester) async {
      tester.view.physicalSize = const Size(390, 844);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);
      final state = await buildState(tester, sampleItems());
      await tester.pumpWidget(
        harness(state, StormTheme.from(StormPreset.stormLight), phone: true),
      );
      await tester.pumpAndSettle();
      expect(find.text('Notifications'), findsOneWidget);
      await save(tester, 'phone-sheet-storm-light');
    });

    for (final (label, theme) in [
      ('storm-dark', StormTheme.from(StormPreset.stormDark)),
      ('storm-light', StormTheme.from(StormPreset.stormLight)),
    ]) {
      testWidgets(
        'full desktop window: the popover anchored under the bell — $label',
        (tester) async {
          final notif = FakeNotifications()
            ..push(
              id: 'w1',
              source: 'storm',
              kind: 'needs_you',
              title: 'tidier needs you',
              sessionId: 'ags_1',
              sessionTitle: 'tidier',
              provider: 'claude-code',
            )
            ..push(
              id: 'w2',
              source: 'storm',
              kind: 'finished',
              title: 'build-vm finished its turn',
              sessionId: 'ags_2',
              sessionTitle: 'build-vm',
              provider: 'claude-code',
            )
            ..push(
              id: 'w3',
              source: 'storm',
              kind: 'error',
              title: 'release-prep hit an error',
              sessionId: 'ags_3',
              sessionTitle: 'release-prep',
              provider: 'codex',
            );
          final agents = FakeAgentServer(
            hosts: [
              agentHost(
                id: 'hst_1',
                name: 'build-vm',
                providers: ['claude-code'],
                identity: 'verified',
                isolation: 'verified',
              ),
            ],
          );
          agents.extra = notif.handle;
          final c = shellContainer(agentClient: agents.client);
          await pumpShell(tester, c, size: const Size(1400, 900), theme: theme);
          await tester.pumpAndSettle();
          await tester.tap(find.byKey(const Key('bar-notifications')));
          await tester.pumpAndSettle();
          expect(find.text('Notifications'), findsOneWidget);
          await saveWindow(tester, 'desktop-window-$label');
          await disposeShell(tester, c);
        },
      );
    }
  });
}
