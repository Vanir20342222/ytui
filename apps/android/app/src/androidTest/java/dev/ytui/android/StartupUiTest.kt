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
                            rgb =
                                MediaRuntimeTest.videoVisible(
                                    "startup-${if(prepared) "prepared" else "cold"}.png",
                                    false,
                                    true,
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
                }
            }
        } finally {
            File(instrumentation.targetContext.getExternalFilesDir(null), "startup-timings.txt")
                .writeText(StartupTimings.report() + confirmations)
            MobileSettings.save(original)
        }
    }
}
