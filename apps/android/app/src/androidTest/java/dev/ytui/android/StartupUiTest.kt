package dev.ytui.android

import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createEmptyComposeRule
import androidx.lifecycle.ViewModelProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import java.io.File
import java.util.concurrent.atomic.AtomicReference
import org.json.JSONObject
import org.junit.*
import org.junit.runner.RunWith

/** Includes the real card tap, Rust resolution, service creation and visible native frame. */
@RunWith(AndroidJUnit4::class)
@OptIn(androidx.compose.ui.test.ExperimentalTestApi::class)
class StartupUiTest {
    @get:Rule
    val compose = createEmptyComposeRule(effectContext = kotlinx.coroutines.Dispatchers.Main)
    @get:Rule
    val notifications =
        androidx.test.rule.GrantPermissionRule.grant(
            *if (android.os.Build.VERSION.SDK_INT >= 33)
                arrayOf("android.permission.POST_NOTIFICATIONS")
            else emptyArray()
        )

    @Test
    fun coldAndPreparedCardTapsReachAVisibleFrameWithinTwoSeconds() {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val file = File(instrumentation.targetContext.cacheDir, "startup-fixture.mp4")
        instrumentation.context.assets.open("fixture.mp4").use { input ->
            file.outputStream().use { input.copyTo(it) }
        }
        val original = MobileSettings.snapshot()
        MobileSettings.save(json("preload_next" to false, "autoplay_related" to false))
        val confirmations = StringBuilder()
        try {
            MediaRuntimeTest.FixtureServer(file).use { server ->
                RuntimeTest.launchOffline().use { scenario ->
                    compose.waitUntil(10000) {
                        compose
                            .onAllNodes(
                                hasTestTag("app-background") and
                                    SemanticsMatcher.expectValue(
                                        SemanticsProperties.StateDescription,
                                        "Ready",
                                    )
                            )
                            .fetchSemanticsNodes()
                            .isNotEmpty()
                    }
                    val vm = AtomicReference<YtuiViewModel>()
                    scenario.onActivity { vm.set(ViewModelProvider(it)[YtuiViewModel::class.java]) }
                    for (prepared in listOf(false, true)) {
                        NativeBridge.request(json("op" to "resolve_cache_clear").toString())
                        if (prepared) {
                            val result =
                                JSONObject(
                                    NativeBridge.request(
                                        json(
                                                "op" to "resolve",
                                                "media" to
                                                    json("url" to server.url(), "kind" to "video"),
                                                "audio" to false,
                                                "quality" to
                                                    (vm.get()
                                                        .config
                                                        .value
                                                        .optJSONObject("download")
                                                        ?.optString("quality", "1080") ?: "1080"),
                                            )
                                            .toString()
                                    )
                                )
                            Assert.assertTrue(result.optString("error"), result.optBoolean("ok"))
                        }
                        scenario.onActivity {
                            vm.get().playerOpen.value = false
                            vm.get().feeds[1].value =
                                FeedState(
                                    cards =
                                        listOf(
                                            MediaCard(
                                                json(
                                                    "url" to server.url(),
                                                    "title" to "Startup fixture",
                                                    "kind" to "video",
                                                )
                                            )
                                        ),
                                    generation = if (prepared) 902 else 901,
                                )
                        }
                        compose.onNodeWithText("Startup fixture").performClick()
                        compose.waitUntil(10000) {
                            val status = MediaRuntimeTest.status()
                            status.optJSONObject("media")?.optString("url") == server.url() &&
                                status.optLong("tap_to_output_ms", -1) > 0
                        }
                        val elapsed = MediaRuntimeTest.status().getLong("tap_to_output_ms")
                        Assert.assertTrue(
                            "${if (prepared) "Prepared" else "Cold"} tap to visible frame was $elapsed ms\n${StartupTimings.report()}",
                            elapsed <= 2000,
                        )
                        var rgb = false
                        var observed = -1L
                        do {
                            observed =
                                StartupTimings.elapsed(
                                    PlaybackService.current.startupToken,
                                    android.os.SystemClock.elapsedRealtime(),
                                )
                            if (observed < 0 || observed > 2000) break
                            val sample =
                                VisibleVideoProbe.capture(
                                    scenario,
                                    PlaybackService.current.startupToken,
                                    "startup-${if(prepared) "prepared" else "cold"}.png",
                                )
                            rgb = sample.rgb
                            observed =
                                StartupTimings.elapsed(
                                    PlaybackService.current.startupToken,
                                    sample.capturedAt,
                                )
                            if (!rgb) Thread.sleep(16)
                        } while (!rgb)
                        confirmations.append(
                            "${if(prepared) "prepared" else "cold"} screenshot_confirmation_ms=$observed\n"
                        )
                        if (!rgb) MediaRuntimeTest.capture("startup-missing-frame.png")
                        Assert.assertTrue(
                            "RGB image was not presented within 2000 ms: $observed ms\n${StartupTimings.report()}",
                            rgb && observed in 0..2000,
                        )
                        MediaRuntimeTest.command("stop")
                        compose.waitUntil(10000) {
                            MediaRuntimeTest.status().optJSONObject("media") == null
                        }
                    }
                    scenario.onActivity { vm.get().playerOpen.value = false }
                    compose.onNodeWithTag("settings-button").performClick()
                    compose.onNodeWithTag("settings-Performance").performClick()
                    compose.onNodeWithTag("startup-timings").assertIsDisplayed()
                    compose.onNodeWithTag("copy-startup-timings").performScrollTo().performClick()
                    scenario.onActivity {
                        val clipboard =
                            it.getSystemService(android.content.Context.CLIPBOARD_SERVICE)
                                as android.content.ClipboardManager
                        val text = clipboard.primaryClip?.getItemAt(0)?.coerceToText(it).toString()
                        Assert.assertTrue(text.contains("Video prepared"))
                        Assert.assertFalse(text.contains("http"))
                    }
                    MediaRuntimeTest.capture("startup-settings.png")
                }
            }
        } finally {
            File(instrumentation.targetContext.getExternalFilesDir(null), "startup-timings.txt")
                .writeText(StartupTimings.report() + confirmations)
            MobileSettings.save(original)
        }
    }

    @Test
    fun selectedHlsCodecCatalogReachesTheVisiblePlayerWithoutAnotherMasterRequest() {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val assets = instrumentation.context.assets
        val bodies =
            assets.list("hls")!!.associate { name ->
                "/hls/$name" to assets.open("hls/$name").use { it.readBytes() }
            }
        val original = MobileSettings.snapshot()
        MobileSettings.save(json("preload_next" to false, "autoplay_related" to false))
        val socket = java.net.ServerSocket(0, 8, java.net.InetAddress.getByName("127.0.0.1"))
        val paths = java.util.Collections.synchronizedList(mutableListOf<String>())
        val server = Thread {
            while (!socket.isClosed) {
                try {
                    socket.accept().use { client ->
                        val reader = client.getInputStream().bufferedReader()
                        val request = reader.readLine() ?: return@use
                        val path = request.split(' ')[1].substringBefore('?')
                        paths.add(path)
                        while (!reader.readLine().isNullOrEmpty()) {}
                        val body = bodies[path]
                        val status = if (body == null) "404 Not Found" else "200 OK"
                        val type =
                            if (path.endsWith("m3u8")) "application/vnd.apple.mpegurl"
                            else "video/mp2t"
                        client.getOutputStream().apply {
                            write(
                                "HTTP/1.1 $status\r\nContent-Type: $type\r\nContent-Length: ${body?.size ?: 0}\r\nConnection: close\r\n\r\n"
                                    .toByteArray()
                            )
                            if (body != null) write(body)
                            flush()
                        }
                    }
                } catch (_: java.io.IOException) {}
            }
        }
        server.start()
        try {
            RuntimeTest.launchOffline().use { scenario ->
                compose.waitUntil(10000) {
                    compose.onAllNodesWithTag("app-background").fetchSemanticsNodes().isNotEmpty()
                }
                scenario.onActivity {
                    ViewModelProvider(it)[YtuiViewModel::class.java].playerOpen.value = true
                }
                compose.waitForIdle()
                val url = "http://127.0.0.1:${socket.localPort}/hls/media.m3u8"
                val master =
                    "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-STREAM-INF:BANDWIDTH=800000,CODECS=\"avc1.42c00d,mp4a.40.2\"\n$url\n"
                for (prepared in listOf(false, true)) {
                    if (prepared) {
                        PlaybackCache.cancel()
                        val base = "http://127.0.0.1:${socket.localPort}"
                        bodies.keys.forEach { PlaybackCache.cache().removeResource(base + it) }
                        paths.clear()
                        MobileSettings.prefs()
                            .edit()
                            .putBoolean("preload_next", true)
                            .putBoolean("preload_mobile", true)
                            .commit()
                        PlaybackCache.prefetch(json("url" to url, "kind" to "video"), false, "best")
                        compose.waitUntil(10000) { PlaybackCache.ready }
                        val first = "$base/hls/segment0.ts"
                        Assert.assertTrue(
                            "HLS preload must contain segment bytes, not only playlist text",
                            PlaybackCache.cache()
                                .getCachedBytes(
                                    first,
                                    0,
                                    androidx.media3.common.C.LENGTH_UNSET.toLong(),
                                ) > 0,
                        )
                        Assert.assertEquals(
                            1,
                            synchronized(paths) { paths.count { it == "/hls/segment0.ts" } },
                        )
                        MobileSettings.prefs().edit().putBoolean("preload_next", false).commit()
                    }
                    val token = StartupTimings.begin(android.os.SystemClock.elapsedRealtime(), true)
                    StartupTimings.resolved(token, android.os.SystemClock.elapsedRealtime(), false)
                    val request =
                        json(
                            "media" to
                                json(
                                    "url" to url,
                                    "title" to "HLS startup fixture",
                                    "kind" to "video",
                                ),
                            "stream" to json("url" to url, "hls_master" to master),
                            "audio" to false,
                            "quality" to "best",
                            "startup_token" to token,
                        )
                    MediaBridge.execute(json("op" to "play", "request" to request).toString())
                    compose.waitUntil(10000) { StartupTimings.total(token) > 0 }
                    Assert.assertTrue(
                        "HLS first frame exceeded 2000 ms: ${StartupTimings.total(token)}",
                        StartupTimings.total(token) <= 2000,
                    )
                    Assert.assertTrue(
                        MediaRuntimeTest.videoVisible(
                            if (prepared) "startup-hls-prepared.png" else "startup-hls.png",
                            false,
                        )
                    )
                    Assert.assertTrue(
                        StartupTimings.report().contains("quality=best transport=hls-chunkless")
                    )
                    val requested = synchronized(paths) { paths.toList() }
                    Assert.assertTrue(requested.contains("/hls/media.m3u8"))
                    Assert.assertTrue(requested.any { it.endsWith(".ts") })
                    Assert.assertTrue(
                        "Only the selected media playlist and segments may be loaded: $requested",
                        requested.all {
                            it == "/hls/media.m3u8" || it.matches(Regex("/hls/segment[0-9]+\\.ts"))
                        },
                    )
                    if (prepared)
                        Assert.assertEquals(
                            "Playback must reuse the warmed first segment",
                            1,
                            requested.count { it == "/hls/segment0.ts" },
                        )
                    MediaRuntimeTest.command("stop")
                    compose.waitUntil(5000) { PlaybackService.current == null }
                }
            }
        } finally {
            File(instrumentation.targetContext.getExternalFilesDir(null), "startup-hls-timings.txt")
                .writeText(StartupTimings.report())
            socket.close()
            server.join(2000)
            MobileSettings.save(original)
        }
    }

    @Test
    fun foregroundResolutionBypassesSaturatedBackgroundRpcWorkers() {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val file = File(instrumentation.targetContext.cacheDir, "rpc-priority-fixture.mp4")
        instrumentation.context.assets.open("fixture.mp4").use { input ->
            file.outputStream().use { input.copyTo(it) }
        }
        val gate = java.util.concurrent.CountDownLatch(1)
        val occupied = java.util.concurrent.CountDownLatch(8)
        val poolField =
            RustRpc::class.java.getDeclaredField("workers").apply { isAccessible = true }
        val pool = poolField.get(RustRpc) as java.util.concurrent.ThreadPoolExecutor
        val blockers = mutableListOf<java.util.concurrent.Future<*>>()
        try {
            MediaRuntimeTest.FixtureServer(file).use { server ->
                RuntimeTest.launchOffline().use {
                    kotlinx.coroutines.runBlocking { RustRpc.objectCall("settings") }
                    repeat(8) {
                        blockers +=
                            pool.submit {
                                occupied.countDown()
                                gate.await()
                            }
                    }
                    Assert.assertTrue(
                        "Background workers did not reach the saturation fixture",
                        occupied.await(10, java.util.concurrent.TimeUnit.SECONDS),
                    )
                    val stream =
                        kotlinx.coroutines.runBlocking {
                            kotlinx.coroutines.withTimeout(1500) {
                                RustRpc.objectCall(
                                    "resolve",
                                    "media" to json("url" to server.url(), "kind" to "video"),
                                    "audio" to false,
                                    "quality" to "1080",
                                )
                            }
                        }
                    Assert.assertEquals(server.url(), stream.optString("url"))
                    Assert.assertEquals("miss", stream.optString("resolution_cache_state"))
                    Assert.assertEquals(
                        1L,
                        gate.count,
                    ) // foreground finished while every background worker was still blocked
                }
            }
        } finally {
            gate.countDown()
            blockers.forEach { it.get(10, java.util.concurrent.TimeUnit.SECONDS) }
        }
    }
}
