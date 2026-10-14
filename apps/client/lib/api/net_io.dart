import 'dart:async';
import 'dart:io';

import 'package:http/http.dart' as http;
import 'package:http/io_client.dart';
import 'package:web_socket_channel/io.dart';
import 'package:web_socket_channel/web_socket_channel.dart';

/// A client with a connect timeout, unless one was injected through
/// `http.runWithClient`, which the tests and any embedder rely on.
http.Client directHttpClient() {
  final zoned = http.Client();
  if (zoned is! IOClient) return zoned;
  zoned.close();
  return IOClient(
    HttpClient()..connectionTimeout = const Duration(seconds: 10),
  );
}

WebSocketChannel connectFeed(Uri uri) =>
    IOWebSocketChannel.connect(uri, pingInterval: const Duration(seconds: 30));

// ECONNREFUSED on Linux/Android, macOS/iOS and Windows.
const _refused = {111, 61, 10061};

bool isConnectionRefused(Object e) =>
    e is SocketException && _refused.contains(e.osError?.errorCode);

bool isConnectionLevel(Object e) =>
    e is http.ClientException ||
    e is SocketException ||
    e is HttpException ||
    e is TimeoutException;
