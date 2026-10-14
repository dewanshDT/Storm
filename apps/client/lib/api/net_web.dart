import 'dart:async';

import 'package:http/http.dart' as http;
import 'package:web_socket_channel/web_socket_channel.dart';

http.Client directHttpClient() => http.Client();

// A browser cannot ping; the origin's feed keeps the socket alive.
WebSocketChannel connectFeed(Uri uri) => WebSocketChannel.connect(uri);

// A browser does not say why a fetch failed.
bool isConnectionRefused(Object e) => false;

bool isConnectionLevel(Object e) =>
    e is http.ClientException || e is TimeoutException;
