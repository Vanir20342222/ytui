package dev.ytui.android

import android.net.Uri
import androidx.media3.datasource.DataSource
import androidx.media3.datasource.DataSpec
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import java.io.ByteArrayOutputStream
import java.io.IOException
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import okhttp3.mockwebserver.Dispatcher
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import okhttp3.mockwebserver.RecordedRequest
import okhttp3.mockwebserver.SocketPolicy
import okio.Buffer
import org.json.JSONObject
import org.junit.*
import org.junit.runner.RunWith

/** Real loopback sockets/cache spans. No account cookies or public media are used. */
@RunWith(AndroidJUnit4::class)
@androidx.annotation.OptIn(markerClass = [androidx.media3.common.util.UnstableApi::class])
class PlaybackPerformanceTest {
    private lateinit var mobile: JSONObject
    private lateinit var config: JSONObject

    @Before
    fun setup() {
        mobile = MobileSettings.snapshot()
        config = PlaybackService.rpc(json("op" to "settings"))
        InstrumentationRegistry.getInstrumentation().runOnMainSync {
            PlaybackService.current?.stopAndClear()
        }
        // The previous test can deliberately leave a playback failure. Cleanup
        // must still wait for actual service destruction, not fail on that error.
        MediaRuntimeTest.await("Previous test service must be closed", false) {
            PlaybackService.current == null
        }
        PlaybackCache.cancel()
        MobileSettings.save(json("preload_next" to true, "preload_mobile" to true))
        NetworkSettings.apply(json("proxy" to ""))
    }

    @After
    fun restore() {
        PlaybackCache.cancel()
        MobileSettings.save(mobile)
        NetworkSettings.apply(config)
    }

    private fun read(source: DataSource, spec: DataSpec): ByteArray {
        try {
            source.open(spec)
            val out = ByteArrayOutputStream()
            val bytes = ByteArray(8192)
            while (true) {
                val count = source.read(bytes, 0, bytes.size)
                if (count < 0) break
                out.write(bytes, 0, count)
                Assert.assertTrue("Fixture read remains bounded", out.size() <= 2 * 1024 * 1024)
            }
            return out.toByteArray()
        } finally {
            source.close()
        }
    }

    @Test
    fun foregroundPreparesAnIdlePlayerAndReleasesItWithoutPlayback() {
        val owner = java.util.concurrent.atomic.AtomicReference<RustActivity>()
        RuntimeTest.launchOffline().use { scene ->
            scene.onActivity { owner.set(it) }
            MediaRuntimeTest.await("Visible app should prepare player infrastructure") {
                PlaybackService.warmReady() && owner.get().videoViews.ready()
            }
            scene.onActivity {
                val view = it.videoViews.peek()!!
                Assert.assertNull("Idle view must have no parent", view.parent)
                Assert.assertNull("Idle view must have no bound player", view.player)
                Assert.assertFalse(view.isAttachedToWindow)
                Assert.assertFalse(
                    "Detached preparation must not create a decoder surface",
                    when (val surface = view.videoSurfaceView) {
                        is android.view.TextureView -> surface.isAvailable
                        is android.view.SurfaceView -> surface.holder.surface.isValid
                        else -> true
                    },
                )
                val service = PlaybackService.current!!
                Assert.assertNull(service.currentRequest.optJSONObject("media"))
                Assert.assertFalse(service.player.isPlaying)
                Assert.assertTrue(
                    "Idle preparation must not register a media session",
                    service.sessions.isEmpty(),
                )
                val manager = it.getSystemService(android.app.NotificationManager::class.java)
                Assert.assertTrue(
                    "Idle preparation must not create a playback notification",
                    manager.activeNotifications.none { n -> n.id == NotificationIds.PLAYBACK },
                )
            }
            val previousOwner = owner.get()
            val previousView = previousOwner.videoViews.peek()
            scene.recreate()
            scene.onActivity { owner.set(it) }
            MediaRuntimeTest.await("Recreated browsing activity should prepare its own view") {
                owner.get().videoViews.ready()
            }
            Assert.assertFalse(
                "Destroyed activity must discard its view",
                previousOwner.videoViews.ready(),
            )
            scene.onActivity {
                Assert.assertNotSame(previousView, it.videoViews.peek())
                Assert.assertSame(it, it.videoViews.peek()!!.context)
                Assert.assertNull(it.videoViews.peek()!!.player)
            }
        }
        MediaRuntimeTest.await("Leaving an idle app must release player infrastructure") {
            PlaybackService.current == null
        }
        Assert.assertFalse(
            "The activity must discard its unused view",
            owner.get().videoViews.ready(),
        )
    }

    @Test
    fun pooledConnectionsDoNotShareHeadersOrAcceptResponseCookies() {
        MockWebServer().use { server ->
            server.enqueue(
                MockResponse().setBody("first").addHeader("Set-Cookie", "session=fixture")
            )
            server.enqueue(MockResponse().setBody("second"))
            val first = PlaybackHttp.factory(mapOf("Referer" to "https://first.invalid/"))
            val second = PlaybackHttp.factory(mapOf("Referer" to "https://second.invalid/"))
            Assert.assertEquals(
                "first",
                String(
                    read(first.createDataSource(), DataSpec(Uri.parse(server.url("/a").toString())))
                ),
            )
            Assert.assertEquals(
                "second",
                String(
                    read(
                        second.createDataSource(),
                        DataSpec(Uri.parse(server.url("/b").toString())),
                    )
                ),
            )
            val a = server.takeRequest(2, TimeUnit.SECONDS)!!
            val b = server.takeRequest(2, TimeUnit.SECONDS)!!
            Assert.assertEquals(0, a.sequenceNumber)
            Assert.assertEquals(
                "A second reader should reuse the same connection",
                1,
                b.sequenceNumber,
            )
            Assert.assertEquals("https://first.invalid/", a.getHeader("Referer"))
            Assert.assertEquals("https://second.invalid/", b.getHeader("Referer"))
            Assert.assertNull("No implicit browser/session cookies", b.getHeader("Cookie"))
        }
    }

    @Test
    fun pooledSocketFailureRecoversThroughMedia3But429DoesNotRetry() {
        MockWebServer().use { server ->
            server.enqueue(MockResponse().setBody("first"))
            server.enqueue(MockResponse().setSocketPolicy(SocketPolicy.DISCONNECT_AFTER_REQUEST))
            server.enqueue(MockResponse().setBody("recovered"))
            server.enqueue(MockResponse().setResponseCode(429).addHeader("Retry-After", "30"))
            val factory = PlaybackHttp.factory(emptyMap())
            fun fetch(path: String) =
                read(factory.createDataSource(), DataSpec(Uri.parse(server.url(path).toString())))
            Assert.assertEquals("first", String(fetch("/first")))
            Assert.assertEquals("recovered", String(fetch("/next")))
            Assert.assertEquals(3, server.requestCount)
            Assert.assertEquals(0, server.takeRequest(2, TimeUnit.SECONDS)!!.sequenceNumber)
            Assert.assertEquals(1, server.takeRequest(2, TimeUnit.SECONDS)!!.sequenceNumber)
            Assert.assertEquals(0, server.takeRequest(2, TimeUnit.SECONDS)!!.sequenceNumber)
            val error =
                Assert.assertThrows(
                    androidx.media3.datasource.HttpDataSource.InvalidResponseCodeException::class
                        .java
                ) {
                    fetch("/limited")
                }
            Assert.assertEquals(429, error.responseCode)
            Assert.assertNotNull(server.takeRequest(2, TimeUnit.SECONDS))
            Assert.assertNull(
                "Rate limits must not be retried",
                server.takeRequest(200, TimeUnit.MILLISECONDS),
            )
            Assert.assertEquals(4, server.requestCount)
        }
    }

    @Test
    fun aFactoryCreatedBeforeProxyChangeUsesTheNewRoute() {
        val factory = PlaybackHttp.factory(emptyMap())
        MockWebServer().use { proxy ->
            proxy.enqueue(MockResponse().setBody("proxied"))
            NetworkSettings.apply(json("proxy" to "http://${proxy.hostName}:${proxy.port}"))
            val uri = Uri.parse("http://unreachable-origin.invalid/media")
            Assert.assertEquals("proxied", String(read(factory.createDataSource(), DataSpec(uri))))
            val request = proxy.takeRequest(2, TimeUnit.SECONDS)!!
            Assert.assertTrue(
                request.requestLine.startsWith("GET http://unreachable-origin.invalid/media ")
            )
            Assert.assertTrue(NetworkSettings.explicitProxy)
        }
    }

    @Test
    fun cancellingPrefetchInterruptsAStalledSocket() {
        MockWebServer().use { server ->
            server.enqueue(MockResponse().setSocketPolicy(SocketPolicy.NO_RESPONSE))
            val cancellation = PlaybackHttp.Cancellation()
            val executor = Executors.newSingleThreadExecutor()
            try {
                val future =
                    executor.submit<Boolean> {
                        try {
                            read(
                                PlaybackHttp.factory(emptyMap(), cancellation).createDataSource(),
                                DataSpec(Uri.parse(server.url("/stalled").toString())),
                            )
                            false
                        } catch (_: IOException) {
                            true
                        }
                    }
                Assert.assertNotNull(server.takeRequest(2, TimeUnit.SECONDS))
                cancellation.cancel()
                Assert.assertTrue(
                    "Cancel must release I/O without waiting for the 8-second timeout",
                    future.get(2, TimeUnit.SECONDS),
                )
            } finally {
                cancellation.close()
                executor.shutdownNow()
            }
        }
    }

    @Test
    fun pairedTracksWarmConcurrentlyAndForegroundReadsUseTheirBytes() {
        MockWebServer().use { server ->
            val both = CountDownLatch(2)
            val overlap = java.util.concurrent.atomic.AtomicBoolean(true)
            server.dispatcher =
                object : Dispatcher() {
                    override fun dispatch(request: RecordedRequest): MockResponse {
                        both.countDown()
                        if (!both.await(3, TimeUnit.SECONDS)) overlap.set(false)
                        val size = if (request.path == "/audio") 192 * 1024 else 1024 * 1024
                        return MockResponse().setBody(Buffer().write(ByteArray(size) { 37 }))
                    }
                }
            val media = json("url" to server.url("/watch").toString(), "kind" to "video")
            val stream =
                json(
                    "url" to server.url("/video").toString(),
                    "audio_url" to server.url("/audio").toString(),
                )
            PlaybackCache.prefetchResolved(
                media,
                false,
                "best",
                stream,
                NativeBridge.accountIntent(),
                NetworkSettings.revision.get(),
            )
            MediaRuntimeTest.await("Both track byte ranges must be warm") {
                PlaybackCache.readyFor(media, false, "best")
            }
            Assert.assertTrue("Audio must not wait for the entire video prefix", overlap.get())
            Assert.assertEquals(2, server.requestCount)
            Assert.assertNotNull(server.takeRequest(2, TimeUnit.SECONDS))
            Assert.assertNotNull(server.takeRequest(2, TimeUnit.SECONDS))
            for (key in listOf("url", "audio_url")) {
                val bytes =
                    read(
                        PlaybackCache.source(stream).createDataSource(),
                        DataSpec.Builder().setUri(stream.getString(key)).setLength(1024).build(),
                    )
                Assert.assertArrayEquals(ByteArray(1024) { 37 }, bytes)
            }
            Assert.assertNull(
                "Foreground must reuse both tracks' warmed bytes",
                server.takeRequest(200, TimeUnit.MILLISECONDS),
            )
            Assert.assertEquals(2, server.requestCount)
            Assert.assertFalse(
                "A different selected quality is a different preload",
                PlaybackCache.readyFor(media, false, "720"),
            )
            val key = stream.getString("url")
            for (span in PlaybackCache.cache().getCachedSpans(key).toList()) PlaybackCache.cache()
                .removeSpan(span)
            Assert.assertFalse(
                "Evicted bytes must not be advertised ready at the tap",
                PlaybackCache.readyFor(media, false, "best"),
            )
        }
    }

    @Test
    fun indexedTailMetadataIsCachedBeforeBulkPrefix() {
        MockWebServer().use { server ->
            val requests = java.util.Collections.synchronizedList(mutableListOf<String>())
            server.dispatcher =
                object : Dispatcher() {
                    override fun dispatch(request: RecordedRequest): MockResponse {
                        val range = request.getHeader("Range")!!.removePrefix("bytes=")
                        requests.add(range)
                        val start = range.substringBefore('-').toInt()
                        val end = range.substringAfter('-').toInt()
                        return MockResponse()
                            .setResponseCode(206)
                            .addHeader("Content-Range", "bytes $start-$end/4194304")
                            .setBody(Buffer().write(ByteArray(end - start + 1) { 53 }))
                    }
                }
            val url = server.url("/indexed.mp4").toString()
            val manifest =
                """<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="static" mediaPresentationDuration="PT8S"><Period><AdaptationSet contentType="video" mimeType="video/mp4"><Representation id="v" bandwidth="1000000" codecs="avc1.640028" width="320" height="180"><BaseURL>$url</BaseURL><SegmentBase indexRange="2097152-2097279"><Initialization range="0-31"/></SegmentBase></Representation></AdaptationSet></Period></MPD>"""
            val stream = json("url" to url, "dash_manifest" to manifest)
            val media = json("url" to url, "kind" to "video")
            val plan = StartupPrefetch.plan(stream, false).single()
            Assert.assertTrue(plan.ranges.sumOf { it.length } <= 1024L * 1024)
            PlaybackCache.prefetchResolved(
                media,
                false,
                "best",
                stream,
                NativeBridge.accountIntent(),
                NetworkSettings.revision.get(),
            )
            MediaRuntimeTest.await("Declared tail index must be prefetched") {
                PlaybackCache.readyFor(media, false, "best")
            }
            Assert.assertEquals("2097152-2097279", requests[1])
            val count = requests.size
            val bytes =
                read(
                    PlaybackCache.source(stream).createDataSource(),
                    DataSpec.Builder().setUri(url).setPosition(2097152).setLength(128).build(),
                )
            Assert.assertArrayEquals(ByteArray(128) { 53 }, bytes)
            Assert.assertEquals(
                "Index reads should not require another RTT after the tap",
                count,
                requests.size,
            )
        }
    }

    @Test
    fun aResolvedDescriptorFromAnOldAccountOrNetworkCannotStartWork() {
        MockWebServer().use { server ->
            val media = json("url" to server.url("/watch").toString(), "kind" to "video")
            PlaybackCache.prefetchResolved(
                media,
                false,
                "best",
                json("url" to server.url("/private").toString()),
                NativeBridge.accountIntent() - 1,
                NetworkSettings.revision.get(),
            )
            PlaybackCache.prefetchResolved(
                media,
                false,
                "best",
                json("url" to server.url("/old-route").toString()),
                NativeBridge.accountIntent(),
                NetworkSettings.revision.get() - 1,
            )
            Assert.assertFalse(PlaybackCache.readyFor(media, false, "best"))
            Assert.assertNull(server.takeRequest(200, TimeUnit.MILLISECONDS))
        }
    }
}
