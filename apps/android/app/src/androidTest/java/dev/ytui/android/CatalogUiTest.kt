package dev.ytui.android

import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.LinearGradient
import android.graphics.Paint
import android.graphics.Path
import android.graphics.Shader
import androidx.compose.ui.graphics.toPixelMap
import androidx.compose.ui.semantics.SemanticsActions
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
                    .onAllNodes(
                        hasTestTag("artwork-${cards.first().key}") and
                            hasAnyAncestor(hasTestTag("media-card-${cards.first().key}")),
                        useUnmergedTree = true,
                    )
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
                    .onNode(
                        hasTestTag("artwork-${cards.first().key}") and
                            hasAnyAncestor(hasTestTag("media-card-${cards.first().key}")),
                        useUnmergedTree = true,
                    )
                    .captureToImage()
            val color = bitmap.toPixelMap()[bitmap.width / 2, bitmap.height / 4]
            color.red > color.green * 1.15f
        }
        awaitDecodedCovers(cards)
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
        try {
            compose.onNodeWithTag("proxy-field").assertTextContains("editable-native-field")
        } catch (failure: AssertionError) {
            compose.onRoot(useUnmergedTree = true).printToLog("ytui-settings-ui")
            MediaRuntimeTest.capture("native-catalog-error.png")
            throw failure
        }
    }

    @Test
    fun offlinePlaylistsUseEditableNativeFieldsAndRealRustPersistence() {
        compose.onNodeWithTag("tab-0").performClick()
        compose.onNodeWithTag("create-offline-playlist").performClick()
        val title = "UI fixture " + java.util.UUID.randomUUID().toString().take(8)
        compose.onNodeWithTag("offline-playlist-name").performTextInput(title)
        compose.onNodeWithTag("offline-playlist-name").assertTextContains(title)
        settleUi()
        compose.onNodeWithText("Create").assertIsDisplayed().performClick()
        compose.waitUntil(10_000) {
            compose.onAllNodesWithText(title).fetchSemanticsNodes().isNotEmpty()
        }
        settleUi()
        compose.onNodeWithText(title).performClick()
        compose.onNodeWithTag("offline-playlist-edit-name").performTextReplacement("$title renamed")
        compose.onNodeWithTag("offline-playlist-edit-name").assertTextContains("$title renamed")
        // Complete editing before tapping a moving modal. UIA idleness alone
        // does not establish that its IME-driven window relocation has finished.
        val automation = InstrumentationRegistry.getInstrumentation().uiAutomation
        if (
            automation.windows.any {
                it.type == android.view.accessibility.AccessibilityWindowInfo.TYPE_INPUT_METHOD
            }
        ) {
            UiDevice.getInstance(InstrumentationRegistry.getInstrumentation()).pressBack()
            compose.waitUntil(10_000) {
                automation.windows.none {
                    it.type == android.view.accessibility.AccessibilityWindowInfo.TYPE_INPUT_METHOD
                }
            }
        }
        compose.onNodeWithTag("offline-playlist-edit-name").assertIsDisplayed()
        settleUi()
        compose.onNodeWithText("Rename").assertIsDisplayed().performClick()
        try {
            compose.waitUntil(10_000) {
                PlaybackService.rpcArray(json("op" to "offline_playlists")).objects().any {
                    it.optString("title") == "$title renamed"
                }
            }
        } catch (error: Throwable) {
            runCatching { captureSettled("restyle-offline-playlist-rename-error.png") }
            throw error
        }
        settleUi()
        compose.onNodeWithText("Delete playlist").performClick()
        settleUi()
        compose.onNodeWithText("Delete").assertIsDisplayed().performClick()
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
            captureSettled("restyle-mini-player.png")
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
            compose.waitUntil(10_000) { MediaRuntimeTest.status().optBoolean("repeat_one") }
            compose.onNodeWithTag("player-repeat").assertIsSelected().performClick()
            MediaRuntimeTest.await("Repeat UI must reflect the existing Media3 command") {
                !MediaRuntimeTest.status().optBoolean("repeat_one")
            }
            compose.onNodeWithTag("player-repeat").assertIsNotSelected().performClick()
            MediaRuntimeTest.await("Repeat may be restored through the real UI") {
                MediaRuntimeTest.status().optBoolean("repeat_one")
            }
            compose.onNodeWithTag("player-shuffle").assertIsNotSelected().performClick()
            MediaRuntimeTest.await("Shuffle UI must reflect the existing Media3 command") {
                MediaRuntimeTest.status().optBoolean("shuffle")
            }
            compose.onNodeWithTag("player-shuffle").assertIsSelected()
            captureSettled("restyle-player-active-modes.png")
            compose.onNodeWithTag("player-shuffle").performClick()
            MediaRuntimeTest.await("Restore the original shuffle mode") {
                !MediaRuntimeTest.status().optBoolean("shuffle")
            }
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
            captureSettled("restyle-player-options.png")
            compose.onNodeWithText("Audio & subtitles").performScrollTo().performClick()
            compose.onNodeWithText("Toggle captions").performScrollTo().assertIsDisplayed()
            captureSettled("restyle-player-options-tracks.png")
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
            assertNavigationLabelsFit()
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
            // The fixture deliberately stores the legacy "dark" alias.
            compose.onNodeWithTag("theme-ytui-dark").performScrollTo().assertIsSelected()
            compose.onNodeWithTag("theme-ytui-light").performScrollTo().assertIsDisplayed()
            captureSettled("restyle-large-text-theme-dialog.png")
            compose.onNodeWithText("CANCEL").performClick()
            captureSettled("restyle-large-text-settings.png")
            compose.onNodeWithContentDescription("Close").performClick()
            scenario.close()
            device.executeShellCommand("wm size 2560x1600")
            device.executeShellCommand("wm density 320")
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
            val tabletCards = populate(1)
            captureSettled("restyle-fidelity-large-text-tablet-feed.png")
            assertNavigationLabelsFit()
            compose.onNodeWithTag("navigation-rail").assertIsDisplayed()
            awaitDecodedCovers(tabletCards)
            captureSettled("restyle-fidelity-large-text-tablet-feed.png")
            compose.onNodeWithTag("settings-button").performClick()
            compose.onNodeWithTag("settings-Appearance").performClick()
            captureSettled("restyle-fidelity-large-text-tablet-settings.png")
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
        compose.onNodeWithTag("app-toolbar").assertDoesNotExist()
        compose.onNodeWithTag("search-field").assertTextContains("synthetic")
        compose.onNodeWithContentDescription("Leave search").assertIsDisplayed()
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
        val badge =
            compose
                .onNodeWithTag("duration-${cards[0].key}")
                .assertIsDisplayed()
                .fetchSemanticsNode()
                .boundsInRoot
        Assert.assertTrue(
            "Wide search retains its duration within the thumbnail",
            badge.left >= cover.left &&
                badge.right <= cover.right &&
                badge.top >= cover.top &&
                badge.bottom <= cover.bottom,
        )
        compose.onNodeWithTag("media-menu-${cards[0].key}").performClick()
        compose.onNodeWithText("Preview").assertIsDisplayed()
    }

    @Test
    fun supplementalSheetsKeepRealQueueCommandsAndFitTheirContent() {
        val device = UiDevice.getInstance(InstrumentationRegistry.getInstrumentation())
        fun queueNow() = PlaybackService.rpcArray(json("op" to "queue")).objects()
        fun clearQueue() {
            val items = queueNow()
            items.indices.reversed().forEach { index ->
                PlaybackService.rpc(
                    json(
                        "op" to "queue_remove",
                        "index" to index,
                        "url" to items[index].optString("url"),
                    )
                )
            }
        }
        val saved = queueNow()
        try {
            for (wide in listOf(false, true)) {
                if (wide) {
                    scenario.close()
                    device.executeShellCommand("wm size 2560x1600")
                    device.executeShellCommand("wm density 320")
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
                }
                val profile = if (wide) "tablet" else "phone"
                val cards = populate(1).take(3)
                clearQueue()
                for (card in cards) {
                    PlaybackService.rpc(json("op" to "enqueue", "media" to card.value))
                    // Synthetic records in the explicitly disposable test app only.
                    PlaybackService.rpc(json("op" to "history_record", "media" to card.value))
                }
                compose.onNodeWithTag("menu-button").performClick()
                compose.onNodeWithText("Playback queue").performClick()
                compose.waitUntil(10_000) {
                    compose
                        .onAllNodesWithTag("queue-item-${cards[2].key}")
                        .fetchSemanticsNodes()
                        .isNotEmpty()
                }
                compose.onNodeWithTag("queue-menu-${cards[0].key}").performClick()
                compose.onNodeWithTag("queue-up-${cards[0].key}").assertIsNotEnabled()
                device.pressBack()
                compose.onNodeWithTag("queue-menu-${cards[2].key}").performClick()
                compose.onNodeWithTag("queue-down-${cards[2].key}").assertIsNotEnabled()
                device.pressBack()
                for (card in cards) {
                    compose.waitUntil(10_000) {
                        compose
                            .onAllNodes(
                                hasTestTag("artwork-${card.key}") and
                                    hasAnyAncestor(hasTestTag("queue-list")),
                                useUnmergedTree = true,
                            )
                            .fetchSemanticsNodes()
                            .isNotEmpty()
                    }
                }
                captureSettled("restyle-fidelity-$profile-queue-populated.png")
                compose.onNodeWithTag("queue-menu-${cards[1].key}").performClick()
                captureSettled("restyle-fidelity-$profile-queue-actions.png")
                compose.onNodeWithTag("queue-up-${cards[1].key}").performClick()
                compose.waitUntil(10_000) {
                    queueNow().firstOrNull()?.optString("url") == cards[1].url
                }
                compose.onNodeWithTag("queue-menu-${cards[2].key}").performClick()
                compose.onNodeWithTag("queue-remove-${cards[2].key}").performClick()
                compose.waitUntil(10_000) { queueNow().size == 2 }
                captureSettled("restyle-fidelity-$profile-queue-edited.png")
                compose.onNodeWithContentDescription("Close").performClick()
                compose.onNodeWithTag("menu-button").performClick()
                compose.onNodeWithText("Recently played").performClick()
                compose.onNodeWithTag("history-list").assertIsDisplayed()
                compose.onNodeWithText("Clear history and resume positions").assertIsDisplayed()
                compose.waitUntil(10_000) {
                    compose
                        .onAllNodes(
                            hasTestTag("artwork-${cards.last().key}") and
                                hasAnyAncestor(hasTestTag("history-list")),
                            useUnmergedTree = true,
                        )
                        .fetchSemanticsNodes()
                        .isNotEmpty()
                }
                captureSettled("restyle-fidelity-$profile-history.png")
                compose.onNodeWithContentDescription("Close").performClick()
                compose.onNodeWithTag("menu-button").performClick()
                compose.onNodeWithText("Sleep timer").performClick()
                compose.onNodeWithText("15 minutes").assertIsDisplayed()
                compose.onNodeWithText("Cancel timer").assertIsDisplayed()
                captureSettled("restyle-fidelity-$profile-sleep.png")
                compose.onNodeWithContentDescription("Close").performClick()
                compose.onNodeWithTag("menu-button").performClick()
                compose.onNodeWithTag("feeds-button").performClick()
                compose.onNodeWithText("Feeds and library").assertIsDisplayed()
                captureSettled("restyle-fidelity-$profile-feeds.png")
                compose.onNodeWithContentDescription("Close").performClick()
                clearQueue()
                compose.onNodeWithTag("menu-button").performClick()
                compose.onNodeWithText("Playback queue").performClick()
                compose.waitUntil(10_000) {
                    compose.onAllNodesWithTag("queue-empty").fetchSemanticsNodes().isNotEmpty()
                }
                val sheet = compose.onNodeWithTag("panel-body").fetchSemanticsNode().boundsInRoot
                Assert.assertTrue(
                    "An empty queue must fit content instead of reserving a full list",
                    sheet.height < device.displayHeight * 0.65f,
                )
                compose.onAllNodesWithTag("queue-list").assertCountEquals(0)
                captureSettled("restyle-fidelity-$profile-queue-empty.png")
                compose.onNodeWithContentDescription("Close").performClick()
            }
        } finally {
            clearQueue()
            for (item in saved) PlaybackService.rpc(json("op" to "enqueue", "media" to item))
        }
    }

    @Test
    fun tabletPlayerKeepsSideAndFocusedControlsReachable() {
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
        val cards = populate(2)
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val fixture = File(instrumentation.targetContext.cacheDir, "tablet-ui-fixture.m4a")
        instrumentation.context.assets.open("fixture-audio.m4a").use { input ->
            fixture.outputStream().use { input.copyTo(it) }
        }
        try {
            MediaBridge.execute(
                json(
                        "op" to "play",
                        "request" to
                            json(
                                "media" to cards[0].value,
                                "audio" to true,
                                "stream" to
                                    json("url" to android.net.Uri.fromFile(fixture).toString()),
                            ),
                    )
                    .toString()
            )
            MediaRuntimeTest.await("Tablet fixture must play") {
                MediaRuntimeTest.status().optBoolean("playing")
            }
            MediaRuntimeTest.command("pause")
            MediaRuntimeTest.await("Tablet fixture must pause") {
                MediaRuntimeTest.status().optBoolean("paused")
            }
            compose.onNodeWithTag("tablet-player-panel").assertIsDisplayed()
            scenario.onActivity {
                PlaybackService.current!!.player.shuffleModeEnabled = false
                PlaybackService.current!!.player.repeatMode =
                    androidx.media3.common.Player.REPEAT_MODE_OFF
            }
            MediaRuntimeTest.await("Tablet test starts with known modes") {
                !MediaRuntimeTest.status().optBoolean("shuffle") &&
                    !MediaRuntimeTest.status().optBoolean("repeat_one")
            }
            compose
                .onNodeWithTag("pane-shuffle")
                .performScrollTo()
                .assertIsNotSelected()
                .performClick()
            MediaRuntimeTest.await("Sidebar shuffle uses the real mode") {
                MediaRuntimeTest.status().optBoolean("shuffle")
            }
            compose.onNodeWithTag("pane-repeat").assertIsNotSelected().performClick()
            MediaRuntimeTest.await("Sidebar repeat uses the real mode") {
                MediaRuntimeTest.status().optBoolean("repeat_one")
            }
            compose.onNodeWithTag("pane-shuffle").assertIsSelected()
            compose.onNodeWithTag("pane-repeat").assertIsSelected()
            captureSettled("restyle-fidelity-tablet-side-player.png")
            compose.onNodeWithText("Open full player").performScrollTo().performClick()
            compose.onNodeWithTag("focused-player").assertIsDisplayed()
            compose.onNodeWithTag("player-pause").assertIsDisplayed()
            compose.onNodeWithTag("player-shuffle").assertIsSelected()
            compose.onNodeWithTag("player-repeat").assertIsSelected()
            captureSettled("restyle-fidelity-tablet-focused-player.png")
            compose.onNodeWithContentDescription("Minimize player").performClick()
            compose.onNodeWithTag("tablet-player-panel").assertIsDisplayed()
            Assert.assertTrue(
                "Minimizing must retain paused decoder state",
                MediaRuntimeTest.status().optBoolean("paused"),
            )
        } finally {
            MediaBridge.main.post {
                PlaybackService.current?.player?.shuffleModeEnabled = false
                PlaybackService.current?.player?.repeatMode =
                    androidx.media3.common.Player.REPEAT_MODE_OFF
            }
            MediaBridge.execute(json("op" to "stop").toString())
            fixture.delete()
        }
    }

    @Test
    fun videoWatchDetailsFollowTheFrameAndCollapseKeepsPlayback() {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val device = UiDevice.getInstance(instrumentation)
        val cards = populate(1)
        val fixture = File(instrumentation.targetContext.cacheDir, "watch-layout-fixture.mp4")
        instrumentation.context.assets.open("fixture.mp4").use { input ->
            fixture.outputStream().use { input.copyTo(it) }
        }
        var generation = 0L
        val initialFrames = MediaBridge.videoFrames.get()
        try {
            MediaBridge.execute(
                json(
                        "op" to "play",
                        "request" to
                            json(
                                "media" to cards[0].value,
                                "audio" to false,
                                "stream" to
                                    json("url" to android.net.Uri.fromFile(fixture).toString()),
                            ),
                    )
                    .toString()
            )
            MediaBridge.event("show_player", "")
            compose.waitUntil(10_000) {
                compose.onAllNodesWithTag("native-video-surface").fetchSemanticsNodes().isNotEmpty()
            }
            MediaRuntimeTest.await("Watch fixture must play real decoded video") {
                MediaRuntimeTest.status().optBoolean("playing") &&
                    MediaBridge.videoFrames.get() > initialFrames
            }
            scenario.onActivity {
                PlaybackService.current!!.player.repeatMode =
                    androidx.media3.common.Player.REPEAT_MODE_ONE
                generation = PlaybackService.current!!.generation
                ViewModelProvider(it)[YtuiViewModel::class.java].updateMobile("gestures", true)
            }
            for ((index, profile) in
                listOf("phone", "tablet", "phone-large", "tablet-large").withIndex()) {
                val wide = profile.startsWith("tablet")
                if (index > 0) {
                    scenario.close()
                    device.executeShellCommand(
                        "settings put system font_scale ${if (profile.endsWith("large")) "2.0" else "1.0"}"
                    )
                    device.executeShellCommand(
                        if (wide) "wm size 2560x1600" else "wm size 1080x2400"
                    )
                    device.executeShellCommand(if (wide) "wm density 320" else "wm density 420")
                    scenario = RuntimeTest.launchOffline()
                }
                MediaBridge.event("show_player", "")
                compose.waitUntil(10_000) {
                    compose
                        .onAllNodesWithTag("native-video-surface")
                        .fetchSemanticsNodes()
                        .isNotEmpty()
                }
                compose.waitForIdle()
                val density = instrumentation.targetContext.resources.displayMetrics.density
                val video =
                    compose.onNodeWithTag("native-video-surface").fetchSemanticsNode().boundsInRoot
                val title =
                    compose
                        .onNodeWithTag("panel-header")
                        .assertHeightIsAtLeast(androidx.compose.ui.unit.Dp(48f))
                        .fetchSemanticsNode()
                        .boundsInRoot
                val body = compose.onNodeWithTag("panel-body").fetchSemanticsNode().boundsInRoot
                Assert.assertEquals(
                    "Watch video must retain 16:9",
                    16f / 9f,
                    video.width / video.height,
                    0.03f,
                )
                Assert.assertEquals("No generic toolbar above watch video", body.top, video.top, 1f)
                if (wide) {
                    Assert.assertTrue(
                        "Wide details belong beside the video",
                        title.left >= video.right,
                    )
                    Assert.assertTrue(
                        "Wide video uses the larger pane",
                        video.width / body.width in 0.45f..0.70f,
                    )
                } else {
                    Assert.assertEquals(
                        "Portrait watch video is full bleed",
                        body.width,
                        video.width,
                        1f,
                    )
                    Assert.assertTrue(
                        "Title must immediately follow video, not a weighted blank gap",
                        title.top - video.bottom in 0f..(16f * density + 1f),
                    )
                }
                compose.onNodeWithText("Now playing").assertDoesNotExist()
                settleUi()
                MediaRuntimeTest.await("The watch capture must contain actual decoded RGB video") {
                    MediaRuntimeTest.videoVisible(
                        "restyle-fidelity-$profile-watch-details.png",
                        wide,
                        true,
                    )
                }
                compose.onNodeWithTag("player-options").performScrollTo()
                val queueAction =
                    compose
                        .onNodeWithContentDescription("Playback queue")
                        .assertIsDisplayed()
                        .fetchSemanticsNode()
                        .boundsInRoot
                val moreAction =
                    compose
                        .onNodeWithTag("player-options")
                        .assertIsDisplayed()
                        .fetchSemanticsNode()
                        .boundsInRoot
                Assert.assertEquals(
                    "Actions must span the details viewport",
                    title.left,
                    queueAction.left,
                    1f,
                )
                Assert.assertEquals(
                    "More stays at the trailing edge",
                    title.right,
                    moreAction.right,
                    1f,
                )
                Assert.assertEquals(generation, PlaybackService.current!!.generation)
                Assert.assertTrue(MediaRuntimeTest.status().optBoolean("playing"))
                if (index == 0) {
                    // Exercise the actual View control, not a Compose compatibility proxy.
                    scenario.onActivity {
                        val collapse = it.findViewById<android.view.View>(R.id.ytui_collapse)
                        val playerView =
                            generateSequence(collapse) { view -> view.parent as? android.view.View }
                                .filterIsInstance<androidx.media3.ui.PlayerView>()
                                .first()
                        playerView.showController()
                    }
                    // Measure after the controller's next layout pass, not in the
                    // same main-thread task that made it visible.
                    instrumentation.waitForIdleSync()
                    compose.waitForIdle()
                    scenario.onActivity {
                        val collapse = it.findViewById<android.view.View>(R.id.ytui_collapse)
                        Assert.assertEquals("panel-close", collapse.tag)
                        Assert.assertEquals(
                            "Minimize player",
                            collapse.contentDescription.toString(),
                        )
                        Assert.assertTrue(
                            collapse.width >= 48f * density - 1f &&
                                collapse.height >= 48f * density - 1f
                        )
                    }
                    // Keep the auto-hiding controller visible for the UiAutomator
                    // lookup (restored below); match the id like MediaRuntimeTest.
                    fun watchPlayer(activity: android.app.Activity) =
                        generateSequence(
                                activity.findViewById<android.view.View>(R.id.ytui_collapse)
                            ) { view ->
                                view.parent as? android.view.View
                            }
                            .filterIsInstance<androidx.media3.ui.PlayerView>()
                            .first()
                    var hideAfter = 0
                    scenario.onActivity {
                        val view = watchPlayer(it)
                        hideAfter = view.controllerShowTimeoutMs
                        view.controllerShowTimeoutMs = 0
                        view.showController()
                    }
                    val collapse =
                        device.wait(
                            androidx.test.uiautomator.Until.findObject(
                                androidx.test.uiautomator.By.res(
                                    java.util.regex.Pattern.compile(".*:id/ytui_collapse")
                                )
                            ),
                            5000,
                        )
                    if (collapse == null)
                        device.dumpWindowHierarchy(
                            File(
                                instrumentation.targetContext.getExternalFilesDir(null),
                                "watch-collapse-missing.xml",
                            )
                        )
                    Assert.assertNotNull("Native collapse control must be accessible", collapse)
                    collapse.click()
                    // Restore the hide timer after the click; changing it first re-lays
                    // out the controller and makes the found UiObject2 stale.
                    scenario.onActivity {
                        it.findViewById<android.view.View>(R.id.ytui_collapse)?.let { view ->
                            generateSequence(view) { v -> v.parent as? android.view.View }
                                .filterIsInstance<androidx.media3.ui.PlayerView>()
                                .firstOrNull()
                                ?.controllerShowTimeoutMs = hideAfter
                        }
                    }
                    compose.waitUntil(10_000) { compose.onNodeWithTag("mini-player").isDisplayed() }
                    Assert.assertEquals(generation, PlaybackService.current!!.generation)
                    Assert.assertTrue(MediaRuntimeTest.status().optBoolean("playing"))
                    compose.onNodeWithTag("mini-player").performClick()
                    compose.onNodeWithTag("panel-header").performScrollTo()
                    compose.onNodeWithTag("panel-header").performTouchInput {
                        swipe(
                            center,
                            center + androidx.compose.ui.geometry.Offset(0f, height * 3f),
                            300,
                        )
                    }
                    compose.waitUntil(10_000) { compose.onNodeWithTag("mini-player").isDisplayed() }
                    compose.onNodeWithTag("mini-player").performClick()
                    scenario.onActivity {
                        ViewModelProvider(it)[YtuiViewModel::class.java].updateMobile(
                            "gestures",
                            false,
                        )
                    }
                    compose.waitUntil(10_000) {
                        var disabled = false
                        scenario.onActivity {
                            disabled =
                                !ViewModelProvider(it)[YtuiViewModel::class.java]
                                    .mobile
                                    .value
                                    .optBoolean("gestures")
                        }
                        disabled
                    }
                    compose.onNodeWithTag("panel-header").performTouchInput {
                        swipe(
                            center,
                            center + androidx.compose.ui.geometry.Offset(0f, height * 3f),
                            300,
                        )
                    }
                    compose.onNodeWithTag("focused-player").assertIsDisplayed()
                    device.pressBack()
                    compose.waitUntil(10_000) { compose.onNodeWithTag("mini-player").isDisplayed() }
                    Assert.assertEquals(generation, PlaybackService.current!!.generation)
                    Assert.assertTrue(MediaRuntimeTest.status().optBoolean("playing"))
                    compose.onNodeWithTag("mini-player").performClick()
                }
            }
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
    fun musicTransitionsKeepTheDecoderAndProduceMotionEvidence() {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val device = UiDevice.getInstance(instrumentation)
        val scales =
            listOf(
                    "window_animation_scale",
                    "transition_animation_scale",
                    "animator_duration_scale",
                )
                .associateWith { device.executeShellCommand("settings get global $it").trim() }
        val cards = populate(2)
        val fixture = File(instrumentation.targetContext.cacheDir, "motion-ui-fixture.m4a")
        instrumentation.context.assets.open("fixture-audio.m4a").use { input ->
            fixture.outputStream().use { input.copyTo(it) }
        }
        val output =
            File(
                instrumentation.targetContext.getExternalFilesDir(null),
                "restyle-fidelity-motion.mp4",
            )
        if (output.isFile) Assert.assertTrue(output.delete())
        val recordingPath = "/data/local/tmp/ytui-fidelity-${android.os.Process.myPid()}.mp4"
        var recording: android.os.ParcelFileDescriptor? = null
        val previousClock = compose.mainClock.autoAdvance
        try {
            for (key in scales.keys) device.executeShellCommand("settings put global $key 1")
            scenario.onActivity {
                val vm = ViewModelProvider(it)[YtuiViewModel::class.java]
                vm.updateMobile("gestures", true)
                vm.updateConfig(false) { config -> config.put("reduced_motion", false) }
            }
            compose.waitUntil(10_000) {
                var animated = false
                scenario.onActivity {
                    val vm = ViewModelProvider(it)[YtuiViewModel::class.java]
                    animated =
                        !vm.config.value.optBoolean("reduced_motion", true) &&
                            vm.mobile.value.optBoolean("gestures")
                }
                animated
            }
            MediaBridge.execute(
                json(
                        "op" to "play",
                        "request" to
                            json(
                                "media" to cards[0].value,
                                "audio" to true,
                                "stream" to
                                    json("url" to android.net.Uri.fromFile(fixture).toString()),
                            ),
                    )
                    .toString()
            )
            MediaRuntimeTest.await("Motion fixture must start its real decoder") {
                MediaRuntimeTest.status().optBoolean("playing")
            }
            var generation = 0L
            scenario.onActivity {
                PlaybackService.current!!.player.repeatMode =
                    androidx.media3.common.Player.REPEAT_MODE_ONE
                generation = PlaybackService.current!!.generation
            }
            compose.onNodeWithTag("mini-player").assertIsDisplayed()
            recording =
                instrumentation.uiAutomation.executeShellCommand(
                    "screenrecord --size 720x1600 --bit-rate 2000000 --time-limit 30 $recordingPath"
                )
            // Drive the Compose animation clock in real-time while recording;
            // test auto-advancement would otherwise skip visible intermediate frames.
            compose.mainClock.autoAdvance = false
            fun frames(milliseconds: Int) {
                repeat((milliseconds + 15) / 16) {
                    compose.mainClock.advanceTimeByFrame()
                    android.os.SystemClock.sleep(16)
                }
            }
            val started = android.os.SystemClock.elapsedRealtime()
            val stages = mutableListOf<JSONObject>()
            fun mark(name: String) {
                stages +=
                    json(
                        "action" to name,
                        "elapsed_ms" to android.os.SystemClock.elapsedRealtime() - started,
                    )
            }
            frames(400)
            mark("expand player")
            compose.onNodeWithTag("mini-player").performClick()
            frames(400)
            compose.onNodeWithTag("focused-player").assertIsDisplayed()
            mark("swipe to minimize")
            compose.onNodeWithTag("panel-header").performTouchInput {
                swipe(center, center + androidx.compose.ui.geometry.Offset(0f, height * 3f), 300)
            }
            frames(400)
            compose.onNodeWithTag("mini-player").assertIsDisplayed()
            mark("expand player again")
            compose.onNodeWithTag("mini-player").performClick()
            frames(400)
            mark("minimize by chevron")
            compose.onNodeWithContentDescription("Minimize player").performClick()
            frames(400)
            mark("open timer sheet")
            compose.onNodeWithTag("menu-button").performClick()
            frames(240)
            compose.onNodeWithText("Sleep timer").performClick()
            frames(600)
            compose.onNodeWithText("Cancel timer").assertIsDisplayed()
            mark("close timer sheet")
            compose.onNodeWithContentDescription("Close").performClick()
            frames(600)
            compose.onNodeWithText("Cancel timer").assertDoesNotExist()
            mark("timer dismissed")
            Assert.assertTrue(
                "Recording must still cover the final closed sheet state",
                device.executeShellCommand("pidof screenrecord").trim().isNotEmpty(),
            )
            frames(400)
            scenario.onActivity {
                Assert.assertEquals(
                    "UI transitions must retain the actual decoder",
                    generation,
                    PlaybackService.current!!.generation,
                )
            }
            Assert.assertTrue(
                "Playback must continue through collapse and sheets",
                MediaRuntimeTest.status().optBoolean("playing"),
            )
            compose.mainClock.autoAdvance = previousClock
            compose.waitUntil(35_000) {
                device.executeShellCommand("pidof screenrecord").trim().isEmpty()
            }
            // shell owns the recording. Transfer bytes through UiAutomation so
            // the retained copy belongs to the app and is readable under FUSE.
            android.os.ParcelFileDescriptor.AutoCloseInputStream(
                    instrumentation.uiAutomation.executeShellCommand("cat $recordingPath")
                )
                .use { input -> output.outputStream().use { input.copyTo(it) } }
            Assert.assertTrue(
                "Native screen recording must be retained",
                output.isFile && output.length() > 10_000,
            )
            val retriever = android.media.MediaMetadataRetriever()
            val duration =
                try {
                    // The media service cannot reopen app-scoped storage by path.
                    java.io.FileInputStream(output).use { retriever.setDataSource(it.fd) }
                    retriever
                        .extractMetadata(
                            android.media.MediaMetadataRetriever.METADATA_KEY_DURATION
                        )!!
                        .toLong()
                } finally {
                    retriever.release()
                }
            Assert.assertTrue(
                "Motion evidence must contain a real multi-second clip",
                duration >= 5000,
            )
            File(output.parentFile, "restyle-fidelity-motion.json")
                .writeText(
                    json(
                            "source" to BuildConfig.SOURCE_REVISION,
                            "fixture" to "generated local audio/artwork",
                            "video_width" to 720,
                            "video_height" to 1600,
                            "duration_ms" to duration,
                            "animations" to "enabled; Compose clock advanced alongside wall time",
                            "actions" to org.json.JSONArray(stages),
                        )
                        .toString(2)
                )
            MediaRuntimeTest.capture("restyle-fidelity-motion-end.png")
        } finally {
            compose.mainClock.autoAdvance = previousClock
            recording?.close()
            device.executeShellCommand("rm -f $recordingPath")
            for ((key, value) in scales) {
                if (value.matches(Regex("[0-9]+(?:\\.[0-9]+)?")))
                    device.executeShellCommand("settings put global $key $value")
                else device.executeShellCommand("settings delete global $key")
            }
            MediaBridge.main.post {
                PlaybackService.current?.player?.repeatMode =
                    androidx.media3.common.Player.REPEAT_MODE_OFF
            }
            MediaBridge.execute(json("op" to "stop").toString())
            fixture.delete()
        }
    }

    private fun assertNavigationLabelsFit() {
        for (index in 0..3) {
            compose
                .onNodeWithTag("tab-$index")
                .assertIsDisplayed()
                .assertHeightIsAtLeast(androidx.compose.ui.unit.Dp(48f))
                .assert(
                    SemanticsMatcher.expectValue(
                        SemanticsProperties.Role,
                        androidx.compose.ui.semantics.Role.Tab,
                    )
                )
            val layouts = mutableListOf<androidx.compose.ui.text.TextLayoutResult>()
            compose
                .onNodeWithTag("navigation-label-$index", useUnmergedTree = true)
                .performSemanticsAction(SemanticsActions.GetTextLayoutResult) { it(layouts) }
            Assert.assertEquals(
                "A full scaled navigation word must fit on one line",
                1,
                layouts.single().lineCount,
            )
            Assert.assertFalse(
                "Navigation $index must not clip or ellipsize: " +
                    "text=${layouts.single().layoutInput.text}, size=${layouts.single().size}, " +
                    "constraints=${layouts.single().layoutInput.constraints}, " +
                    "paragraphWidth=${layouts.single().multiParagraph.width}, " +
                    "intrinsicWidth=${layouts.single().multiParagraph.maxIntrinsicWidth}, " +
                    "lineRight=${layouts.single().getLineRight(0)}, " +
                    "widthOverflow=${layouts.single().didOverflowWidth}, " +
                    "heightOverflow=${layouts.single().didOverflowHeight}",
                layouts.single().hasVisualOverflow,
            )
        }
        compose.onNodeWithTag("tab-1").assertIsSelected()
    }

    private fun awaitDecodedCovers(cards: List<MediaCard>) {
        var diagnostics = emptyList<String>()
        try {
            compose.waitUntil(15_000) {
                val viewport = compose.onNodeWithTag("feed-list").getUnclippedBoundsInRoot()
                var inspected = 0
                var decoded = true
                val checks = mutableListOf<String>()
                for (media in cards) {
                    if (!compose.onNodeWithTag("media-card-${media.key}").isDisplayed()) continue
                    runCatching {
                            // Judge visibility by the artwork frame, which exists before the
                            // cover loads: a still-loading cover in a partially visible row is
                            // not a full cover either.
                            val bounds =
                                compose
                                    .onNode(
                                        hasTestTag("artwork-frame-${media.key}") and
                                            hasAnyAncestor(hasTestTag("media-card-${media.key}")),
                                        useUnmergedTree = true,
                                    )
                                    .getUnclippedBoundsInRoot()
                            // A partially visible virtualized row is not a full cover.
                            // captureToImage clips it, changing the sample's coordinates.
                            if (
                                bounds.left < viewport.left ||
                                    bounds.right > viewport.right ||
                                    bounds.top < viewport.top ||
                                    bounds.bottom > viewport.bottom
                            ) {
                                checks += "partial cover bounds=$bounds viewport=$viewport"
                                return@runCatching
                            }
                            inspected++
                            val node =
                                compose.onNode(
                                    hasTestTag("artwork-${media.key}") and
                                        hasAnyAncestor(hasTestTag("media-card-${media.key}")),
                                    useUnmergedTree = true,
                                )
                            val frame = node.captureToImage()
                            val color = frame.toPixelMap()[frame.width / 2, frame.height / 4]
                            val saturated =
                                maxOf(color.red, color.green, color.blue) -
                                    minOf(color.red, color.green, color.blue) > 0.12f
                            decoded = decoded && saturated
                            checks += "cover ${frame.width}x${frame.height} saturated=$saturated"
                        }
                        .onFailure {
                            decoded = false
                            checks +=
                                "cover_index=${cards.indexOf(media)} capture=${it.javaClass.simpleName}:${it.message?.take(240)}"
                        }
                }
                diagnostics = checks
                inspected > 0 && decoded
            }
        } catch (error: Throwable) {
            captureSettled("restyle-fidelity-cover-failed.png")
            android.util.Log.i(
                "CoverEvidence",
                diagnostics.joinToString("; ") + " requests=" + covers.requestCounts(),
            )
            // Failure-only probes use this test's synthetic loopback artwork.
            // Never retry the assertion or turn an initial failed cover into a pass.
            for ((index, media) in cards.withIndex()) {
                runCatching {
                        if (!compose.onNodeWithTag("media-card-${media.key}").isDisplayed())
                            return@runCatching
                        if (
                            compose
                                .onAllNodesWithTag("artwork-${media.key}", useUnmergedTree = true)
                                .fetchSemanticsNodes()
                                .isNotEmpty()
                        )
                            return@runCatching
                        val data = NativeBridge.thumbnail(media.value.toString(), 384)
                        android.util.Log.i(
                            "CoverEvidence",
                            "missing_index=$index native_probe=${data?.let { "${it[0]}x${it[1]}" } ?: "unavailable"} requests=${covers.requestCounts()}",
                        )
                    }
                    .onFailure {
                        android.util.Log.i(
                            "CoverEvidence",
                            "probe_index=$index failed=${it.javaClass.simpleName}",
                        )
                    }
            }
            throw error
        }
    }

    private fun settleUi() {
        compose.waitForIdle()
        // A semantics node can precede its final window pixels/IME position.
        // This is UI-driver settling, never part of a playback deadline.
        InstrumentationRegistry.getInstrumentation().uiAutomation.waitForIdle(300, 3000)
    }

    private fun captureSettled(filename: String) {
        settleUi()
        MediaRuntimeTest.capture(filename)
    }

    private class CoverServer : AutoCloseable {
        private val socket = ServerSocket(0, 16, java.net.InetAddress.getByName("127.0.0.1"))
        val requests = AtomicInteger()

        fun requestCounts() = (0..5).map { perCover.get(it) }

        val failures = AtomicInteger()
        val perCover = java.util.concurrent.atomic.AtomicIntegerArray(6)
        @Volatile var failFirstCover = false
        private val images = (0..5).map(::cover)
        private val worker =
            Thread {
                    while (!socket.isClosed) try {
                        val accepted = socket.accept()
                        // Serve each connection on its own thread: one stalled read must not
                        // queue every other parallel cover load behind it.
                        Thread {
                                accepted.use { client ->
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
                                    perCover.incrementAndGet(index.coerceIn(0, 5))
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
                            }
                            .apply {
                                isDaemon = true
                                start()
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
