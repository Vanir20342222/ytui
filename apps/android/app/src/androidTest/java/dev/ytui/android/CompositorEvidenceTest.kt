package dev.ytui.android

import androidx.compose.ui.test.junit4.createEmptyComposeRule
import androidx.lifecycle.ViewModelProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import java.io.File
import org.junit.*
import org.junit.runner.RunWith

/** Functional occlusion proof, not a startup latency sample. */
@RunWith(AndroidJUnit4::class)
@OptIn(androidx.compose.ui.test.ExperimentalTestApi::class)
@androidx.annotation.OptIn(markerClass = [androidx.media3.common.util.UnstableApi::class])
class CompositorEvidenceTest {
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
    fun compositedPixelsIncludeTheSurfaceAndRejectAnOpaqueCover() {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val file = File(instrumentation.targetContext.cacheDir, "compositor-fixture.mp4")
        instrumentation.context.assets.open("fixture.mp4").use { input ->
            file.outputStream().use { input.copyTo(it) }
        }
        val original = MobileSettings.snapshot()
        MobileSettings.save(json("preload_next" to false, "autoplay_related" to false))
        try {
            RuntimeTest.launchOffline().use { scene ->
                val observer = VisibleVideoProbe.observe(scene)
                scene.onActivity {
                    ViewModelProvider(it)[YtuiViewModel::class.java].playerOpen.value = true
                }
                val token = StartupTimings.begin(android.os.SystemClock.elapsedRealtime(), true)
                MediaBridge.execute(
                    json(
                            "op" to "play",
                            "request" to
                                json(
                                    "media" to
                                        json(
                                            "url" to "https://media.test/compositor",
                                            "title" to "Compositor fixture",
                                            "kind" to "video",
                                        ),
                                    "stream" to
                                        json("url" to android.net.Uri.fromFile(file).toString()),
                                    "audio" to false,
                                    "local" to true,
                                    "startup_token" to token,
                                ),
                        )
                        .toString()
                )
                compose.waitUntil(10000) { StartupTimings.usable(token) > 0 }
                MediaRuntimeTest.command("pause")
                MediaRuntimeTest.await("Keep a real decoded frame while checking overlays") {
                    !MediaRuntimeTest.status().optBoolean("playing")
                }
                fun capture(name: String): VisibleVideoProbe.Result =
                    observer.capture(token, name, 20000)
                var clear = capture("startup-compositor-clear.png")
                val deadline = android.os.SystemClock.elapsedRealtime() + 5000
                while (!clear.rgb && android.os.SystemClock.elapsedRealtime() < deadline) {
                    Thread.sleep(50)
                    clear = capture("startup-compositor-clear.png")
                }
                Assert.assertEquals("window-compositor", clear.method)
                Assert.assertTrue(
                    "Actual SurfaceView video must appear in the composed window",
                    clear.rgb,
                )
                val overlay = java.util.concurrent.atomic.AtomicReference<android.view.View>()
                scene.onActivity {
                    val root = it.window.decorView as android.view.ViewGroup
                    Assert.assertTrue(
                        VisibleVideoProbe.find(root).videoSurfaceView is android.view.SurfaceView
                    )
                    overlay.set(
                        android.view.View(it).apply {
                            setBackgroundColor(android.graphics.Color.BLACK)
                        }
                    )
                    root.addView(overlay.get(), android.view.ViewGroup.LayoutParams(-1, -1))
                }
                try {
                    compose.waitForIdle()
                    val covered = capture("startup-compositor-covered.png")
                    Assert.assertEquals("window-compositor", covered.method)
                    Assert.assertFalse(
                        "A raw surface copy must not bypass its opaque window cover",
                        covered.rgb,
                    )
                } finally {
                    scene.onActivity {
                        (overlay.get().parent as? android.view.ViewGroup)?.removeView(overlay.get())
                    }
                }
                compose.waitForIdle()
                var restored = capture("startup-compositor-restored.png")
                val restoreDeadline = android.os.SystemClock.elapsedRealtime() + 5000
                while (
                    !restored.rgb && android.os.SystemClock.elapsedRealtime() < restoreDeadline
                ) {
                    Thread.sleep(50)
                    restored = capture("startup-compositor-restored.png")
                }
                Assert.assertTrue(
                    "Removing the cover must reveal the same decoded frame",
                    restored.rgb,
                )
            }
        } finally {
            MediaRuntimeTest.command("stop")
            MobileSettings.save(original)
            file.delete()
        }
    }
}
