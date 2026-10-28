package dev.ytui.android

import android.content.Intent
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.detectVerticalDragGestures
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.media3.ui.PlayerView

@androidx.annotation.OptIn(markerClass = [androidx.media3.common.util.UnstableApi::class])
@Composable
internal fun VideoSurface(modifier: Modifier = Modifier) {
    AndroidView(
        factory = { context ->
            (android.view.LayoutInflater.from(context)
                    .inflate(R.layout.ytui_player, android.widget.FrameLayout(context), false)
                    as PlayerView)
                .apply {
                    useController = true
                    setShowSubtitleButton(true)
                    setShowShuffleButton(true)
                    setFullscreenButtonClickListener {
                        context.startActivity(Intent(context, FullscreenActivity::class.java))
                    }
                    VideoGestures.attach(this)
                }
        },
        modifier = modifier.background(Color.Black).testTag("native-video-surface"),
        update = { view ->
            val actual = PlaybackService.current?.session?.player
            if (view.player !== actual) view.player = actual
            view.keepScreenOn = actual?.playWhenReady == true
        },
        onRelease = {
            it.keepScreenOn = false
            it.player = null
        },
    )
}

/** A bounded screen: advanced controls open separately, never below a long cover. */
@Composable
internal fun FocusedPlayer(
    vm: YtuiViewModel,
    minimize: () -> Unit,
    onQueue: () -> Unit,
    onMore: () -> Unit,
) {
    val player by vm.player.collectAsState()
    val config by vm.config.collectAsState()
    val mobile by vm.mobile.collectAsState()
    val context = LocalContext.current
    val threshold = with(LocalDensity.current) { 72.dp.toPx() }
    val dismiss =
        Modifier.pointerInput(mobile.optBoolean("gestures", true)) {
            if (mobile.optBoolean("gestures", true)) {
                var distance = 0f
                detectVerticalDragGestures(
                    onDragStart = { distance = 0f },
                    onVerticalDrag = { change, amount ->
                        distance += amount
                        change.consume()
                    },
                    onDragEnd = { if (distance > threshold) minimize() },
                )
            }
        }
    BoxWithConstraints(Modifier.fillMaxSize().testTag("focused-player")) {
        val landscape = maxWidth > maxHeight * 1.25f
        @Composable
        fun Controls(modifier: Modifier) {
            Column(
                modifier,
                verticalArrangement = Arrangement.spacedBy(if (landscape) 4.dp else 8.dp),
            ) {
                Text(
                    player.title,
                    style = MaterialTheme.typography.titleLarge,
                    fontWeight = FontWeight.Bold,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                if (
                    mobile.optBoolean("show_channel", true) ||
                        player.media?.optBoolean("live") == true
                )
                    Text(
                        player.value.optString("station").ifBlank {
                            player.media?.optString("creator") ?: ""
                        },
                        style = MaterialTheme.typography.bodyMedium,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                if (player.video) player.media?.let { MediaInformation(it, mobile) }
                player.value
                    .optString("error")
                    .takeUnless { it.isBlank() || it == "null" }
                    ?.let { Text(it, color = MaterialTheme.colorScheme.error, maxLines = 2) }
                if (!player.video) PlayerSeek(vm, player)
                if (!player.video)
                    Row(
                        Modifier.fillMaxWidth(),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.SpaceEvenly,
                    ) {
                        MediaIconButton(Glyph.Shuffle, "Shuffle") { vm.command("shuffle") }
                        MediaIconButton(Glyph.Previous, "Previous track") { vm.command("previous") }
                        FilledIconButton(
                            { vm.command("pause") },
                            Modifier.size(64.dp).testTag("player-pause").semantics {
                                contentDescription = if (player.playing) "Pause" else "Play"
                            },
                        ) {
                            MediaIcon(
                                if (player.playing) Glyph.Pause else Glyph.Play,
                                Modifier.size(32.dp),
                            )
                        }
                        MediaIconButton(Glyph.Next, "Next track", "player-next") {
                            vm.command("next")
                        }
                        MediaIconButton(Glyph.Repeat, "Repeat one") { vm.command("repeat") }
                    }
                Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
                    MediaIconButton(Glyph.Queue, "Playback queue", action = onQueue)
                    if (player.video)
                        MediaIconButton(Glyph.Expand, "Fullscreen video", "fullscreen-button") {
                            context.startActivity(Intent(context, FullscreenActivity::class.java))
                        }
                    MediaIconButton(
                        Glyph.More,
                        "Playback options",
                        "player-options",
                        action = onMore,
                    )
                }
            }
        }
        @Composable
        fun Art(modifier: Modifier) {
            BoxWithConstraints(
                modifier.then(if (player.video) Modifier else dismiss),
                contentAlignment = Alignment.Center,
            ) {
                if (player.video)
                    VideoSurface(
                        Modifier.width(minOf(maxWidth, maxHeight * (16f / 9f)))
                            .aspectRatio(16f / 9f)
                    )
                else {
                    val ratio =
                        if (config.optString("music_player_style", "classic") == "classic") 1f
                        else 1.35f
                    Visualizer(
                        player,
                        config,
                        Modifier.width(minOf(maxWidth, maxHeight * ratio)).aspectRatio(ratio),
                    )
                }
            }
        }
        if (landscape)
            Row(Modifier.fillMaxSize(), horizontalArrangement = Arrangement.spacedBy(24.dp)) {
                Art(Modifier.weight(1f).fillMaxHeight())
                Controls(Modifier.weight(1f).align(Alignment.CenterVertically))
            }
        else
            Column(Modifier.fillMaxSize(), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                Art(Modifier.weight(1f).fillMaxWidth())
                Controls(Modifier.fillMaxWidth())
            }
    }
}

@Composable
internal fun PlayerSeek(vm: YtuiViewModel, player: PlayerState) {
    val live = player.media?.optBoolean("live") == true
    var drag by remember(player.media?.optString("url")) { mutableStateOf<Float?>(null) }
    if (live || player.duration <= 0f) {
        Text(
            if (live) "LIVE" else if (player.value.optBoolean("buffering")) "Buffering…" else "",
            Modifier.heightIn(min = 36.dp),
            color = MaterialTheme.colorScheme.primary,
            style = MaterialTheme.typography.labelMedium,
        )
    } else {
        Column {
            Slider(
                (drag ?: player.position).coerceIn(0f, player.duration),
                { drag = it },
                valueRange = 0f..player.duration,
                onValueChangeFinished = {
                    drag?.let { vm.command("seek", "seconds" to it) }
                    drag = null
                },
                modifier = Modifier.fillMaxWidth().testTag("seek-slider"),
            )
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
                Text(time(drag ?: player.position), style = MaterialTheme.typography.labelMedium)
                Text(time(player.duration), style = MaterialTheme.typography.labelMedium)
            }
        }
    }
}
