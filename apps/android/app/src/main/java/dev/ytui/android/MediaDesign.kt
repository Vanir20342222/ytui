package dev.ytui.android

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.drawscope.scale
import androidx.compose.ui.graphics.luminance
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

internal val LocalReducedMotion = staticCompositionLocalOf { false }

/** Shared visual roles. Palette colours remain owned by the shared Rust theme. */
internal object MediaTokens {
    // Android-only neutral rendering of ytui-dark; shared/desktop palettes stay unchanged.
    val neutralBackground = Color.Black.copy(red = 0.06f, green = 0.06f, blue = 0.06f)
    val neutralSurface = Color.Black.copy(red = 0.14f, green = 0.14f, blue = 0.14f)
    val neutralText = Color.White.copy(red = 0.94f, green = 0.94f, blue = 0.94f)
    val grid = 4.dp
    val textInset = 16.dp
    val small = RoundedCornerShape(4.dp)
    val medium = RoundedCornerShape(8.dp)
    val pill = RoundedCornerShape(50)
    val sheet = RoundedCornerShape(topStart = 28.dp, topEnd = 28.dp)
    val dialog = RoundedCornerShape(28.dp)
    // Reuse the shared ytui dark/light accent values for a stable teal brand role.
    val brandOnDark = Color(0xff6ddbc1)
    val brandOnLight = Color(0xff006f62)
    val videoInk = Color.White
    val videoScrim = Color.Black.copy(alpha = 0.72f)
    val videoDisc = Color.Black.copy(alpha = 0.48f)
    val transparent = Color.Transparent
    val typography =
        Typography(
            titleLarge =
                TextStyle(fontSize = 20.sp, lineHeight = 26.sp, fontWeight = FontWeight.Medium),
            titleMedium =
                TextStyle(fontSize = 16.sp, lineHeight = 22.sp, fontWeight = FontWeight.SemiBold),
            titleSmall =
                TextStyle(fontSize = 16.sp, lineHeight = 22.sp, fontWeight = FontWeight.SemiBold),
            bodySmall = TextStyle(fontSize = 13.sp, lineHeight = 18.sp),
            labelLarge =
                TextStyle(fontSize = 14.sp, lineHeight = 20.sp, fontWeight = FontWeight.Medium),
            labelMedium =
                TextStyle(fontSize = 13.sp, lineHeight = 18.sp, fontWeight = FontWeight.Medium),
        )
}

/** Ensure secondary text remains AA-readable on raised surfaces, including imports. */
internal fun mediaContrast(a: Color, b: Color): Float =
    (maxOf(a.luminance(), b.luminance()) + 0.05f) / (minOf(a.luminance(), b.luminance()) + 0.05f)

internal fun mediaReadableText(candidate: Color, background: Color): Color {
    if (mediaContrast(candidate, background) >= 4.5f) return candidate
    val end = mediaOnAccent(background)
    for (step in 1..20) {
        val result = androidx.compose.ui.graphics.lerp(candidate, end, step / 20f)
        if (mediaContrast(result, background) >= 4.5f) return result
    }
    return end
}

internal fun mediaBrand(background: Color): Color {
    val candidate =
        if (
            mediaContrast(MediaTokens.brandOnDark, background) >=
                mediaContrast(MediaTokens.brandOnLight, background)
        )
            MediaTokens.brandOnDark
        else MediaTokens.brandOnLight
    return mediaReadableText(candidate, background)
}

/** Pick the stronger black/white contrast, including imported and light palettes. */
internal fun mediaOnAccent(accent: Color): Color =
    if (accent.luminance() > 0.179f) Color.Black else Color.White

internal fun mediaProgress(position: Float, duration: Float): Float =
    if (!position.isFinite() || !duration.isFinite() || duration <= 0f) 0f
    else (position / duration).coerceIn(0f, 1f)

/** Small original vector drawings: no icon font, bitmap pack or network dependency. */
internal enum class Glyph {
    Home,
    Music,
    Radio,
    Download,
    Search,
    Settings,
    Account,
    More,
    Play,
    Pause,
    Next,
    Previous,
    Back,
    Close,
    Queue,
    Save,
    Preview,
    Expand,
    Volume,
    Check,
    History,
    Lyrics,
    Sleep,
    Shuffle,
    Repeat,
    Link,
    Palette,
    Info,
    Up,
    Down,
    Stop,
    ArrowRight,
}

@Composable
internal fun MediaIcon(
    glyph: Glyph,
    modifier: Modifier = Modifier,
    tint: Color = LocalContentColor.current,
    filled: Boolean = false,
) {
    val cutout = MaterialTheme.colorScheme.background
    Canvas(modifier.size(24.dp)) {
        scale(size.width / 24f, size.height / 24f, Offset.Zero) {
            fun line(x: Float, y: Float, x2: Float, y2: Float) =
                drawLine(tint, Offset(x, y), Offset(x2, y2), 1.8f, StrokeCap.Round)
            fun outline(vararg points: Pair<Float, Float>, closed: Boolean = false) {
                val path =
                    Path().apply {
                        points.forEachIndexed { i, p ->
                            if (i == 0) moveTo(p.first, p.second) else lineTo(p.first, p.second)
                        }
                        if (closed) close()
                    }
                if (filled && closed) drawPath(path, tint)
                else drawPath(path, tint, style = Stroke(1.8f, cap = StrokeCap.Round))
            }
            fun play(x: Float = 8f) =
                drawPath(
                    Path().apply {
                        moveTo(x, 5f)
                        lineTo(x + 11f, 12f)
                        lineTo(x, 19f)
                        close()
                    },
                    tint,
                )
            when (glyph) {
                Glyph.Play -> play()
                Glyph.Pause -> {
                    drawRoundRect(tint, Offset(6f, 5f), Size(4f, 14f))
                    drawRoundRect(tint, Offset(14f, 5f), Size(4f, 14f))
                }
                Glyph.Next -> {
                    play(5f)
                    line(19f, 5f, 19f, 19f)
                }
                Glyph.Previous -> {
                    drawPath(
                        Path().apply {
                            moveTo(18f, 5f)
                            lineTo(7f, 12f)
                            lineTo(18f, 19f)
                            close()
                        },
                        tint,
                    )
                    line(5f, 5f, 5f, 19f)
                }
                Glyph.Home -> {
                    outline(3f to 10f, 12f to 3f, 21f to 10f)
                    outline(
                        5f to 9f,
                        5f to 21f,
                        10f to 21f,
                        10f to 14f,
                        14f to 14f,
                        14f to 21f,
                        19f to 21f,
                        19f to 9f,
                        closed = filled,
                    )
                }
                Glyph.Music -> {
                    outline(10f to 17f, 10f to 5f, 20f to 3f, 20f to 15f)
                    line(10f, 8f, 20f, 6f)
                    drawOval(
                        tint,
                        Offset(3f, 15f),
                        Size(7f, 5f),
                        style =
                            if (filled) androidx.compose.ui.graphics.drawscope.Fill
                            else Stroke(1.8f),
                    )
                    drawOval(
                        tint,
                        Offset(13f, 13f),
                        Size(7f, 5f),
                        style =
                            if (filled) androidx.compose.ui.graphics.drawscope.Fill
                            else Stroke(1.8f),
                    )
                }
                Glyph.Radio -> {
                    drawRoundRect(
                        tint,
                        Offset(3f, 8f),
                        Size(18f, 13f),
                        style =
                            if (filled) androidx.compose.ui.graphics.drawscope.Fill
                            else Stroke(1.8f),
                    )
                    line(5f, 8f, 18f, 3f)
                    drawCircle(
                        if (filled) cutout else tint,
                        3f,
                        Offset(8f, 15f),
                        style = Stroke(1.8f),
                    )
                    for (y in listOf(13f, 17f)) drawLine(
                        if (filled) cutout else tint,
                        Offset(14f, y),
                        Offset(18f, y),
                        1.8f,
                        StrokeCap.Round,
                    )
                }
                Glyph.Download -> {
                    if (filled)
                        outline(
                            9f to 3f,
                            15f to 3f,
                            15f to 10f,
                            19f to 10f,
                            12f to 17f,
                            5f to 10f,
                            9f to 10f,
                            closed = true,
                        )
                    else {
                        outline(7f to 11f, 12f to 16f, 17f to 11f)
                        line(12f, 3f, 12f, 16f)
                    }
                    outline(4f to 16f, 4f to 21f, 20f to 21f, 20f to 16f)
                }
                Glyph.Search -> {
                    drawCircle(tint, 6.5f, Offset(10f, 10f), style = Stroke(1.8f))
                    line(15f, 15f, 21f, 21f)
                }
                Glyph.Account -> {
                    drawCircle(tint, 9f, Offset(12f, 12f), style = Stroke(1.8f))
                    drawCircle(tint, 3f, Offset(12f, 9f), style = Stroke(1.8f))
                    drawArc(
                        tint,
                        200f,
                        140f,
                        false,
                        Offset(6f, 13f),
                        Size(12f, 10f),
                        style = Stroke(1.8f),
                    )
                }
                Glyph.Settings -> {
                    for (y in listOf(5f, 12f, 19f)) line(3f, y, 21f, y)
                    drawCircle(tint, 2.5f, Offset(8f, 5f))
                    drawCircle(tint, 2.5f, Offset(16f, 12f))
                    drawCircle(tint, 2.5f, Offset(10f, 19f))
                }
                Glyph.More -> for (y in listOf(5f, 12f, 19f)) drawCircle(tint, 1.7f, Offset(12f, y))
                Glyph.ArrowRight -> {
                    outline(14f to 5f, 21f to 12f, 14f to 19f)
                    line(3f, 12f, 21f, 12f)
                }
                Glyph.Back -> {
                    outline(12f to 4f, 4f to 12f, 12f to 20f)
                    line(4f, 12f, 21f, 12f)
                }
                Glyph.Close -> {
                    line(6f, 6f, 18f, 18f)
                    line(18f, 6f, 6f, 18f)
                }
                Glyph.Queue,
                Glyph.Lyrics -> {
                    line(3f, 5f, 21f, 5f)
                    line(3f, 10f, 17f, 10f)
                    line(3f, 15f, 13f, 15f)
                    if (glyph == Glyph.Queue) {
                        line(18f, 14f, 18f, 22f)
                        line(14f, 18f, 22f, 18f)
                    } else line(3f, 20f, 19f, 20f)
                }
                Glyph.Save ->
                    outline(6f to 3f, 18f to 3f, 18f to 21f, 12f to 17f, 6f to 21f, closed = true)
                Glyph.Preview -> {
                    drawRoundRect(tint, Offset(2f, 4f), Size(20f, 16f), style = Stroke(1.8f))
                    drawPath(
                        Path().apply {
                            moveTo(10f, 8f)
                            lineTo(16f, 12f)
                            lineTo(10f, 16f)
                            close()
                        },
                        tint,
                    )
                }
                Glyph.Expand -> {
                    outline(3f to 9f, 3f to 3f, 9f to 3f)
                    outline(15f to 3f, 21f to 3f, 21f to 9f)
                    outline(21f to 15f, 21f to 21f, 15f to 21f)
                    outline(9f to 21f, 3f to 21f, 3f to 15f)
                }
                Glyph.Volume -> {
                    outline(
                        3f to 9f,
                        7f to 9f,
                        12f to 4f,
                        12f to 20f,
                        7f to 15f,
                        3f to 15f,
                        closed = true,
                    )
                    drawArc(
                        tint,
                        -65f,
                        130f,
                        false,
                        Offset(10f, 3f),
                        Size(12f, 18f),
                        style = Stroke(1.8f),
                    )
                }
                Glyph.Check -> outline(4f to 12f, 10f to 18f, 21f to 6f)
                Glyph.History,
                Glyph.Sleep,
                Glyph.Info -> {
                    drawCircle(tint, 9f, Offset(12f, 12f), style = Stroke(1.8f))
                    if (glyph == Glyph.Info) {
                        line(12f, 10f, 12f, 17f)
                        drawCircle(tint, 1f, Offset(12f, 7f))
                    } else {
                        line(12f, 6f, 12f, 12f)
                        line(12f, 12f, 17f, 15f)
                    }
                }
                Glyph.Shuffle -> {
                    outline(3f to 5f, 6f to 5f, 17f to 19f, 21f to 19f)
                    outline(3f to 19f, 6f to 19f, 17f to 5f, 21f to 5f)
                    outline(18f to 2f, 21f to 5f, 18f to 8f)
                    outline(18f to 16f, 21f to 19f, 18f to 22f)
                }
                Glyph.Repeat -> {
                    outline(4f to 10f, 4f to 5f, 20f to 5f, 17f to 2f)
                    outline(20f to 14f, 20f to 19f, 4f to 19f, 7f to 22f)
                }
                Glyph.Link -> {
                    drawRoundRect(tint, Offset(3f, 6f), Size(10f, 12f), style = Stroke(1.8f))
                    drawRoundRect(tint, Offset(11f, 6f), Size(10f, 12f), style = Stroke(1.8f))
                    line(8f, 12f, 16f, 12f)
                }
                Glyph.Palette -> {
                    drawCircle(tint, 9f, Offset(12f, 12f), style = Stroke(1.8f))
                    for (p in listOf(8f to 8f, 15f to 7f, 17f to 13f, 9f to 16f)) drawCircle(
                        tint,
                        1.5f,
                        Offset(p.first, p.second),
                    )
                }
                Glyph.Up -> outline(5f to 15f, 12f to 8f, 19f to 15f)
                Glyph.Down -> outline(5f to 9f, 12f to 16f, 19f to 9f)
                Glyph.Stop -> drawRoundRect(tint, Offset(5f, 5f), Size(14f, 14f))
            }
        }
    }
}

@Composable
internal fun MediaChip(
    label: String,
    selected: Boolean = false,
    modifier: Modifier = Modifier,
    onClick: () -> Unit,
) {
    // All filter chips share this composable so their height cannot drift;
    // 48dp keeps the visible chip at Material's minimum touch-target size.
    FilterChip(
        selected = selected,
        onClick = onClick,
        modifier = modifier.heightIn(min = 48.dp),
        label = { Text(label, style = MaterialTheme.typography.labelLarge) },
        shape = MediaTokens.pill,
        border = BorderStroke(0.dp, MediaTokens.transparent),
        colors =
            FilterChipDefaults.filterChipColors(
                containerColor = MaterialTheme.colorScheme.surfaceContainerHigh,
                labelColor = MaterialTheme.colorScheme.onSurface,
                selectedContainerColor = MaterialTheme.colorScheme.onBackground,
                selectedLabelColor = MaterialTheme.colorScheme.background,
            ),
    )
}

@Composable
internal fun MediaIconButton(glyph: Glyph, label: String, tag: String = "", action: () -> Unit) {
    IconButton(action, Modifier.size(48.dp).testTag(tag).semantics { contentDescription = label }) {
        val secondary = glyph == Glyph.Shuffle || glyph == Glyph.Repeat
        MediaIcon(
            glyph,
            Modifier.size(if (secondary) 20.dp else 24.dp),
            if (secondary) MaterialTheme.colorScheme.onSurfaceVariant else LocalContentColor.current,
        )
    }
}

@Composable
internal fun SettingGroup(
    title: String,
    subtitle: String = "",
    content: @Composable ColumnScope.() -> Unit,
) {
    Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Text(
            title,
            style = MaterialTheme.typography.labelLarge,
            fontWeight = FontWeight.Bold,
            modifier = Modifier.padding(top = 16.dp, start = 4.dp),
        )
        if (subtitle.isNotBlank())
            Text(
                subtitle,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                style = MaterialTheme.typography.bodyMedium,
                modifier = Modifier.padding(horizontal = 4.dp),
            )
        Column(
            Modifier.fillMaxWidth(),
            verticalArrangement = Arrangement.spacedBy(12.dp),
            content = content,
        )
    }
}

@Composable
internal fun EmptyMediaState(
    icon: Glyph,
    title: String,
    detail: String,
    modifier: Modifier = Modifier,
    content: @Composable () -> Unit = {},
) {
    Column(
        modifier.fillMaxWidth().padding(horizontal = 24.dp, vertical = 36.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        MediaIcon(icon, Modifier.size(56.dp), MaterialTheme.colorScheme.onSurfaceVariant)
        Text(title, style = MaterialTheme.typography.titleLarge, fontWeight = FontWeight.Bold)
        Text(
            detail,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            style = MaterialTheme.typography.bodyMedium,
            textAlign = androidx.compose.ui.text.style.TextAlign.Center,
        )
        content()
    }
}

@Composable
internal fun ChannelAvatar(creator: String, size: androidx.compose.ui.unit.Dp) {
    Box(
        Modifier.size(size).background(MaterialTheme.colorScheme.surfaceContainerHigh, CircleShape),
        contentAlignment = Alignment.Center,
    ) {
        // Keep a complete code point for emoji-leading channel names.
        val initial =
            creator.trim().let { name ->
                if (name.isEmpty()) "·"
                else name.substring(0, name.offsetByCodePoints(0, 1)).uppercase()
            }
        Text(
            initial,
            color = MaterialTheme.colorScheme.onSurface,
            style = MaterialTheme.typography.labelLarge,
        )
    }
}

@Composable
internal fun SheetAction(
    text: @Composable () -> Unit,
    onClick: () -> Unit,
    leadingIcon: @Composable (() -> Unit)? = null,
) {
    Row(
        Modifier.fillMaxWidth()
            .heightIn(min = 56.dp)
            .clickable(onClick = onClick)
            .padding(horizontal = 8.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(20.dp),
    ) {
        leadingIcon?.invoke()
        Box(Modifier.weight(1f)) { text() }
    }
}
