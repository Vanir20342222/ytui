package dev.ytui.android

import androidx.compose.ui.graphics.toPixelMap
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createEmptyComposeRule
import androidx.compose.ui.text.TextLayoutResult
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.UiDevice
import java.util.concurrent.atomic.AtomicBoolean
import org.json.JSONObject
import org.junit.*
import org.junit.runner.RunWith

/** These checks exercise the actual Android UI, not Rust's headless egui frames. */
@RunWith(AndroidJUnit4::class)
@OptIn(androidx.compose.ui.test.ExperimentalTestApi::class)
class NativeUiTest {
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

    @Before
    fun launch() {
        original =
            JSONObject(NativeBridge.request(json("op" to "settings").toString()))
                .getJSONObject("data")
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
        Assert.assertFalse(
            "A system ANR dialog is covering the app",
            UiDevice.getInstance(InstrumentationRegistry.getInstrumentation())
                .hasObject(androidx.test.uiautomator.By.res("android", "aerr_close")),
        )
    }

    @After
    fun close() {
        scenario.close()
        NativeBridge.request(json("op" to "save_settings", "config" to original).toString())
        MediaBridge.execute(json("op" to "settings", "config" to original).toString())
    }

    @Test
    fun realKeyboardAndSettingsTextFieldsAcceptInput() {
        compose.onNodeWithTag("search-field").performClick()
        val ime = AtomicBoolean()
        compose.waitUntil(10_000) {
            scenario.onActivity {
                ime.set(
                    ViewCompat.getRootWindowInsets(it.window.decorView)
                        ?.isVisible(WindowInsetsCompat.Type.ime()) == true
                )
            }
            ime.get()
        }
        UiDevice.getInstance(InstrumentationRegistry.getInstrumentation())
            .executeShellCommand("input text tablet-input")
        compose.onNodeWithTag("search-field").assertTextContains("tablet-input")
        compose.onNodeWithTag("search-field").performTextInput(" Grüße")
        compose.onNodeWithTag("search-field").assertTextContains("tablet-input Grüße")
        captureSettled("native-search.png")
        UiDevice.getInstance(InstrumentationRegistry.getInstrumentation()).pressBack()
        compose.onNodeWithTag("search-field").assertIsDisplayed()
        compose.onNodeWithTag("settings-button").performClick()
        compose.onNodeWithTag("settings-Network").performClick()
        compose
            .onNodeWithTag("proxy-field")
            .performScrollTo()
            .performTextInput("http://127.0.0.1:8080")
        compose.onNodeWithTag("proxy-field").assertTextContains("http://127.0.0.1:8080")
        // Do not apply a proxy: this test verifies actual editing, not networking.
    }

    @Test
    fun everySharedThemeChangesThePaintedNativeUiAndPersists() {
        val themes =
            JSONObject(NativeBridge.request(json("op" to "themes").toString())).getJSONArray("data")
        for (index in 0 until themes.length()) {
            val theme = themes.getString(index)
            compose.onNodeWithTag("settings-button").performClick()
            compose.onNodeWithTag("settings-Appearance").performClick()
            compose.onNodeWithTag("theme-selector").performClick()
            compose.onNodeWithTag("theme-$theme").performScrollTo().performClick()
            compose.waitUntil(10_000) {
                compose.onAllNodesWithText("Theme: $theme").fetchSemanticsNodes().isNotEmpty()
            }
            compose.onNodeWithContentDescription("Close").performClick()
            compose.waitForIdle()
            val dark =
                InstrumentationRegistry.getInstrumentation()
                    .targetContext
                    .resources
                    .configuration
                    .uiMode and android.content.res.Configuration.UI_MODE_NIGHT_MASK ==
                    android.content.res.Configuration.UI_MODE_NIGHT_YES
            val palette =
                JSONObject(NativeBridge.request(json("op" to "theme", "dark" to dark).toString()))
                    .getJSONObject("data")
            val expectedColor =
                palette.color("background", androidx.compose.ui.graphics.Color.Black)
            val actual = compose.onNodeWithTag("app-background").captureToImage().toPixelMap()[0, 0]
            Assert.assertEquals(expectedColor.red, actual.red, 0.03f)
            Assert.assertEquals(expectedColor.green, actual.green, 0.03f)
            Assert.assertEquals(expectedColor.blue, actual.blue, 0.03f)
            val layouts = mutableListOf<TextLayoutResult>()
            compose.onNodeWithTag("feed-heading").performSemanticsAction(
                SemanticsActions.GetTextLayoutResult
            ) {
                it(layouts)
            }
            Assert.assertTrue("The feed heading should expose a text layout", layouts.isNotEmpty())
            val textColor = layouts.single().layoutInput.style.color
            val ink = palette.color("text", androidx.compose.ui.graphics.Color.White)
            Assert.assertEquals(ink.red, textColor.red, 0.03f)
            Assert.assertEquals(ink.green, textColor.green, 0.03f)
            Assert.assertEquals(ink.blue, textColor.blue, 0.03f)
            val saved =
                JSONObject(NativeBridge.request(json("op" to "settings").toString()))
                    .getJSONObject("data")
            Assert.assertEquals(theme, saved.getString("theme"))
            captureSettled("native-theme-$index.png")
        }
    }

    @Test
    fun importedPaletteWithBuiltInNameKeepsItsOwnPaint() {
        val file =
            java.io.File(
                InstrumentationRegistry.getInstrumentation().targetContext.filesDir,
                "palette.json",
            )
        val previous = if (file.isFile) file.readBytes() else null
        try {
            val document =
                JSONObject(NativeBridge.request(json("op" to "theme", "dark" to true).toString()))
                    .getJSONObject("data")
                    .put("name", "ytui-dark")
                    .put("source", "synthetic imported palette")
                    .put("background", org.json.JSONArray(listOf(12, 20, 50)))
                    .put("surface", org.json.JSONArray(listOf(20, 32, 60)))
                    .put("text", org.json.JSONArray(listOf(230, 240, 250)))
                    .put("light", false)
            val imported =
                JSONObject(
                    NativeBridge.request(
                        json("op" to "import_theme", "text" to document.toString()).toString()
                    )
                )
            Assert.assertTrue("Synthetic palette import must succeed", imported.optBoolean("ok"))
            scenario.onActivity {
                androidx.lifecycle.ViewModelProvider(it)[YtuiViewModel::class.java].updateConfig(
                    false
                ) { config ->
                    config.put("theme", "custom")
                }
            }
            compose.waitUntil(10_000) {
                val color =
                    compose.onNodeWithTag("app-background").captureToImage().toPixelMap()[0, 0]
                kotlin.math.abs(color.blue - 50f / 255f) < 0.02f &&
                    kotlin.math.abs(color.red - 12f / 255f) < 0.02f
            }
            captureSettled("restyle-imported-dark-name.png")
        } finally {
            NativeBridge.request(json("op" to "save_settings", "config" to original).toString())
            if (previous != null) file.writeBytes(previous)
            else if (file.isFile) Assert.assertTrue(file.delete())
        }
    }

    @Test
    fun accountRequiredFeedOffersReachableSignInAndCookieImport() {
        Assume.assumeFalse(
            "Uses a disposable disconnected app",
            JSONObject(NativeBridge.request(json("op" to "account").toString()))
                .getJSONObject("data")
                .optBoolean("connected"),
        )
        compose.onNodeWithTag("feeds-button").performClick()
        compose.onNodeWithText("Subscriptions").performClick()
        compose.waitUntil(10_000) {
            compose.onAllNodesWithTag("feed-account-action").fetchSemanticsNodes().isNotEmpty()
        }
        compose.onNodeWithText("Account needed").assertIsDisplayed()
        compose.onNodeWithTag("feed-account-action").performClick()
        compose.onNodeWithTag("account-import").assertIsDisplayed()
        captureSettled("native-account.png")
        compose
            .onNodeWithTag("cookie-header-field")
            .performScrollTo()
            .performTextInput("synthetic-not-an-import")
        compose.onNodeWithTag("cookie-header-field").assertIsFocused()
        UiDevice.getInstance(InstrumentationRegistry.getInstrumentation()).pressBack()
        compose.onNodeWithTag("account-sign-in").performScrollTo().performClick()
        compose.waitUntil(10_000) {
            compose
                .onAllNodesWithTag("sign-in-complete")
                .fetchSemanticsNodes(atLeastOneRootRequired = false)
                .isNotEmpty()
        }
        compose.onNodeWithTag("sign-in-cancel").assertIsDisplayed().performClick()
        compose.waitUntil(10_000) {
            compose
                .onAllNodesWithTag("account-import")
                .fetchSemanticsNodes(atLeastOneRootRequired = false)
                .isNotEmpty()
        }
        Assert.assertFalse(
            JSONObject(NativeBridge.request(json("op" to "account").toString()))
                .getJSONObject("data")
                .optBoolean("connected")
        )
    }

    @Test
    fun pastedPlaylistAndMusicUrlsKeepTheirProviderIntent() {
        scenario.onActivity {
            val vm = androidx.lifecycle.ViewModelProvider(it)[YtuiViewModel::class.java]
            vm.chooseTab(0)
            val playlist = vm.linked("https://music.youtube.com/playlist?list=PLfixture")
            Assert.assertTrue(playlist.collection)
            Assert.assertTrue(playlist.musicSource)
            Assert.assertEquals("VLPLfixture", playlist.collectionId)
            Assert.assertEquals(
                "VLPLfixture",
                MediaCard(
                        json(
                            "kind" to "playlist",
                            "id" to "PLfixture",
                            "browse_id" to "VLPLfixture",
                        )
                    )
                    .collectionId,
            )
            Assert.assertFalse(
                vm.linked("https://music.youtube.com/watch?v=fixture&list=PLfixture").collection
            )
            Assert.assertTrue(vm.linked("https://music.youtube.com/watch?v=fixture").audio)
            Assert.assertFalse(vm.linked("https://youtube.com/playlist?list=").collection)
            Assert.assertFalse(
                vm.linked("https://media.invalid/playlist?list=PLfixture").collection
            )
        }
    }

    @Test
    fun continuationFailureDoesNotTriggerAnImmediateScrollRetry() {
        val reference = java.util.concurrent.atomic.AtomicReference<YtuiViewModel>()
        scenario.onActivity {
            val vm = androidx.lifecycle.ViewModelProvider(it)[YtuiViewModel::class.java]
            reference.set(vm)
            vm.feeds[1].value =
                FeedState(
                    cards =
                        listOf(
                            MediaCard(
                                json(
                                    "title" to "Retained result",
                                    "url" to "https://media.invalid/retained",
                                )
                            )
                        ),
                    continuation = "fixture:invalid-endpoint",
                    error = "Service cooldown",
                    generation = 500,
                    failedAppend = true,
                )
        }
        compose.waitForIdle()
        android.os.SystemClock.sleep(600)
        Assert.assertEquals(500L, reference.get().feeds[1].value.generation)
        Assert.assertEquals("Retained result", reference.get().feeds[1].value.cards.single().title)
        // If a prefetch accidentally runs, the synthetic endpoint is rejected
        // before any HTTP request; no public provider is contacted by this test.
    }

    @Test
    fun searchAndFilterGeometryStaysUsableInPortraitAndLandscape() {
        val device = UiDevice.getInstance(InstrumentationRegistry.getInstrumentation())
        val density =
            InstrumentationRegistry.getInstrumentation()
                .targetContext
                .resources
                .displayMetrics
                .density
        fun chipBounds(tags: List<String>) {
            val heights =
                tags.map { tag ->
                    val bounds =
                        compose
                            .onNodeWithTag(tag)
                            .assertIsDisplayed()
                            .fetchSemanticsNode()
                            .boundsInRoot
                    Assert.assertTrue(
                        "$tag must have a 48dp visible/touch height: ${bounds.height / density}",
                        bounds.height + 1f >= 48f * density,
                    )
                    bounds.height
                }
            Assert.assertTrue(
                "Chips in one row must have the same height",
                heights.max() - heights.min() <= 1f,
            )
        }
        try {
            for (wide in listOf(false, true)) {
                if (wide) device.setOrientationLeft() else device.setOrientationNatural()
                compose.waitForIdle()
                compose.onNodeWithTag("tab-1").performClick()
                val pane = compose.onNodeWithTag("browse-pane").fetchSemanticsNode().boundsInRoot
                val search = compose.onNodeWithTag("search-field").fetchSemanticsNode().boundsInRoot
                val expected = kotlin.math.min(pane.width - 32f * density, 720f * density)
                Assert.assertEquals(
                    "Search must be capped on wide screens and fill narrow screens",
                    expected,
                    search.width,
                    2f,
                )
                Assert.assertEquals(
                    "Search must stay centered in its pane",
                    pane.center.x,
                    search.center.x,
                    2f,
                )
                chipBounds(listOf("source-Home", "source-Playlists", "source-Liked"))
                compose.onNodeWithTag("tab-0").performClick()
                chipBounds(listOf("library-Offline", "library-Playlists", "library-Transfers"))
            }
        } finally {
            device.setOrientationNatural()
            device.unfreezeRotation()
        }
    }

    @Test
    fun wideLayoutKeepsTabsSettingsAndPlayerPanelsReachable() {
        val device = UiDevice.getInstance(InstrumentationRegistry.getInstrumentation())
        try {
            device.setOrientationLeft()
            compose.waitForIdle()
            for (index in listOf(0, 1, 2, 3)) {
                compose.onNodeWithTag("tab-$index").performClick()
                if (index == 3 || index == 0)
                    captureSettled(
                        if (index == 3) "restyle-radio-empty.png" else "restyle-downloads.png"
                    )
                compose.onNodeWithTag("settings-button").assertIsDisplayed()
                if (index == 0) compose.onNodeWithTag("add-download").assertIsDisplayed()
                else compose.onNodeWithTag("search-field").assertIsDisplayed()
            }
            for (label in
                listOf("Playback queue", "Recently played", "Lyrics", "Sleep timer", "About")) {
                compose.onNodeWithTag("menu-button").performClick()
                compose.onNodeWithText(label).performClick()
                captureSettled("restyle-panel-${label.lowercase().replace(" ", "-")}.png")
                compose.onNodeWithContentDescription("Close").assertIsDisplayed().performClick()
            }
            captureSettled("native-tablet.png")
        } finally {
            device.setOrientationNatural()
            device.unfreezeRotation()
        }
    }

    private fun captureSettled(filename: String) {
        compose.waitForIdle()
        InstrumentationRegistry.getInstrumentation().uiAutomation.waitForIdle(300, 3000)
        MediaRuntimeTest.capture(filename)
    }
}
