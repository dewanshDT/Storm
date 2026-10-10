package dev.storm.storm

import android.content.ClipboardManager
import android.content.Context
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
        MethodChannel(flutterEngine.dartExecutor.binaryMessenger, "storm/clipboard")
            .setMethodCallHandler { call, result ->
                if (call.method == "readImage") result.success(clipboardImage())
                else result.notImplemented()
            }
    }

    /**
     * The clipboard's image, as its bytes (D15 AM45): the first item whose
     * content URI names an image type. Bounded, so a huge copy cannot exhaust
     * memory; the server refuses anything over 10 MiB anyway.
     */
    private fun clipboardImage(): ByteArray? {
        val clipboard = getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
        val clip = clipboard.primaryClip ?: return null
        for (i in 0 until clip.itemCount) {
            val uri = clip.getItemAt(i).uri ?: continue
            val type = contentResolver.getType(uri) ?: continue
            if (!type.startsWith("image/")) continue
            return try {
                contentResolver.openInputStream(uri)?.use { input ->
                    val bytes = input.readNBytesCompat(MAX_IMAGE + 1)
                    if (bytes.size > MAX_IMAGE) null else bytes
                }
            } catch (e: Exception) {
                null
            }
        }
        return null
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

private const val MAX_IMAGE = 12 * 1024 * 1024

/** `readNBytes` is API 33+; this reads at most [limit] bytes on any API. */
private fun java.io.InputStream.readNBytesCompat(limit: Int): ByteArray {
    val out = java.io.ByteArrayOutputStream()
    val buffer = ByteArray(64 * 1024)
    while (out.size() < limit) {
        val n = read(buffer, 0, minOf(buffer.size, limit - out.size()))
        if (n < 0) break
        out.write(buffer, 0, n)
    }
    return out.toByteArray()
}
