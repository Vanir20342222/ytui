package dev.ytui.android

import android.net.Uri
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createEmptyComposeRule
import androidx.lifecycle.ViewModelProvider
import androidx.media3.datasource.*
import androidx.media3.datasource.cache.CacheDataSource
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import java.io.File
import java.net.*
import java.util.concurrent.*
import org.json.JSONObject
import org.junit.*
import org.junit.runner.RunWith

/** Real indexed MP4 requests; whole-file cold reads are deliberately too slow. */
@RunWith(AndroidJUnit4::class)
@androidx.annotation.OptIn(markerClass = [androidx.media3.common.util.UnstableApi::class])
class IndexedPlaybackTest {
    @get:Rule val compose = createEmptyComposeRule()
    @get:Rule
    val notifications =
        androidx.test.rule.GrantPermissionRule.grant(
            *if (android.os.Build.VERSION.SDK_INT >= 33)
                arrayOf("android.permission.POST_NOTIFICATIONS")
            else emptyArray()
        )

    private class Cdn : AutoCloseable {
        val assets = InstrumentationRegistry.getInstrumentation().context.assets
        val contract =
            JSONObject(assets.open("dash/contract.json").use { it.bufferedReader().readText() })
        val files =
            (0..1).map {
                assets.open("dash/index-stream$it.mp4").use { input -> input.readBytes() }
            }
        val socket = ServerSocket(0, 8, InetAddress.getByName("127.0.0.1"))
        val workers = Executors.newFixedThreadPool(4)
        val requests = java.util.Collections.synchronizedList(mutableListOf<String>())
        val thread =
            Thread {
                    while (!socket.isClosed) {
                        try {
                            val client = socket.accept()
                            workers.execute {
                                client.use {
                                    try {
                                        val reader = it.getInputStream().bufferedReader()
                                        val line = reader.readLine() ?: return@use
                                        val uri = Uri.parse("http://localhost" + line.split(' ')[1])
                                        var header: String? = null
                                        while (true) {
                                            val h = reader.readLine() ?: break
                                            if (h.isEmpty()) break
                                            if (h.startsWith("Range:", true))
                                                header =
                                                    h.substringAfter(':')
                                                        .trim()
                                                        .removePrefix("bytes=")
                                        }
                                        val bytes =
                                            if (uri.getQueryParameter("invalid_media") == "1")
                                                "not media".toByteArray()
                                            else
                                                files[
                                                    uri.getQueryParameter("track")?.toIntOrNull()
                                                        ?: 0]
                                        val ignored = uri.getQueryParameter("ignore_query") == "1"
                                        if (
                                            uri.getQueryParameter("reject_query") == "1" &&
                                                uri.getQueryParameter("range") != null
                                        ) {
                                            requests.add("rejected-query")
                                            it.getOutputStream().apply {
                                                write(
                                                    "HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n"
                                                        .toByteArray()
                                                )
                                                flush()
                                            }
                                            return@use
                                        }
                                        val query =
                                            if (ignored) null else uri.getQueryParameter("range")
                                        val raw = query ?: header
                                        val start = raw?.substringBefore('-')?.toIntOrNull() ?: 0
                                        val end =
                                            (raw?.substringAfter('-')?.toIntOrNull()
                                                    ?: bytes.lastIndex)
                                                .coerceAtMost(bytes.lastIndex)
                                        requests.add(
                                            if (raw != null && raw.endsWith('-')) "unbounded:$raw"
                                            else if (query != null) "query:$raw"
                                            else if (header != null) "header:$raw" else "whole"
                                        )
                                        val status =
                                            if (header != null && query == null)
                                                "206 Partial Content"
                                            else "200 OK"
                                        val range =
                                            if (status.startsWith("206"))
                                                "Content-Range: bytes $start-$end/${bytes.size}\r\n"
                                            else ""
                                        val out = it.getOutputStream()
                                        out.write(
                                            "HTTP/1.1 $status\r\nContent-Type: video/mp4\r\nContent-Length: ${end-start+1}\r\n$range\r\n"
                                                .toByteArray()
                                        )
                                        out.flush()
                                        if ((raw == null || raw.endsWith('-')) && !ignored)
                                            Thread.sleep(3000)
                                        out.write(bytes, start, end - start + 1)
                                        out.flush()
                                    } catch (_: Exception) {}
                                }
                            }
                        } catch (_: Exception) {}
                    }
                }
                .apply { start() }
        val base: String
            get() = "http://127.0.0.1:${socket.localPort}"

        fun url(index: Int) =
            contract
                .getJSONArray("tracks")
                .getJSONObject(index)
                .getString("url")
                .replace("http://127.0.0.1:1", base)

        override fun close() {
            socket.close()
            thread.join(1000)
            workers.shutdownNow()
            workers.awaitTermination(3, TimeUnit.SECONDS)
        }
    }

    @Test
    fun indexedColdVideoStartsAndIsVisibleWithoutAnyWholeFileRead() {
        val original = MobileSettings.snapshot()
        MobileSettings.save(json("preload_next" to false, "autoplay_related" to false))
        val evidence = StringBuilder()
        try {
            Cdn().use { cdn ->
                RuntimeTest.launchOffline().use { scene ->
                    compose.waitUntil(10000) {
                        compose
                            .onAllNodesWithTag("app-background")
                            .fetchSemanticsNodes()
                            .isNotEmpty()
                    }
                    scene.onActivity {
                        ViewModelProvider(it)[YtuiViewModel::class.java].playerOpen.value = true
                    }
                    compose.waitForIdle()
                    val token = StartupTimings.begin(android.os.SystemClock.elapsedRealtime(), true)
                    StartupTimings.resolved(token, android.os.SystemClock.elapsedRealtime(), false)
                    val request =
                        json(
                            "media" to
                                json(
                                    "url" to cdn.url(0),
                                    "kind" to "video",
                                    "title" to "Indexed cold fixture",
                                ),
                            "stream" to
                                json(
                                    "url" to cdn.url(0),
                                    "audio_url" to cdn.url(1),
                                    "dash_manifest" to
                                        cdn.contract
                                            .getString("dash_manifest")
                                            .replace("http://127.0.0.1:1", cdn.base),
                                ),
                            "audio" to false,
                            "quality" to "best",
                            "startup_token" to token,
                        )
                    MediaBridge.execute(json("op" to "play", "request" to request).toString())
                    compose.waitUntil(10000) { StartupTimings.usable(token) > 0 }
                    Assert.assertTrue(
                        "Indexed usable start: ${StartupTimings.usable(token)}\n${StartupTimings.report()}",
                        StartupTimings.usable(token) <= 2000,
                    )
                    var rgb = false
                    var observed = -1L
                    do {
                        if (
                            StartupTimings.elapsed(
                                token,
                                android.os.SystemClock.elapsedRealtime(),
                            ) > 2000
                        )
                            break
                        val sample = VisibleVideoProbe.capture(scene, token, "startup-indexed.png")
                        rgb = sample.rgb
                        observed = StartupTimings.elapsed(token, sample.capturedAt)
                        if (!rgb) Thread.sleep(16)
                    } while (!rgb)
                    Assert.assertTrue("Indexed pixels: $observed", rgb && observed in 0..2000)
                    val requests = synchronized(cdn.requests) { cdn.requests.toList() }
                    Assert.assertTrue(requests.isNotEmpty())
                    Assert.assertFalse(
                        "Unbounded reads defeat indexed startup: $requests",
                        requests.any { it == "whole" || it.startsWith("unbounded:") },
                    )
                    Assert.assertTrue(StartupTimings.report().contains("transport=dash-indexed"))
                    evidence
                        .append(StartupTimings.report())
                        .append(
                            "indexed_screenshot_confirmation_ms=$observed\nrequests=$requests\n"
                        )
                    MediaBridge.execute(json("op" to "seek", "seconds" to 3).toString())
                    compose.waitUntil(5000) {
                        MediaRuntimeTest.status().optDouble("position") >= 2.8
                    }
                    MediaRuntimeTest.command("stop")
                }
            }
        } finally {
            File(
                    InstrumentationRegistry.getInstrumentation()
                        .targetContext
                        .getExternalFilesDir(null),
                    "startup-indexed-timings.txt",
                )
                .writeText(evidence.toString())
            MobileSettings.save(original)
        }
    }

    @Test
    fun sourceFailureAndStopAreRecordedInsteadOfRemainingPending() {
        Cdn().use { cdn ->
            RuntimeTest.launchOffline().use {
                fun request(url: String, token: Long) =
                    json(
                        "op" to "play",
                        "request" to
                            json(
                                "media" to
                                    json(
                                        "url" to url,
                                        "kind" to "video",
                                        "title" to "Failure fixture",
                                    ),
                                "stream" to json("url" to url),
                                "audio" to false,
                                "startup_token" to token,
                            ),
                    )
                val failed = StartupTimings.begin(android.os.SystemClock.elapsedRealtime(), true)
                StartupTimings.resolved(failed, android.os.SystemClock.elapsedRealtime(), false)
                MediaBridge.execute(
                    request(cdn.url(0) + "&ignore_query=1&invalid_media=1", failed).toString()
                )
                compose.waitUntil(10000) {
                    PlaybackService.current?.startupToken == failed &&
                        !MediaRuntimeTest.status().isNull("error")
                }
                Assert.assertTrue(
                    StartupTimings.report()
                        .lineSequence()
                        .last { it.startsWith("Video") }
                        .contains("failed")
                )
                Assert.assertEquals(-1L, StartupTimings.usable(failed))
                val cancelled = StartupTimings.begin(android.os.SystemClock.elapsedRealtime(), true)
                StartupTimings.resolved(cancelled, android.os.SystemClock.elapsedRealtime(), false)
                MediaBridge.execute(request(cdn.url(0), cancelled).toString())
                compose.waitUntil(10000) { PlaybackService.current?.startupToken == cancelled }
                MediaRuntimeTest.command("stop")
                compose.waitUntil(5000) {
                    StartupTimings.report()
                        .lineSequence()
                        .last { it.startsWith("Video") }
                        .contains(" cancelled cache=")
                }
                Assert.assertEquals(-1L, StartupTimings.usable(cancelled))
            }
        }
    }

    @Test
    fun queryRange200UsesExactOffsetsAndOriginalCacheIdentity() {
        Cdn().use { cdn ->
            RuntimeTest.launchOffline().use {
                val uri = Uri.parse(cdn.url(0))
                val factory =
                    DataSource.Factory {
                        IndexedRangeDataSource(DefaultHttpDataSource.Factory().createDataSource()) {
                            it.host == "127.0.0.1"
                        }
                    }
                val source =
                    CacheDataSource.Factory()
                        .setCache(PlaybackCache.cache())
                        .setUpstreamDataSourceFactory(factory)
                        .createDataSource()
                val spec = DataSpec.Builder().setUri(uri).setPosition(64).setLength(512).build()
                DataSourceInputStream(source, spec).use { input ->
                    Assert.assertArrayEquals(cdn.files[0].copyOfRange(64, 576), input.readBytes())
                }
                Assert.assertTrue(
                    PlaybackCache.cache().getCachedBytes(uri.toString(), 64, 512) >= 512
                )
                Assert.assertTrue(cdn.requests.contains("query:64-575"))
                Assert.assertTrue(
                    IndexedRangeDataSource.youtubeCdn(
                        Uri.parse("https://r1.googlevideo.com/videoplayback")
                    )
                )
                Assert.assertFalse(
                    IndexedRangeDataSource.youtubeCdn(
                        Uri.parse("https://googlevideo.com.evil.test/videoplayback")
                    )
                )
            }
        }
    }

    @Test
    fun ignoredOrSignedQueryRangesUseTheOriginalHttpRangeRequest() {
        Cdn().use { cdn ->
            RuntimeTest.launchOffline().use {
                for (suffix in listOf("&ignore_query=1", "&reject_query=1", "&sparams=range")) {
                    val source =
                        IndexedRangeDataSource(DefaultHttpDataSource.Factory().createDataSource()) {
                            it.host == "127.0.0.1"
                        }
                    val spec =
                        DataSpec.Builder()
                            .setUri(cdn.url(0) + suffix)
                            .setPosition(64)
                            .setLength(512)
                            .build()
                    DataSourceInputStream(source, spec).use { input ->
                        Assert.assertArrayEquals(
                            cdn.files[0].copyOfRange(64, 576),
                            input.readBytes(),
                        )
                    }
                    Assert.assertTrue(cdn.requests.contains("header:64-575"))
                }
            }
        }
    }
}
