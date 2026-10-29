package dev.ytui.android

import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.LinearGradient
import android.graphics.Paint
import android.graphics.Path
import android.graphics.Shader
import androidx.compose.ui.graphics.toPixelMap
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createEmptyComposeRule
import androidx.lifecycle.ViewModelProvider
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.UiDevice
import java.io.ByteArrayOutputStream
import java.io.File
import java.net.ServerSocket
import java.util.concurrent.atomic.AtomicInteger
import org.json.JSONObject
import org.junit.*
import org.junit.runner.RunWith

/**
 * Populated screens use only original generated artwork and loopback HTTP. Never fake live feeds.
 */
@RunWith(AndroidJUnit4::class)
class CatalogUiTest {
    @get:Rule val compose = createEmptyComposeRule()
    @get:Rule
    val notifications =
        androidx.test.rule.GrantPermissionRule.grant(
            *if (android.os.Build.VERSION.SDK_INT >= 33)
                arrayOf("android.permission.POST_NOTIFICATIONS")
            else emptyArray()
        )
    private lateinit var scenario: ActivityScenario<RustActivity>
    private lateinit var original: JSONObject
    private lateinit var covers: CoverServer

    @Before
    fun launch() {
        MediaBridge.execute(json("op" to "stop").toString())
        original =
            JSONObject(NativeBridge.request(json("op" to "settings").toString()))
                .getJSONObject("data")
        NativeBridge.request(
            json(
                    "op" to "save_settings",
                    "config" to
                        original
                            .copyJson()
                            .put("theme", "dark")
                            .put("thumbnails", true)
                            .put("music_player_style", "classic")
                            .put("reduced_motion", true),
                )
                .toString()
        )
        covers = CoverServer()
        scenario = RuntimeTest.launchOffline()
        compose.waitUntil(10_000) {
            compose
                .onAllNodes(
                    hasTestTag("app-background") and
                        SemanticsMatcher.expectValue(SemanticsProperties.StateDescription, "Ready")
                )
                .fetchSemanticsNodes()
                .isNotEmpty()
        }
    }

    @After
    fun close() {
        MediaBridge.execute(json("op" to "stop").toString())
        scenario.close()
        covers.close()
        NativeBridge.request(json("op" to "save_settings", "config" to original).toString())
        MediaBridge.execute(json("op" to "settings", "config" to original).toString())
        UiDevice.getInstance(InstrumentationRegistry.getInstrumentation()).apply {
            executeShellCommand("wm size reset")
            executeShellCommand("wm density reset")
            setOrientationNatural()
            unfreezeRotation()
        }
    }

    private fun populate(tab: Int): List<MediaCard> {
        val titles =
            if (tab == 2)
                listOf(
                    "After the sunset",
                    "Midnight bloom",
                    "Somewhere, slowly",
                    "Golden hour",
                    "Open water",
                    "Blue room",
                )
            else
                listOf(
                    "A little further from ordinary",
                    "The art of slowing down",
                    "A city that never sleeps",
                    "Small worlds, big ideas",
                    "Taking the scenic route",
                    "Made of light",
                )
        val cards =
            titles.mapIndexed { index, title ->
                MediaCard(
                    json(
                        "title" to title,
                        "creator" to if (tab == 2) "Studio Sessions" else "The Daily Frame",
                        "url" to "https://media.invalid/ui-$tab-$index",
                        "kind" to if (tab == 2) "song" else "video",
                        "thumbnail" to covers.url(index),
                        "duration" to 221 + index * 47,
                        "section" to if (tab == 2) "Made for your day" else "Picked for you",
                    )
                )
            }
        scenario.onActivity {
            val vm = ViewModelProvider(it)[YtuiViewModel::class.java]
            vm.chooseTab(tab)
            vm.feeds[tab].value =
                FeedState(
                    title = if (tab == 2) "Listen again" else "For you",
                    cards = cards,
                    categories =
                        listOf(
                            json("title" to "All", "request" to "Home"),
                            json("title" to "Discover", "request" to "Home"),
                        ),
                    generation = 100,
                )
        }
        compose.waitUntil(15_000) {
            compose
                .onAllNodesWithTag("artwork-${cards.first().key}", useUnmergedTree = true)
                .fetchSemanticsNodes()
                .isNotEmpty()
        }
        compose.waitForIdle()
        // Validate a rendered warm cover pixel, not merely an Image semantics node.
        compose.waitUntil(10_000) {
            val bitmap =
                compose
                    .onNodeWithTag("artwork-${cards.first().key}", useUnmergedTree = true)
                    .captureToImage()
            val color = bitmap.toPixelMap()[bitmap.width / 2, bitmap.height / 4]
            color.red > color.green * 1.15f
        }
        InstrumentationRegistry.getInstrumentation().uiAutomation.waitForIdle(150, 3000)
        return cards
    }

    @Test
    fun artworkGridAdaptsAndCardActionsStayReachable() {
        val cards = populate(1)
        compose.onNodeWithTag("bottom-navigation").assertIsDisplayed()
        compose.onNodeWithTag("media-card-${cards[0].key}").assertIsDisplayed()
        compose.onAllNodesWithText("Preview").assertCountEquals(0)
        Assert.assertTrue("Real thumbnail HTTP/decode must run", covers.requests.get() > 0)
        MediaRuntimeTest.capture("native-catalog.png")
        compose.onNodeWithTag("media-menu-${cards[0].key}").performClick()
        for (label in
            listOf("Play now", "Preview", "Add to queue", "Save to library", "Download")) compose
            .onNodeWithText(label)
            .assertIsDisplayed()
        MediaRuntimeTest.capture("native-card-menu.png")
        UiDevice.getInstance(InstrumentationRegistry.getInstrumentation()).pressBack()
        compose.waitForIdle()
        compose.onAllNodesWithText("Play now").assertCountEquals(0)
        // A tablet cold start avoids photographing old popup surfaces while wm
        // applies two successive physical-size/density overrides. The separate
        // NativeUiTest still checks live activity rotation and panel navigation.
        scenario.close()
        val device = UiDevice.getInstance(InstrumentationRegistry.getInstrumentation())
        device.setOrientationNatural()
        device.executeShellCommand("wm size 2560x1600")
        device.executeShellCommand("wm density 320")
        scenario = RuntimeTest.launchOffline()
        compose.waitUntil(10_000) {
            compose
                .onAllNodes(
                    hasTestTag("app-background") and
                        SemanticsMatcher.expectValue(SemanticsProperties.StateDescription, "Ready")
                )
                .fetchSemanticsNodes()
                .isNotEmpty()
        }
        populate(1)
        compose.waitUntil(10_000) {
            compose.onAllNodesWithTag("navigation-rail").fetchSemanticsNodes().isNotEmpty()
        }
        compose.onNodeWithTag("navigation-rail").assertIsDisplayed()
        val first =
            compose.onNodeWithTag("media-card-${cards[0].key}").fetchSemanticsNode().boundsInRoot
        val second =
            compose.onNodeWithTag("media-card-${cards[1].key}").fetchSemanticsNode().boundsInRoot
        Assert.assertEquals("Wide layout should show adjacent cards", first.top, second.top, 3f)
        Assert.assertTrue(second.left > first.left)
        compose.onNodeWithTag("settings-button").assertIsDisplayed()
        MediaRuntimeTest.capture("native-wide-catalog.png")
    }

    @Test
    fun musicArtworkIsSquareAndSettingsRemainNative() {
        val cards = populate(2)
        val cover =
            compose
                .onNodeWithTag("artwork-${cards[0].key}", useUnmergedTree = true)
                .fetchSemanticsNode()
                .boundsInRoot
        Assert.assertEquals(cover.width, cover.height, 3f)
        Assert.assertTrue(
            "Two music covers should fit across a phone",
            compose
                .onNodeWithTag("media-card-${cards[1].key}")
                .fetchSemanticsNode()
                .boundsInRoot
                .left > 0,
        )
        MediaRuntimeTest.capture("native-music.png")
        compose.onNodeWithTag("settings-button").performClick()
        compose.onNodeWithText("Appearance").assertIsDisplayed()
        compose.onNodeWithTag("theme-selector").assertIsDisplayed()
        MediaRuntimeTest.capture("native-settings.png")
        compose.onNodeWithText("Playback & downloads").performScrollTo().assertIsDisplayed()
        compose
            .onNodeWithTag("proxy-field")
            .performScrollTo()
            .performTextInput("editable-native-field")
        compose.onNodeWithTag("proxy-field").assertTextContains("editable-native-field")
    }

    @Test
    fun miniPlayerAndLargeTransportControlTheActualDecoder() {
        val cards = populate(2)
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val fixture = File(instrumentation.targetContext.cacheDir, "ui-player-fixture.m4a")
        instrumentation.context.assets.open("fixture-audio.m4a").use { input ->
            fixture.outputStream().use { input.copyTo(it) }
        }
        val request =
            json(
                "media" to cards[0].value,
                "audio" to true,
                "stream" to json("url" to android.net.Uri.fromFile(fixture).toString()),
            )
        MediaBridge.execute(json("op" to "play", "request" to request).toString())
        MediaRuntimeTest.await("UI fixture must play") {
            MediaRuntimeTest.status().optBoolean("playing")
        }
        try {
            MediaBridge.main.post {
                PlaybackService.current?.player?.repeatMode =
                    androidx.media3.common.Player.REPEAT_MODE_ONE
            }
            compose.waitUntil(10_000) {
                compose.onAllNodesWithTag("mini-player").fetchSemanticsNodes().isNotEmpty()
            }
            compose
                .onNodeWithTag("dock-pause")
                .assertHeightIsAtLeast(androidx.compose.ui.unit.Dp(48f))
                .performClick()
            MediaRuntimeTest.await("Mini-player pause must reach Media3") {
                MediaRuntimeTest.status().optBoolean("paused")
            }
            compose.onNodeWithTag("mini-player").performClick()
            compose.onNodeWithTag("music-visualizer").assertIsDisplayed()
            MediaRuntimeTest.capture("native-player.png")
            compose.onNodeWithTag("player-pause").performScrollTo().performClick()
            MediaRuntimeTest.await("Large transport must resume Media3") {
                MediaRuntimeTest.status().optBoolean("playing")
            }
            compose.onNodeWithTag("volume-slider").performScrollTo().assertIsDisplayed()
            compose.onNodeWithText("Audio & subtitles").performScrollTo().performClick()
            compose.onNodeWithText("Toggle captions").performScrollTo().assertIsDisplayed()
            compose.onNodeWithText("Close").performClick()
            MediaBridge.execute(json("op" to "stop").toString())
            compose.waitUntil(10_000) {
                compose.onAllNodesWithTag("mini-player").fetchSemanticsNodes().isEmpty()
            }
            compose.onNodeWithTag("bottom-navigation").assertIsDisplayed()
        } finally {
            MediaBridge.main.post {
                PlaybackService.current?.player?.repeatMode =
                    androidx.media3.common.Player.REPEAT_MODE_OFF
            }
            MediaBridge.execute(json("op" to "stop").toString())
            fixture.delete()
        }
    }

    private class CoverServer : AutoCloseable {
        private val socket = ServerSocket(0, 16, java.net.InetAddress.getByName("127.0.0.1"))
        val requests = AtomicInteger()
        private val images = (0..5).map(::cover)
        private val worker =
            Thread {
                    while (!socket.isClosed) try {
                        socket.accept().use { client ->
                            client.soTimeout = 5000
                            val reader = client.getInputStream().bufferedReader()
                            val request = reader.readLine() ?: return@use
                            while (!reader.readLine().isNullOrEmpty()) {
                                /* consume headers */
                            }
                            val index =
                                Regex("/cover-(\\d+)")
                                    .find(request)
                                    ?.groupValues
                                    ?.get(1)
                                    ?.toIntOrNull() ?: 0
                            val bytes = images[index.coerceIn(0, 5)]
                            requests.incrementAndGet()
                            client.getOutputStream().apply {
                                write(
                                    "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: ${bytes.size}\r\nConnection: close\r\n\r\n"
                                        .toByteArray()
                                )
                                write(bytes)
                                flush()
                            }
                        }
                    } catch (_: java.io.IOException) {
                        /* closed fixture client/server */
                    }
                }
                .apply {
                    isDaemon = true
                    start()
                }

        fun url(index: Int) = "http://127.0.0.1:${socket.localPort}/cover-$index.png"

        override fun close() {
            socket.close()
            worker.join(2000)
        }

        companion object {
            private fun cover(index: Int): ByteArray {
                val bitmap = Bitmap.createBitmap(960, 640, Bitmap.Config.ARGB_8888)
                val canvas = Canvas(bitmap)
                val pairs =
                    listOf(
                        0xffe7aa75 to 0xff4b275c,
                        0xff559c9d to 0xff102f4d,
                        0xffb49cd9 to 0xff322450,
                        0xffeaa46b to 0xff5d4866,
                        0xff7399b9 to 0xff173e58,
                        0xffe08c9c to 0xff483b79,
                    )
                val (top, bottom) = pairs[index]
                val paint = Paint(Paint.ANTI_ALIAS_FLAG)
                paint.shader =
                    LinearGradient(
                        0f,
                        0f,
                        960f,
                        640f,
                        top.toInt(),
                        bottom.toInt(),
                        Shader.TileMode.CLAMP,
                    )
                canvas.drawRect(0f, 0f, 960f, 640f, paint)
                paint.shader = null
                paint.color = 0xfffce8c3.toInt()
                canvas.drawCircle(670f - index * 35, 190f, 76f, paint)
                paint.color = bottom.toInt()
                val hills =
                    Path().apply {
                        moveTo(0f, 460f)
                        cubicTo(220f, 210f, 370f, 560f, 600f, 350f)
                        cubicTo(760f, 210f, 850f, 410f, 960f, 310f)
                        lineTo(960f, 640f)
                        lineTo(0f, 640f)
                        close()
                    }
                canvas.drawPath(hills, paint)
                paint.color = Color.WHITE
                paint.alpha = 200
                paint.textSize = 28f
                paint.typeface =
                    android.graphics.Typeface.create(
                        "sans-serif-medium",
                        android.graphics.Typeface.NORMAL,
                    )
                canvas.drawText("STUDIO / ${index + 1}", 48f, 580f, paint)
                return ByteArrayOutputStream().use {
                    bitmap.compress(Bitmap.CompressFormat.PNG, 100, it)
                    bitmap.recycle()
                    it.toByteArray()
                }
            }
        }
    }
}
