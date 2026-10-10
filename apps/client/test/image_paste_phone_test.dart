import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:storm/router.dart';

import 'agent_fakes.dart';
import 'shell_harness.dart';

/// D15 AM45 on a phone: the extra keys' Paste takes an image first.
void main() {
  const phone = Size(411, 900);
  const channel = MethodChannel('storm/clipboard');
  final png = Uint8List.fromList([
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 1, 2, 3, //
  ]);

  Future<(FakeAgentServer, ProviderContainer)> openOnPhone(
    WidgetTester tester,
  ) async {
    final agents = FakeAgentServer(
      hosts: [agentHost()],
      sessions: [agentSession('ags_1')],
    );
    final c = shellContainer(agentClient: agents.client);
    await pumpShell(tester, c, size: phone);
    c.read(routerProvider).go(Routes.agentSession('ags_1'));
    await tester.pumpAndSettle();
    tester.view.viewInsets = const FakeViewPadding(bottom: 300);
    addTearDown(tester.view.resetViewInsets);
    await tester.pumpAndSettle();
    return (agents, c);
  }

  void clipboard(Uint8List? image) {
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(channel, (call) async {
          return call.method == 'readImage' ? image : null;
        });
    addTearDown(
      () => TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(channel, null),
    );
  }

  int staged(FakeAgentServer a) =>
      a.count('POST', '/v1/agent/sessions/ags_1/terminal/images');

  testWidgets('an image on the clipboard is staged, not pasted as text', (
    tester,
  ) async {
    final (agents, c) = await openOnPhone(tester);
    clipboard(png);
    await tester.tap(find.byKey(const Key('key-paste')));
    await tester.pumpAndSettle();
    await tester.pump(const Duration(milliseconds: 100));
    expect(staged(agents), 1);
    final req = agents.requests.lastWhere(
      (r) => r.url.path.endsWith('/terminal/images'),
    );
    expect(req.bodyBytes, png);
    expect(req.headers['content-type'], 'image/png');
    final typed = agents.requests
        .where((r) => r.url.path.endsWith('/terminal/input'))
        .map((r) => String.fromCharCodes(r.bodyBytes))
        .join();
    expect(typed, contains('/h/sessions/ags_1/inbox/img_1.png'));
    await disposeShell(tester, c);
  });

  testWidgets('no image: Paste stays a text paste', (tester) async {
    final (agents, c) = await openOnPhone(tester);
    clipboard(null);
    await tester.tap(find.byKey(const Key('key-paste')));
    await tester.pumpAndSettle();
    expect(staged(agents), 0);
    await disposeShell(tester, c);
  });
}
