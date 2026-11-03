package dev.ytui.android

import org.junit.Assert.assertEquals
import org.junit.Test

class ThemeLabelTest {
    @Test
    fun themeIdsBecomeReadableNames() {
        assertEquals("ytui Dark", themeLabel("ytui-dark"))
        assertEquals("Catppuccin Mocha", themeLabel("catppuccin-mocha"))
        assertEquals("System", themeLabel("system"))
        assertEquals("Gruvbox", themeLabel("gruvbox"))
        assertEquals("", themeLabel(""))
    }
}
