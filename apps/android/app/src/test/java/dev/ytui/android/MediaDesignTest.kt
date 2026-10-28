package dev.ytui.android

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.luminance
import org.junit.Assert.assertTrue
import org.junit.Test

class MediaDesignTest {
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
