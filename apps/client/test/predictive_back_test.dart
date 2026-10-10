import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:storm/router.dart';

import 'agent_fakes.dart';
import 'fake_server.dart';
import 'shell_harness.dart';

/// Android 16's predictive back (target SDK 36): the platform asks the app
/// whether it handles back *before* the gesture, through
/// `SystemNavigator.setFrameworkHandlesBack`, and closes the app when the
/// answer is no. A page reached with `go` has nothing to pop, so it must say
/// yes whenever it has a logical parent — or back leaves Storm.
void main() {
  const v = FakeServer.primaryVault;
  const phone = Size(411, 900);
  late List<bool> claims;
  final android = TargetPlatformVariant.only(TargetPlatform.android);

  setUp(() {
    claims = [];
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(SystemChannels.platform, (call) async {
          if (call.method == 'SystemNavigator.setFrameworkHandlesBack') {
            claims.add(call.arguments as bool);
          }
          return null;
        });
  });

  tearDown(() {
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(SystemChannels.platform, null);
  });

  String where(ProviderContainer c) => c.read(routerProvider).state.uri.path;

  Future<void> go(WidgetTester tester, ProviderContainer c, String to) async {
    c.read(routerProvider).go(to);
    await tester.pumpAndSettle();
  }

  /// What the platform was last told; then the gesture itself.
  Future<bool> back(WidgetTester tester) async {
    final handled = await tester.binding.handlePopRoute();
    await tester.pumpAndSettle();
    return handled;
  }

  Future<ProviderContainer> open(WidgetTester tester) async {
    final agents = FakeAgentServer(
      hosts: [agentHost()],
      sessions: [agentSession('ags_1')],
    );
    final c = shellContainer(agentClient: agents.client);
    tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.resumed);
    await pumpShell(tester, c, size: phone);
    return c;
  }

  testWidgets('a note opened with go claims back, and back is its folder', (
    tester,
  ) async {
    final c = await open(tester);
    expect(where(c), Routes.browse(v));
    expect(claims.last, isFalse, reason: 'the vault root lets back leave');

    await go(tester, c, Routes.note(v, 'n3'));
    expect(claims.last, isTrue, reason: 'or Android closes the app');
    expect(await back(tester), isTrue);
    expect(where(c), startsWith(Routes.browse(v)));
    expect(where(c), isNot(contains('/note/')));
    await disposeShell(tester, c);
  }, variant: android);

  testWidgets('a session claims back, and back is the Agents list', (
    tester,
  ) async {
    final c = await open(tester);
    await go(tester, c, Routes.agentSession('ags_1'));
    expect(claims.last, isTrue);
    expect(await back(tester), isTrue);
    expect(where(c), Routes.agents);

    // From the list, back goes to the last Notes place, not out.
    expect(claims.last, isTrue);
    expect(await back(tester), isTrue);
    expect(where(c), startsWith('/v/'));
    await disposeShell(tester, c);
  }, variant: android);

  testWidgets('the Agents place opens the list, not the last session', (
    tester,
  ) async {
    final c = await open(tester);
    await go(tester, c, Routes.agentSession('ags_1'));
    await go(tester, c, Routes.browse(v));
    await tester.tap(find.byKey(const Key('places-bubble')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('place-agents')));
    await tester.pumpAndSettle();
    expect(where(c), Routes.agents);
    await disposeShell(tester, c);
  }, variant: android);
}
