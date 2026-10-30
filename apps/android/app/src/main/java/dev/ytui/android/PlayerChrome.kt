package dev.ytui.android

import android.graphics.drawable.GradientDrawable
import android.view.View
import androidx.compose.ui.graphics.toArgb
import androidx.media3.ui.DefaultTimeBar
import androidx.media3.ui.PlayerView

/** Presentation only: preserve Media3 commands, player binding, gestures and auto-hide. */
@androidx.annotation.OptIn(markerClass = [androidx.media3.common.util.UnstableApi::class])
internal object PlayerChrome {
    private var accent = MediaTokens.videoInk.toArgb()

    @JvmStatic
    fun rememberAccent(value: Int) {
        accent = value
    }

    @JvmStatic
    fun apply(view: PlayerView) {
        val clear = MediaTokens.transparent.toArgb()
        val scrim = MediaTokens.videoScrim.toArgb()
        view.findViewById<View>(R.id.ytui_top_scrim)?.background =
            GradientDrawable(GradientDrawable.Orientation.TOP_BOTTOM, intArrayOf(scrim, clear))
        view.findViewById<View>(R.id.ytui_bottom_scrim)?.background =
            GradientDrawable(GradientDrawable.Orientation.BOTTOM_TOP, intArrayOf(scrim, clear))
        view.findViewById<View>(androidx.media3.ui.R.id.exo_play_pause)?.background =
            GradientDrawable().apply {
                shape = GradientDrawable.OVAL
                setColor(MediaTokens.videoDisc.toArgb())
            }
        view.findViewById<DefaultTimeBar>(androidx.media3.ui.R.id.exo_progress)?.apply {
            setPlayedColor(accent)
            setScrubberColor(accent)
        }
    }
}
