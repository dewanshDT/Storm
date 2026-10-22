import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';

import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;

import 'package:storm/agent/agent_state.dart';
import 'package:storm/agent/terminal_events.dart';
import 'package:storm/agent/terminal_stream.dart';
import 'package:storm/api/auth_models.dart';
import 'package:storm/api/storm_api.dart';
import 'package:storm/state/app_state.dart';
import 'package:storm/ui/storm_image_provider.dart';

/// Signed in away from home (decision 112): the saved address is a closed
/// port, and Agents (list, launch, the live terminal) and an attachment image
/// reach the real storm-server through the real `storm-relay`, over the app's
/// own providers. `make test-live` registers the relay, enrolls a runtime host
/// named `STORM_AGENT_HOST` offering the `fake` provider, and passes both.
void main() {
  const origin = String.fromEnvironment('STORM_RELAY_ORIGIN');
  const session = String.fromEnvironment('STORM_SESSION');
  const hostName = String.fromEnvironment('STORM_AGENT_HOST');
  const deadline = Duration(seconds: 20);

  late ProviderContainer c;
  late StormApi direct;
  late String vaultId;

  Future<T> until<T>(Future<T?> Function() probe, String what) async {
    final end = DateTime.now().add(deadline);
    while (DateTime.now().isBefore(end)) {
      final got = await probe();
      if (got != null) return got;
      await Future<void>.delayed(const Duration(milliseconds: 200));
    }
    fail('timed out waiting for $what');
  }

  setUpAll(() async {
    if (origin.isEmpty || session.isEmpty || hostName.isEmpty) {
      fail(
        'Run through `make test-live`: it starts storm-relay and a runtime '
        'host and passes STORM_RELAY_ORIGIN, STORM_SESSION and '
        'STORM_AGENT_HOST.',
      );
    }
    final token = session.replaceFirst('Bearer ', '');
    direct = StormApi(baseUrl: origin, token: token);
    vaultId = (await direct.vaults()).first.id;

    final info = await until(() async {
      final i = ServerInfo.fromJson(
        jsonDecode((await http.get(Uri.parse('$origin/v1/server'))).body)
            as Map<String, dynamic>,
      );
      return i.relays.isEmpty ? null : i;
    }, 'the server to register with the relay');

    // A port nothing listens on: the LAN address is "not here".
    final socket = await ServerSocket.bind(InternetAddress.loopbackIPv4, 0);
    final closed = socket.port;
    await socket.close();

    final settings = Settings(
      baseUrl: 'http://127.0.0.1:$closed',
      accessToken: token,
      serverId: info.serverId,
      serverKeyId: info.keyId,
      serverPublicKey: info.publicKey,
      activeVault: vaultId,
      relays: info.relays,
    );
    c = ProviderContainer(
      overrides: [settingsProvider.overrideWith(() => _Fixed(settings))],
    );
    await c.read(settingsProvider.future);
  });

  tearDownAll(() {
    c.read(connectionProvider)?.close();
    c.dispose();
    direct.dispose();
  });

  test('Agents lists, launches and streams a live terminal through the '
      'relay', () async {
    final api = c.read(agentApiFactoryProvider)!();
    final host = await until(() async {
      final hosts = await api.hosts();
      return hosts.where((h) => h.name == hostName && h.online).firstOrNull;
    }, 'the runtime host to come online');
    expect(c.read(connectionProvider)!.isRelayed, isTrue);

    final s = await api.launch(
      hostId: host.id,
      workspace: 'storm',
      provider: 'fake',
      cols: 80,
      rows: 24,
    );
    expect((await api.sessions()).map((x) => x.id), contains(s.id));

    final text = StringBuffer();
    final statuses = <String>[];
    final done = Completer<void>();
    final sub = openTerminalStream(api, s.id, 0).listen(
      (e) {
        switch (e) {
          case StatusEvent(:final session):
            statuses.add(session.status);
          case OutputEvent(:final bytes):
            text.write(utf8.decode(bytes));
          case GapEvent() || LiveEvent():
            break;
        }
      },
      onDone: done.complete,
      onError: done.completeError,
    );

    await until(
      () async => text.toString().contains('fake session ${s.id}') ? 1 : null,
      'the start line',
    );
    // Typed after the stream opened, so it can only arrive if the relayed
    // stream hands chunks on as they come rather than at its end.
    await api.input(s.id, Uint8List.fromList(utf8.encode('via relay\r')));
    await until(
      () async => text.toString().contains('via relay') ? 1 : null,
      'the echo of the input',
    );

    await api.end(s.id);
    await done.future.timeout(deadline);
    expect(statuses.last, 'stopped');
    await sub.cancel();
    await api.dismiss(s.id);
    api.dispose();
  });

  test('an attachment image loads through the relay', () async {
    final png = base64.decode(
      'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==',
    );
    final path =
        'attachments/relayed-${DateTime.now().microsecondsSinceEpoch}.png';
    await direct.uploadAttachment(vaultId, path, png);

    final image = StormImageProvider.attachment(
      c.read(apiProvider)!,
      vaultId,
      path,
    );
    expect(image.uri.port, isNot(Uri.parse(origin).port));
    expect(await image.fetch().timeout(deadline), png);
    expect(c.read(connectionProvider)!.isRelayed, isTrue);
  });
}

class _Fixed extends SettingsNotifier {
  _Fixed(this.value);
  final Settings value;

  @override
  Future<Settings> build() async => value;

  @override
  Future<void> save(Settings next) async => state = AsyncData(next);
}
