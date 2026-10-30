package dev.ytui.android

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.luminance
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class MediaDesignTest {
    @Test
    fun secondaryInkStaysReadableOnRaisedImportedSurfaces() {
        for (surface in (0..255 step 8).map { Color(it, it, it) }) {
            for (ink in listOf(Color.Black, Color.White, Color.Gray, Color.Cyan, Color.Magenta)) {
                assertTrue(mediaContrast(mediaReadableText(ink, surface), surface) >= 4.5f)
            }
        }
        assertTrue(mediaContrast(MediaTokens.neutralText, MediaTokens.neutralBackground) >= 4.5f)
        assertTrue(mediaContrast(MediaTokens.neutralText, MediaTokens.neutralSurface) >= 4.5f)
        assertEquals(MediaTokens.neutralBackground.red, MediaTokens.neutralBackground.green)
        assertEquals(MediaTokens.neutralBackground.green, MediaTokens.neutralBackground.blue)
    }

    @Test
    fun progressSurvivesStopUnknownDurationAndInvalidTelemetry() {
        for ((position, duration) in
            listOf(
                0f to 0f,
                8f to 0f,
                Float.NaN to 8f,
                8f to Float.NaN,
                Float.POSITIVE_INFINITY to 8f,
                8f to Float.POSITIVE_INFINITY,
                8f to -1f,
            )) assertEquals(0f, mediaProgress(position, duration), 0f)
        assertEquals(0.5f, mediaProgress(4f, 8f), 0f)
        assertEquals(1f, mediaProgress(12f, 8f), 0f)
        assertEquals(0f, mediaProgress(-1f, 8f), 0f)
    }

    @Test
    fun accentControlsKeepReadableTextAcrossImportedColours() {
        val colors =
            (0..255).map { Color(it, it, it) } +
                listOf(0, 64, 128, 192, 255).flatMap { r ->
                    listOf(0, 64, 128, 192, 255).flatMap { g ->
                        listOf(0, 64, 128, 192, 255).map { b -> Color(r, g, b) }
                    }
                }
        colors.forEach { color ->
            val a = color.luminance()
            val b = mediaOnAccent(color).luminance()
            val contrast = (maxOf(a, b) + 0.05f) / (minOf(a, b) + 0.05f)
            assertTrue("Accent $color needs WCAG AA text contrast; was $contrast", contrast >= 4.5f)
        }
    }
}
