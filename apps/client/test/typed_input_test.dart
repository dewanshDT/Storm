import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';

import 'package:storm/agent/agent_api.dart';
import 'package:storm/agent/agent_models.dart';

/// 153-S3: a Shell launched with a command typed in, and each agent's
/// sign-in command from the server.
void main() {
  Map<String, dynamic> launched() => {
    'id': 'ags_1',
    'host_id': 'hst_1',
    'workspace': 'storm',
    'provider': 'shell',
    'status': 'starting',
    'cols': 80,
    'rows': 24,
    'created_at': '2026-11-12T12:00:00Z',
  };

  Future<Map<String, dynamic>> sent(String? typedInput) async {
    late Map<String, dynamic> body;
    final api = AgentApi(
      baseUrl: 'http://s',
      token: 't',
      client: MockClient((req) async {
        body = jsonDecode(req.body) as Map<String, dynamic>;
        return http.Response(jsonEncode(launched()), 200);
      }),
    );
    await api.launch(
      hostId: 'hst_1',
      workspace: 'storm',
      provider: 'shell',
      cols: 80,
      rows: 24,
      typedInput: typedInput,
    );
    return body;
  }

  test('the launch carries typed_input, and none when there is none', () async {
    expect(
      (await sent('claude auth login'))['typed_input'],
      'claude auth login',
    );
    expect((await sent(null)).containsKey('typed_input'), isFalse);
    expect((await sent('')).containsKey('typed_input'), isFalse);
  });

  test('a host says whether it types and each agent its sign-in', () {
    final host = AgentHost.fromJson({
      'id': 'hst_1',
      'name': 'box',
      'status': 'online',
      'capabilities': {
        'typed_input': true,
        'providers': [
          {
            'id': 'claude-code',
            'kind': 'cli',
            'available': true,
            'login_command': 'claude auth login',
          },
          {
            'id': 'shell',
            'kind': 'cli',
            'available': true,
            'login_command': null,
          },
        ],
      },
    });
    expect(host.typedInput, isTrue);
    expect(host.providers[0].loginCommand, 'claude auth login');
    expect(host.providers[1].loginCommand, isNull);

    final old = AgentHost.fromJson({
      'id': 'hst_2',
      'name': 'old',
      'status': 'online',
      'capabilities': {
        'providers': [
          {'id': 'claude-code', 'kind': 'cli', 'available': true},
        ],
      },
    });
    expect(old.typedInput, isFalse, reason: 'an older runtime');
    expect(
      old.providers.single.loginCommand,
      isNull,
      reason: 'an older server',
    );
  });
}
