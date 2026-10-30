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
@OptIn(androidx.compose.ui.test.ExperimentalTestApi::class)
class CatalogUiTest {
    @get:Rule
    val compose = createEmptyComposeRule(effectContext = kotlinx.coroutines.Dispatchers.Main)
    @get:Rule
    val notifications =
        androidx.test.rule.GrantPermissionRule.grant(
            *if (android.os.Build.VERSION.SDK_INT >= 33)
                arrayOf("android.permission.POST_NOTIFICATIONS")
            else emptyArray()
        )
    private lateinit var scenario: ActivityScenario<RustActivity>
    private lateinit var original: JSONObject
    private lateinit var originalMobile: JSONObject
    private lateinit var covers: CoverServer

    @Before
    fun launch() {
        originalMobile = MobileSettings.snapshot()
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
        MobileSettings.save(originalMobile)
        NativeBridge.request(json("op" to "save_settings", "config" to original).toString())
        MediaBridge.execute(json("op" to "settings", "config" to original).toString())
        UiDevice.getInstance(InstrumentationRegistry.getInstrumentation()).apply {
            executeShellCommand("wm size reset")
            executeShellCommand("wm density reset")
            executeShellCommand("cmd uimode night auto")
            executeShellCommand("settings put system font_scale 1.0")
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
                    "A little further from ordinary: a long video title with enough words to exercise the full two-line feed layout",
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
                        "views_text" to "1.2K views",
                        "published_text" to "2 days ago",
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
        try {
            compose.waitUntil(15_000) {
                compose
                    .onAllNodesWithTag("artwork-${cards.first().key}", useUnmergedTree = true)
                    .fetchSemanticsNodes()
                    .isNotEmpty()
            }
        } catch (error: Exception) {
            captureSettled("native-catalog-error.png")
            scenario.onActivity {
                val vm = ViewModelProvider(it)[YtuiViewModel::class.java]
                android.util.Log.i(
                    "CatalogEvidence",
                    "tab=${vm.tab.value} cards=${vm.feeds[tab].value.cards.size} ready=${vm.ready.value} thumbnails=${vm.config.value.optBoolean("thumbnails")} requests=${covers.requests.get()} width=${it.resources.configuration.screenWidthDp} height=${it.resources.configuration.screenHeightDp}",
                )
            }
            throw error
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
        var widthDp = 0
        scenario.onActivity { widthDp = it.resources.configuration.screenWidthDp }
        val visible = if (widthDp >= 840) cards.size else if (tab == 2) 4 else 1
        awaitDecodedCovers(cards.take(visible))
        InstrumentationRegistry.getInstrumentation().uiAutomation.waitForIdle(150, 3000)
        return cards
    }

    @Test
    fun artworkGridAdaptsAndCardActionsStayReachable() {
        covers.failFirstCover = true
        val cards = populate(1)
        compose.onNodeWithTag("bottom-navigation").assertIsDisplayed()
        compose.onNodeWithTag("media-card-${cards[0].key}").assertIsDisplayed()
        compose.onAllNodesWithText("Preview").assertCountEquals(0)
        Assert.assertTrue("Real thumbnail HTTP/decode must run", covers.requests.get() > 0)
        Assert.assertEquals(
            "The first cover's transient failure must recover",
            1,
            covers.failures.get(),
        )
        val pane = compose.onNodeWithTag("browse-pane").fetchSemanticsNode().boundsInRoot
        val cover =
            compose
                .onNodeWithTag("artwork-${cards[0].key}", useUnmergedTree = true)
                .fetchSemanticsNode()
                .boundsInRoot
        Assert.assertEquals("Video art must be full bleed", pane.left, cover.left, 2f)
        Assert.assertEquals("Video art must fill the pane", pane.width, cover.width, 2f)
        Assert.assertEquals("Video art remains 16:9", 16f / 9f, cover.width / cover.height, 0.01f)
        val source = compose.onNodeWithTag("source-Home").fetchSemanticsNode().boundsInRoot
        // Categories and source filters share one lazy row, not two independently styled strips.
        compose.onNodeWithTag("feed-filters").performScrollToNode(hasTestTag("category-All"))
        val category = compose.onNodeWithTag("category-All").fetchSemanticsNode().boundsInRoot
        Assert.assertEquals(source.top, category.top, 2f)
        compose.onNodeWithTag("feed-filters").performScrollToNode(hasTestTag("source-Home"))
        captureSettled("native-catalog.png")
        compose.onNodeWithTag("media-menu-${cards[0].key}").performClick()
        for (label in
            listOf("Play now", "Preview", "Add to queue", "Save to library", "Download")) compose
            .onNodeWithText(label)
            .assertIsDisplayed()
        captureSettled("native-card-menu.png")
        compose.onNodeWithText("Add to queue").performClick()
        compose.waitUntil(10_000) {
            var contains = false
            scenario.onActivity {
                contains =
                    ViewModelProvider(it)[YtuiViewModel::class.java].queue.value.any {
                        it.url == cards[0].url
                    }
            }
            contains
        }
        MediaBridge.event("notice", "Playback added")
        compose.waitForIdle()
        compose.onAllNodesWithText("Added to queue").assertCountEquals(0)
        compose.onAllNodesWithText("Playback added").assertCountEquals(0)
        compose.onNodeWithTag("bottom-navigation").assertIsDisplayed()
        val queued = PlaybackService.rpcArray(json("op" to "queue")).objects()
        val index = queued.indexOfFirst { it.optString("url") == cards[0].url }
        if (index >= 0)
            PlaybackService.rpc(
                json("op" to "queue_remove", "index" to index, "url" to cards[0].url)
            )
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
        captureSettled("native-wide-catalog.png")
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
        captureSettled("native-music.png")
        compose.onNodeWithTag("settings-button").performClick()
        compose.onNodeWithTag("settings-Appearance").assertIsDisplayed().performClick()
        compose.onNodeWithTag("theme-selector").assertIsDisplayed()
        captureSettled("native-settings.png")
        compose.onNodeWithTag("settings-back").performClick()
        compose.onNodeWithTag("settings-Network").performClick()
        compose
            .onNodeWithTag("proxy-field")
            .performScrollTo()
            .performTextInput("editable-native-field")
        compose.onNodeWithTag("proxy-field").assertTextContains("editable-native-field")
    }

    @Test
    fun offlinePlaylistsUseEditableNativeFieldsAndRealRustPersistence() {
        compose.onNodeWithTag("tab-0").performClick()
        compose.onNodeWithTag("create-offline-playlist").performClick()
        val title = "UI fixture " + java.util.UUID.randomUUID().toString().take(8)
        compose.onNodeWithTag("offline-playlist-name").performTextInput(title)
        compose.onNodeWithText("Create").performClick()
        compose.waitUntil(10_000) {
            compose.onAllNodesWithText(title).fetchSemanticsNodes().isNotEmpty()
        }
        compose.onNodeWithText(title).performClick()
        compose.onNodeWithTag("offline-playlist-edit-name").performTextReplacement("$title renamed")
        compose.onNodeWithText("Rename").performClick()
        compose.waitUntil(10_000) {
            PlaybackService.rpcArray(json("op" to "offline_playlists")).objects().any {
                it.optString("title") == "$title renamed"
            }
        }
        compose.onNodeWithText("Delete playlist").performClick()
        compose.onNodeWithText("Delete").performClick()
        compose.waitUntil(10_000) {
            PlaybackService.rpcArray(json("op" to "offline_playlists")).objects().none {
                it.optString("title").startsWith(title)
            }
        }
        compose.onAllNodesWithText("$title renamed").assertCountEquals(0)
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
            try {
                // Await the measured player layout and decoded artwork.
                compose.waitUntil(10_000) {
                    compose.onNodeWithTag("music-visualizer").isDisplayed()
                }
            } finally {
                captureSettled("native-player.png")
                android.util.Log.i(
                    "PlayerLayoutTest",
                    compose.onNodeWithTag("focused-player", useUnmergedTree = true).printToString(),
                )
            }
            compose
                .onNodeWithTag("music-visualizer")
                .assertIsDisplayed()
                .assertHeightIsAtLeast(androidx.compose.ui.unit.Dp(160f))
            val focusedCover =
                hasTestTag("artwork-${cards[0].key}") and
                    hasAnyAncestor(hasTestTag("focused-player"))
            // Vinyl deliberately draws the same art on the sleeve and record label.
            fun loadedCoverVisible(): Boolean {
                val nodes = compose.onAllNodes(focusedCover, useUnmergedTree = true)
                return nodes.fetchSemanticsNodes().indices.any { nodes[it].isDisplayed() }
            }
            compose.waitUntil(10_000) { loadedCoverVisible() }
            captureSettled("native-player.png")
            compose.onNodeWithTag("seek-slider").performTouchInput {
                swipe(
                    androidx.compose.ui.geometry.Offset(width * 0.2f, height / 2f),
                    androidx.compose.ui.geometry.Offset(width * 0.5f, height / 2f),
                    250,
                )
            }
            MediaRuntimeTest.await(
                "Restyled seek control must commit to the actual paused decoder"
            ) {
                val position = MediaRuntimeTest.status().optDouble("position")
                position > 2.0 && position < 6.0
            }
            for (style in listOf("vinyl", "vu", "orbit")) {
                scenario.onActivity {
                    ViewModelProvider(it)[YtuiViewModel::class.java].updateConfig(false) { config ->
                        config.put("music_player_style", style)
                    }
                }
                compose.waitUntil(10_000) {
                    var selected = false
                    scenario.onActivity {
                        selected =
                            ViewModelProvider(it)[YtuiViewModel::class.java]
                                .config
                                .value
                                .optString("music_player_style") == style
                    }
                    selected && loadedCoverVisible()
                }
                compose
                    .onNodeWithTag("music-visualizer")
                    .assertIsDisplayed()
                    .assertHeightIsAtLeast(androidx.compose.ui.unit.Dp(160f))
                compose.onNodeWithTag("player-pause").assertIsDisplayed()
                if (style == "vinyl") {
                    val record =
                        compose.onNodeWithTag("music-visualizer").fetchSemanticsNode().boundsInRoot
                    val label =
                        compose
                            .onNodeWithTag("vinyl-record-label")
                            .fetchSemanticsNode()
                            .boundsInRoot
                    Assert.assertEquals(
                        "Record art must stay at its hub",
                        record.center.y,
                        label.center.y,
                        3f,
                    )
                    Assert.assertEquals(record.left + record.width * 0.65f, label.center.x, 3f)
                }
                captureSettled("native-player-$style.png")
            }
            compose.onNodeWithTag("player-pause").assertIsDisplayed().performClick()
            MediaRuntimeTest.await("Large transport must resume Media3") {
                MediaRuntimeTest.status().optBoolean("playing")
            }
            compose.onNodeWithTag("player-options").performClick()
            compose.onNodeWithTag("volume-slider").performScrollTo().assertIsDisplayed()
            compose.onNodeWithText("Audio & subtitles").performScrollTo().performClick()
            compose.onNodeWithText("Toggle captions").performScrollTo().assertIsDisplayed()
            compose.onNodeWithContentDescription("Close").performClick()
            scenario.onActivity {
                ViewModelProvider(it)[YtuiViewModel::class.java].updateMobile("gestures", false)
            }
            compose.waitUntil(10_000) {
                var disabled = false
                scenario.onActivity {
                    disabled =
                        !ViewModelProvider(it)[YtuiViewModel::class.java]
                            .mobile
                            .value
                            .optBoolean("gestures", true)
                }
                disabled
            }
            compose.onNodeWithTag("panel-header").performTouchInput {
                swipe(center, center + androidx.compose.ui.geometry.Offset(0f, height * 3f), 300)
            }
            compose.onNodeWithTag("focused-player").assertIsDisplayed()
            compose.onNodeWithContentDescription("Minimize player").performClick()
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

    @Test
    fun restyleThemeMatrixCoversPhoneTabletLightAndDark() {
        val device = UiDevice.getInstance(InstrumentationRegistry.getInstrumentation())
        val themes =
            JSONObject(NativeBridge.request(json("op" to "themes").toString())).getJSONArray("data")
        for (wide in listOf(false, true)) for (dark in listOf(false, true)) {
            scenario.close()
            device.setOrientationNatural()
            device.executeShellCommand(if (wide) "wm size 2560x1600" else "wm size 1080x2400")
            device.executeShellCommand(if (wide) "wm density 320" else "wm density 420")
            device.executeShellCommand(if (dark) "cmd uimode night yes" else "cmd uimode night no")
            scenario = RuntimeTest.launchOffline()
            compose.waitUntil(10_000) {
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
            for (index in 0 until themes.length()) {
                val theme = themes.getString(index)
                scenario.onActivity {
                    ViewModelProvider(it)[YtuiViewModel::class.java].updateConfig(dark) { config ->
                        config.put("theme", theme)
                    }
                }
                compose.waitUntil(10_000) {
                    var applied = false
                    scenario.onActivity {
                        applied =
                            ViewModelProvider(it)[YtuiViewModel::class.java]
                                .config
                                .value
                                .optString("theme") == theme
                    }
                    applied
                }
                val palette =
                    JSONObject(
                            NativeBridge.request(json("op" to "theme", "dark" to dark).toString())
                        )
                        .getJSONObject("data")
                val background =
                    palette.color("background", androidx.compose.ui.graphics.Color.Black)
                val surface = palette.color("surface", androidx.compose.ui.graphics.Color.Black)
                val ink = palette.color("text", androidx.compose.ui.graphics.Color.White)
                Assert.assertTrue(
                    "$theme body/background contrast",
                    mediaContrast(ink, background) >= 4.5f,
                )
                Assert.assertTrue(
                    "$theme body/surface contrast",
                    mediaContrast(ink, surface) >= 4.5f,
                )
                val secondary =
                    mediaReadableText(
                        androidx.compose.ui.graphics.lerp(background, ink, 0.7f),
                        surface,
                    )
                Assert.assertTrue(
                    "$theme secondary contrast",
                    mediaContrast(secondary, surface) >= 4.5f,
                )
                val prefix =
                    "restyle-${if (wide) "tablet" else "phone"}-${if (dark) "dark" else "light"}-$theme"
                populate(1)
                captureSettled("$prefix-feed.png")
                populate(2)
                captureSettled("$prefix-music.png")
                compose.onNodeWithTag("settings-button").performClick()
                compose.onNodeWithTag("settings-Appearance").performClick()
                compose.onNodeWithTag("theme-selector").assertIsDisplayed()
                captureSettled("$prefix-settings.png")
                compose.onNodeWithContentDescription("Close").performClick()
            }
        }
    }

    @Test
    fun largeTextKeepsSettingsSearchAndCardMenuReachable() {
        val device = UiDevice.getInstance(InstrumentationRegistry.getInstrumentation())
        scenario.close()
        device.executeShellCommand("settings put system font_scale 2.0")
        try {
            scenario = RuntimeTest.launchOffline()
            compose.waitUntil(10_000) {
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
            val cards = populate(1)
            captureSettled("restyle-large-text-feed.png")
            compose
                .onNodeWithTag("search-field")
                .assertIsDisplayed()
                .performTextInput("large input")
            compose.onNodeWithContentDescription("Clear search").performClick()
            compose.onNodeWithTag("search-field").assertTextContains("Search or paste a link")
            device.pressBack()
            compose.onNodeWithTag("media-menu-${cards[0].key}").performClick()
            compose.onNodeWithText("Download").performScrollTo().assertIsDisplayed()
            captureSettled("restyle-large-text-card-menu.png")
            compose.onNodeWithContentDescription("Close").performClick()
            compose.onNodeWithTag("settings-button").performClick()
            compose.onNodeWithTag("settings-Appearance").performClick()
            compose.onNodeWithTag("theme-selector").assertIsDisplayed().performClick()
            compose.onNodeWithTag("theme-ytui-light").performScrollTo().assertIsDisplayed()
            captureSettled("restyle-large-text-theme-dialog.png")
            compose.onNodeWithText("CANCEL").performClick()
            captureSettled("restyle-large-text-settings.png")
        } finally {
            device.executeShellCommand("settings put system font_scale 1.0")
        }
    }

    @Test
    fun tabletSearchUsesRowsWithoutChangingCardActions() {
        val device = UiDevice.getInstance(InstrumentationRegistry.getInstrumentation())
        scenario.close()
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
        val cards = populate(1)
        scenario.onActivity {
            val vm = ViewModelProvider(it)[YtuiViewModel::class.java]
            vm.feeds[1].value =
                vm.feeds[1].value.copy(request = json("Search" to "synthetic"), generation = 200)
        }
        compose.waitForIdle()
        val first =
            compose.onNodeWithTag("media-card-${cards[0].key}").fetchSemanticsNode().boundsInRoot
        val second =
            compose.onNodeWithTag("media-card-${cards[1].key}").fetchSemanticsNode().boundsInRoot
        Assert.assertTrue("Search results must stack as wide rows", second.top >= first.bottom)
        // Switching from grid to a quarter-width row changes the requested image size.
        // Compose idleness does not await its IO decode; require the real image before measuring.
        compose.waitUntil(10_000) {
            compose
                .onAllNodesWithTag("artwork-${cards[0].key}", useUnmergedTree = true)
                .fetchSemanticsNodes()
                .isNotEmpty()
        }
        val cover =
            compose
                .onNodeWithTag("artwork-${cards[0].key}", useUnmergedTree = true)
                .fetchSemanticsNode()
                .boundsInRoot
        Assert.assertEquals(0.25f, cover.width / (first.width - 32f * 2f), 0.03f)
        InstrumentationRegistry.getInstrumentation().uiAutomation.waitForIdle(300, 3000)
        awaitDecodedCovers(cards)
        captureSettled("restyle-tablet-search.png")
        compose.onNodeWithTag("media-menu-${cards[0].key}").performClick()
        compose.onNodeWithText("Preview").assertIsDisplayed()
    }

    private fun awaitDecodedCovers(cards: List<MediaCard>) {
        compose.waitUntil(15_000) {
            cards.all { media ->
                runCatching {
                        val node =
                            compose.onNodeWithTag("artwork-${media.key}", useUnmergedTree = true)
                        val frame = node.captureToImage()
                        // All original fixture covers have saturated warm/cool artwork;
                        // a neutral placeholder cannot satisfy this actual-pixel check.
                        val c = frame.toPixelMap()[frame.width / 2, frame.height / 4]
                        maxOf(c.red, c.green, c.blue) - minOf(c.red, c.green, c.blue) > 0.12f
                    }
                    .getOrDefault(false)
            }
        }
    }

    private fun captureSettled(filename: String) {
        compose.waitForIdle()
        // A semantics node can exist one transaction before its new window pixels.
        // Wait for real UI idleness before taking review screenshots, not a playback deadline.
        InstrumentationRegistry.getInstrumentation().uiAutomation.waitForIdle(300, 3000)
        MediaRuntimeTest.capture(filename)
    }

    private class CoverServer : AutoCloseable {
        private val socket = ServerSocket(0, 16, java.net.InetAddress.getByName("127.0.0.1"))
        val requests = AtomicInteger()
        val failures = AtomicInteger()
        @Volatile var failFirstCover = false
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
                            if (index == 0 && failFirstCover) {
                                failFirstCover = false
                                failures.incrementAndGet()
                                client
                                    .getOutputStream()
                                    .write(
                                        "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                                            .toByteArray()
                                    )
                                return@use
                            }
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
