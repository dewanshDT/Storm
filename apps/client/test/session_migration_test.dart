import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:storm/agent/agent_api.dart';
import 'package:storm/agent/agent_widgets.dart';
import 'package:storm/agent/session_controller.dart';
import 'package:storm/agent/session_screen.dart';
import 'package:storm/agent/terminal_events.dart';
import 'package:storm/ui/cards.dart';
import 'package:storm/ui/settings/settings_widgets.dart' show TextAction;
import 'package:storm/ui/theme.dart';

Widget _host(Widget child) => MaterialApp(
  theme: StormTheme.dark(),
  home: Scaffold(body: Center(child: child)),
);

void main() {
  testWidgets('the image paste chip removes on a tap just outside the x', (
    tester,
  ) async {
    final c = SessionController(
      api: AgentApi(
        baseUrl: 'http://s',
        token: 't',
        client: MockClient((_) async => http.Response('', 204)),
      ),
      sessionId: 'ags_1',
      open: (_, _) => const Stream<TerminalEvent>.empty(),
    );
    c.imagePaste.value = const ImagePasteState.pasting(1);
    await tester.pumpWidget(_host(ImagePasteChip(controller: c)));
    expect(tester.takeException(), isNull);
    final x = find.byKey(const Key('image-paste-dismiss'));
    expect(x, findsOneWidget);
    final rect = tester.getRect(x);
    await tester.tapAt(Offset(rect.left - 4, rect.center.dy));
    await tester.pump();
    expect(c.imagePaste.value, isNull);
    expect(find.byKey(const Key('image-paste-chip')), findsNothing);
  });

  testWidgets('the migrated session pieces render and respond', (tester) async {
    var opened = 0;
    await tester.pumpWidget(
      _host(
        Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            WroteRow(
              key: const Key('plain'),
              title: 'Plan',
              crumb: 'Vault / Plan.md',
              meta: 'v2',
              onTap: () => opened++,
            ),
            WroteRow(
              title: 'Board',
              crumb: 'Vault / Board.md',
              meta: 'new · a',
              ruled: true,
              onTap: null,
            ),
            const Hairline(
              top: true,
              bottom: true,
              child: SizedBox(width: 50, height: 10),
            ),
            TextAction(
              label: 'Open in Notes ›',
              accent: true,
              link: true,
              oneLine: true,
              onTap: () => opened++,
            ),
            StormCard(
              color: Colors.transparent,
              semanticLabel: 'Open note',
              onTap: () => opened++,
              child: const Text('card'),
            ),
          ],
        ),
      ),
    );
    expect(tester.takeException(), isNull);
    await tester.tap(find.byKey(const Key('plain')));
    await tester.tap(find.text('Open in Notes ›'));
    await tester.tap(find.text('card'));
    expect(opened, 3);
  });
}
