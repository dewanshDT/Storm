import 'dart:async';
import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:storm/api/relay/srp_http_client.dart';
import 'package:storm/api/resumable_get.dart';

/// Serves [body] once per call, ending each attempt `client_backlogged`
/// after [cut] bytes until [failures] attempts have failed.
class _Backlogging extends http.BaseClient {
  _Backlogging(this.body, {this.failures = 1});

  final List<int> body;
  final int cut = 4;
  int failures;
  String etag = '"a"';
  bool rangesHonoured = true;
  final requests = <http.BaseRequest>[];

  @override
  Future<http.StreamedResponse> send(http.BaseRequest request) async {
    requests.add(request);
    final range = request.headers['range'];
    var from = 0;
    var status = 200;
    final headers = {'accept-ranges': 'bytes', 'etag': etag};
    if (range != null &&
        rangesHonoured &&
        request.headers['if-range'] == etag) {
      from = int.parse(range.substring('bytes='.length, range.length - 1));
      status = 206;
      headers['content-range'] =
          'bytes $from-${body.length - 1}/${body.length}';
    }
    final rest = body.sublist(from);
    final out = StreamController<List<int>>();
    if (failures > 0) {
      failures--;
      out.add(rest.sublist(0, cut));
      out.addError(const SrpTransportException('client_backlogged'));
    } else {
      out.add(rest);
    }
    unawaited(out.close());
    return http.StreamedResponse(out.stream, status, headers: headers);
  }
}

void main() {
  final body = utf8.encode('0123456789abcdef');

  test('a backlogged download resumes from what arrived (§5.5)', () async {
    final client = _Backlogging(body, failures: 2);
    final got = await resumableGet(client, Uri.parse('http://x/a'), {});
    expect(utf8.decode(got), '0123456789abcdef');
    expect(client.requests[1].headers['range'], 'bytes=4-');
    expect(client.requests[1].headers['if-range'], '"a"');
    expect(client.requests[2].headers['range'], 'bytes=8-');
  });

  test('a resource that changed since starts over, never splices', () async {
    final client = _Backlogging(body)..rangesHonoured = false;
    final got = await resumableGet(client, Uri.parse('http://x/a'), {});
    expect(utf8.decode(got), '0123456789abcdef');
  });

  test('a 206 that starts elsewhere is refused', () async {
    final wrong = _WrongOffset(body);
    await expectLater(
      resumableGet(wrong, Uri.parse('http://x/a'), {}),
      throwsA(isA<http.ClientException>()),
    );
  });

  test('any other transport error is not retried', () async {
    final client = _Failing();
    await expectLater(
      resumableGet(client, Uri.parse('http://x/a'), {}),
      throwsA(isA<SrpTransportException>()),
    );
    expect(client.calls, 1);
  });
}

class _WrongOffset extends _Backlogging {
  _WrongOffset(super.body);

  @override
  Future<http.StreamedResponse> send(http.BaseRequest request) async {
    if (request.headers['range'] == null) return super.send(request);
    return http.StreamedResponse(
      Stream.value(body.sublist(2)),
      206,
      headers: {'content-range': 'bytes 2-15/16', 'accept-ranges': 'bytes'},
    );
  }
}

class _Failing extends http.BaseClient {
  int calls = 0;

  @override
  Future<http.StreamedResponse> send(http.BaseRequest request) async {
    calls++;
    final out = StreamController<List<int>>();
    out.add([1, 2]);
    out.addError(const SrpTransportException('trunk_lost'));
    unawaited(out.close());
    return http.StreamedResponse(
      out.stream,
      200,
      headers: {'accept-ranges': 'bytes'},
    );
  }
}
