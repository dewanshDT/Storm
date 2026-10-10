import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:storm/agent/terminal_surface.dart';
import 'package:storm/router.dart';
import 'package:xterm2/xterm.dart';

import 'agent_fakes.dart';
import 'shell_harness.dart';

/// Decision 87: an image the on-screen keyboard inserts (Gboard's GIFs and
/// stickers) is staged and its path typed, as a pasted one is.
void main() {
  const phone = Size(411, 900);
  final png = Uint8List.fromList([
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 1, 2, 3, //
  ]);

  Future<(FakeAgentServer, ProviderContainer)> openWithKeyboard(
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
    await tester.tap(find.byType(TerminalView));
    await tester.pumpAndSettle();
    return (agents, c);
  }

  Map<String, dynamic> lastClientConfig(WidgetTester tester) {
    final call = tester.testTextInput.log.lastWhere(
      (c) => c.method == 'TextInput.setClient',
    );
    return (call.arguments as List)[1] as Map<String, dynamic>;
  }

  Future<void> insert(WidgetTester tester, String mime, Uint8List? data) async {
    final call = tester.testTextInput.log.lastWhere(
      (c) => c.method == 'TextInput.setClient',
    );
    final id = (call.arguments as List)[0] as int;
    await tester.binding.defaultBinaryMessenger.handlePlatformMessage(
      SystemChannels.textInput.name,
      SystemChannels.textInput.codec.encodeMethodCall(
        MethodCall('TextInputClient.performAction', [
          id,
          'TextInputAction.commitContent',
          {
            'mimeType': mime,
            'uri': 'content://com.google.android.inputmethod.latin/1',
            'data': data,
          },
        ]),
      ),
      (_) {},
    );
    await tester.pumpAndSettle();
    await tester.pump(const Duration(milliseconds: 100));
  }

  int staged(FakeAgentServer a) =>
      a.count('POST', '/v1/agent/sessions/ags_1/terminal/images');

  testWidgets('the session tells the keyboard it takes images', (tester) async {
    final (_, c) = await openWithKeyboard(tester);
    expect(
      lastClientConfig(tester)['contentCommitMimeTypes'],
      StormTerminal.keyboardImageTypes,
    );
    await disposeShell(tester, c);
  });

  testWidgets('an inserted image is staged and its path typed', (tester) async {
    final (agents, c) = await openWithKeyboard(tester);
    await insert(tester, 'image/png', png);
    expect(staged(agents), 1);
    final req = agents.requests.lastWhere(
      (r) => r.url.path.endsWith('/terminal/images'),
    );
    expect(req.bodyBytes, png);
    final typed = agents.requests
        .where((r) => r.url.path.endsWith('/terminal/input'))
        .map((r) => String.fromCharCodes(r.bodyBytes))
        .join();
    expect(typed, contains('/h/sessions/ags_1/inbox/img_1.png'));
    expect(typed, isNot(contains('content://')));
    await disposeShell(tester, c);
  });

  testWidgets('an insertion with no bytes or no image stages nothing', (
    tester,
  ) async {
    final (agents, c) = await openWithKeyboard(tester);
    await insert(tester, 'image/png', null);
    await insert(tester, 'image/png', Uint8List.fromList([1, 2, 3]));
    expect(staged(agents), 0);
    await disposeShell(tester, c);
  });

  testWidgets('a read-only terminal offers the keyboard no images', (
    tester,
  ) async {
    final terminal = StormTerminal()..onKeyboardImage = (_) {};
    addTearDown(terminal.dispose);
    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: StormTerminalView(terminal: terminal, readOnly: true),
        ),
      ),
    );
    final view = tester.widget<TerminalView>(find.byType(TerminalView));
    expect(view.contentInsertionConfiguration, isNull);
  });
}
