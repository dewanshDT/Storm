package dev.storm.storm

import android.content.Intent
import io.flutter.embedding.android.FlutterActivity
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.MethodChannel

/**
 * Hands `storm://oauth` redirects to Dart (spec §10.4, decision 81l).
 *
 * Links are buffered until Dart calls `takeLinks`, then forwarded as `link`
 * calls: a redirect that launched the app arrives before Dart is listening,
 * and must not be lost to that race.
 */
class MainActivity : FlutterActivity() {
    private var links: MethodChannel? = null
    private val buffered = mutableListOf<String>()
    private var dartReady = false

    override fun configureFlutterEngine(flutterEngine: FlutterEngine) {
        super.configureFlutterEngine(flutterEngine)
        oauthLink(intent)?.let { buffered.add(it) }
        val channel = MethodChannel(flutterEngine.dartExecutor.binaryMessenger, "storm/links")
        channel.setMethodCallHandler { call, result ->
            if (call.method == "takeLinks") {
                result.success(buffered.toList())
                buffered.clear()
                dartReady = true
            } else {
                result.notImplemented()
            }
        }
        links = channel
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        val link = oauthLink(intent) ?: return
        val channel = links
        if (dartReady && channel != null) {
            channel.invokeMethod("link", link)
        } else {
            buffered.add(link)
        }
    }

    /** The intent's URI if it is a `storm://oauth` link; nothing else. */
    private fun oauthLink(intent: Intent?): String? {
        val data = intent?.data ?: return null
        return if (data.scheme == "storm" && data.host == "oauth") data.toString() else null
    }
}
