/// A `GET` that survives a relay ending it `client_backlogged` (SRP §5.5).
///
/// v1 has no flow control: a relay holds at most a budget of unread bytes
/// per stream, and a download that outruns its reader past it ends with
/// `client_backlogged`. The bytes already read are good, so an attachment is
/// resumed with `Range: bytes=<received>-` rather than fetched again from
/// the start, which on a slow phone would end the same way.
library;

import 'dart:async';
import 'dart:typed_data';

import 'package:http/http.dart' as http;

import 'relay/srp_http_client.dart';

/// Resumes after this many backlogged ends that each made progress.
const _maxResumes = 8;

/// The whole body of `GET [uri]`, resumed with `Range` when a relay ends it
/// `client_backlogged`. Throws [http.ClientException] for a non-2xx answer.
Future<Uint8List> resumableGet(
  http.Client client,
  Uri uri,
  Map<String, String> headers,
) async {
  final got = BytesBuilder(copy: false);
  String? etag;
  var resumes = 0;
  for (;;) {
    final request = http.Request('GET', uri)..headers.addAll(headers);
    final from = got.length;
    if (from > 0) {
      request.headers['range'] = 'bytes=$from-';
      if (etag != null) request.headers['if-range'] = etag;
    }
    final response = await client.send(request);
    final status = response.statusCode;
    if (from > 0 && status == 206) {
      if (!_startsAt(response.headers['content-range'], from)) {
        unawaited(response.stream.listen(null).cancel());
        throw http.ClientException('resumed at the wrong offset', uri);
      }
    } else if (status == 200) {
      // A first attempt, or a resource that changed since: start over.
      got.clear();
      etag = response.headers['etag'];
    } else {
      unawaited(response.stream.listen(null).cancel());
      throw http.ClientException('HTTP $status', uri);
    }
    final resumable =
        (response.headers['accept-ranges'] ?? '').toLowerCase() == 'bytes';
    final before = got.length;
    try {
      await for (final chunk in response.stream) {
        got.add(chunk);
      }
      return got.takeBytes();
    } on SrpTransportException catch (e) {
      final progressed = got.length > before;
      if (e.code != 'client_backlogged' ||
          !resumable ||
          !progressed ||
          ++resumes > _maxResumes) {
        rethrow;
      }
    }
  }
}

/// Whether `Content-Range: bytes <from>-<end>/<len>` starts where we stopped.
bool _startsAt(String? contentRange, int from) {
  final m = RegExp(
    r'^bytes (\d+)-\d+/(\d+|\*)$',
  ).firstMatch(contentRange ?? '');
  return m != null && int.parse(m.group(1)!) == from;
}
