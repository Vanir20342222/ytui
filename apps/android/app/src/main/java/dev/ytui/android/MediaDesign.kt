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
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.drawscope.Fill
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.drawscope.scale
import androidx.compose.ui.graphics.luminance
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
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
    /** Multi-line inputs cannot be pills; keep a soft rectangle. */
    val fieldShape = RoundedCornerShape(16.dp)
    // Reuse the shared ytui dark/light accent values for a stable teal brand role.
    val brandOnDark = Color(0xff6ddbc1)
    val brandOnLight = Color(0xff006f62)
    val videoInk = Color.White
    val videoScrim = Color.Black.copy(alpha = 0.72f)
    /** Tinted glass behind native video transports: still 3:1 for white icons on white video. */
    val videoDisc = Color(0x80141A20)
    /** Light hairline that gives glass discs and panels a lit edge. */
    val videoGlassEdge = Color.White.copy(alpha = 0.26f)
    /** Glass sheen at the top of the fullscreen options panel. */
    val videoGlassSheen = Color.White.copy(alpha = 0.09f)
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
    Forward,
    Plus,
    Captions,
    Speed,
    Edit,
    Lock,
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
                    // Two interlocking chain links.
                    val round = androidx.compose.ui.geometry.CornerRadius(4f)
                    drawRoundRect(tint, Offset(2f, 8f), Size(11f, 8f), round, Stroke(1.8f))
                    drawRoundRect(tint, Offset(11f, 8f), Size(11f, 8f), round, Stroke(1.8f))
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
                Glyph.Forward -> outline(9f to 5f, 16f to 12f, 9f to 19f)
                Glyph.Plus -> {
                    line(12f, 5f, 12f, 19f)
                    line(5f, 12f, 19f, 12f)
                }
                Glyph.Captions -> {
                    drawRoundRect(
                        tint,
                        Offset(2.5f, 5f),
                        Size(19f, 14f),
                        androidx.compose.ui.geometry.CornerRadius(2.5f),
                        style = Stroke(1.8f),
                    )
                    line(6.5f, 11f, 10.5f, 11f)
                    line(12.5f, 11f, 17.5f, 11f)
                    line(6.5f, 15f, 13.5f, 15f)
                    line(15.5f, 15f, 17.5f, 15f)
                }
                Glyph.Speed -> {
                    drawArc(
                        tint,
                        150f,
                        240f,
                        false,
                        Offset(3f, 4f),
                        Size(18f, 18f),
                        style = Stroke(1.8f, cap = StrokeCap.Round),
                    )
                    line(12f, 13f, 16.5f, 8.5f)
                    drawCircle(tint, 1.6f, Offset(12f, 13f))
                }
                Glyph.Edit -> {
                    outline(4f to 20f, 4f to 16f, 15f to 5f, 19f to 9f, 8f to 20f, closed = true)
                    line(13f, 7f, 17f, 11f)
                }
                Glyph.Lock -> {
                    drawRoundRect(
                        tint,
                        Offset(5f, 10.5f),
                        Size(14f, 10f),
                        CornerRadius(2f, 2f),
                        style = if (filled) Fill else Stroke(1.8f),
                    )
                    drawPath(
                        Path().apply {
                            moveTo(8.5f, 10.5f)
                            lineTo(8.5f, 7.5f)
                            cubicTo(8.5f, 2.8f, 15.5f, 2.8f, 15.5f, 7.5f)
                            lineTo(15.5f, 10.5f)
                        },
                        tint,
                        style = Stroke(1.8f, cap = StrokeCap.Round),
                    )
                }
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
internal fun MediaModeButton(
    glyph: Glyph,
    label: String,
    tag: String,
    checked: Boolean,
    action: () -> Unit,
) {
    IconToggleButton(
        checked,
        { action() },
        Modifier.size(48.dp).testTag(tag).semantics {
            contentDescription = label
            selected = checked
        },
        colors =
            IconButtonDefaults.iconToggleButtonColors(
                contentColor = MaterialTheme.colorScheme.onSurfaceVariant,
                checkedContentColor = MaterialTheme.colorScheme.primary,
            ),
    ) {
        MediaIcon(glyph, Modifier.size(20.dp))
    }
}

@Composable
internal fun SettingGroup(
    title: String,
    subtitle: String = "",
    showTitle: Boolean = true,
    content: @Composable ColumnScope.() -> Unit,
) {
    Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
        if (showTitle)
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
    modifier: Modifier = Modifier,
    leadingIcon: @Composable (() -> Unit)? = null,
    enabled: Boolean = true,
    trailing: @Composable (() -> Unit)? = null,
) {
    val content =
        if (enabled) LocalContentColor.current
        else MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.6f)
    CompositionLocalProvider(LocalContentColor provides content) {
        Row(
            modifier
                .fillMaxWidth()
                .heightIn(min = 56.dp)
                .clickable(enabled = enabled, onClick = onClick)
                .padding(horizontal = 8.dp, vertical = 8.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(20.dp),
        ) {
            leadingIcon?.invoke()
            Box(Modifier.weight(1f)) { text() }
            trailing?.invoke()
        }
    }
}

/**
 * Menu/settings row: leading outlined icon, title, optional grey subtitle and a trailing current
 * value with a chevron (YouTube quality/speed/settings menus).
 */
@Composable
internal fun ValueRow(
    title: String,
    modifier: Modifier = Modifier,
    value: String = "",
    glyph: Glyph? = null,
    subtitle: String = "",
    enabled: Boolean = true,
    chevron: Boolean = true,
    onClick: () -> Unit,
) {
    SheetAction(
        modifier = modifier,
        enabled = enabled,
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
                Text(title, style = MaterialTheme.typography.bodyLarge)
                if (subtitle.isNotBlank())
                    Text(
                        subtitle,
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
            }
        },
        leadingIcon =
            glyph?.let { { MediaIcon(it, tint = MaterialTheme.colorScheme.onSurfaceVariant) } },
        trailing = {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(4.dp),
            ) {
                if (value.isNotBlank())
                    Text(
                        value,
                        Modifier.widthIn(max = 160.dp),
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                if (chevron)
                    MediaIcon(
                        Glyph.Forward,
                        Modifier.size(20.dp),
                        MaterialTheme.colorScheme.onSurfaceVariant,
                    )
            }
        },
        onClick = onClick,
    )
}

/** Secondary pill: outlined, neutral ink. Pair with at most one primary [Action]. */
@Composable
internal fun SecondaryAction(
    label: String,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    glyph: Glyph? = null,
    action: () -> Unit,
) {
    OutlinedButton(
        action,
        modifier.heightIn(min = 48.dp),
        enabled = enabled,
        shape = MediaTokens.pill,
        border = BorderStroke(1.dp, MaterialTheme.colorScheme.outlineVariant),
        colors =
            ButtonDefaults.outlinedButtonColors(contentColor = MaterialTheme.colorScheme.onSurface),
        contentPadding = PaddingValues(horizontal = 16.dp, vertical = 10.dp),
    ) {
        if (glyph != null) {
            MediaIcon(glyph, Modifier.size(18.dp))
            Spacer(Modifier.width(8.dp))
        }
        Text(label, style = MaterialTheme.typography.labelLarge)
    }
}

/** Tertiary text action in neutral ink; destructive actions use the error role. */
@Composable
internal fun QuietAction(
    label: String,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    destructive: Boolean = false,
    action: () -> Unit,
) {
    TextButton(
        action,
        modifier.heightIn(min = 48.dp),
        enabled = enabled,
        colors =
            ButtonDefaults.textButtonColors(
                contentColor =
                    if (destructive) MaterialTheme.colorScheme.error
                    else MaterialTheme.colorScheme.onSurface
            ),
    ) {
        Text(label, style = MaterialTheme.typography.labelLarge)
    }
}

/** Neutral filled input matching the search pill; label keeps native semantics. */
@Composable
internal fun MediaTextField(
    value: String,
    onValueChange: (String) -> Unit,
    label: String,
    modifier: Modifier = Modifier,
    singleLine: Boolean = true,
    keyboardOptions: androidx.compose.foundation.text.KeyboardOptions =
        androidx.compose.foundation.text.KeyboardOptions.Default,
    visualTransformation: androidx.compose.ui.text.input.VisualTransformation =
        androidx.compose.ui.text.input.VisualTransformation.None,
    minLines: Int = 1,
    keyboardActions: androidx.compose.foundation.text.KeyboardActions =
        androidx.compose.foundation.text.KeyboardActions.Default,
) {
    TextField(
        value,
        onValueChange,
        modifier.fillMaxWidth(),
        keyboardActions = keyboardActions,
        label = { Text(label, maxLines = 1, overflow = TextOverflow.Ellipsis) },
        singleLine = singleLine,
        minLines = minLines,
        keyboardOptions = keyboardOptions,
        visualTransformation = visualTransformation,
        shape = if (singleLine) MediaTokens.pill else MediaTokens.fieldShape,
        colors =
            TextFieldDefaults.colors(
                focusedIndicatorColor = MediaTokens.transparent,
                unfocusedIndicatorColor = MediaTokens.transparent,
                disabledIndicatorColor = MediaTokens.transparent,
                focusedContainerColor = MaterialTheme.colorScheme.surfaceContainer,
                unfocusedContainerColor = MaterialTheme.colorScheme.surfaceContainer,
            ),
    )
}

/** Small section header used inside sheets and settings pages. */
@Composable
internal fun SectionHeader(title: String, modifier: Modifier = Modifier) {
    Text(
        title,
        modifier.padding(top = 16.dp, bottom = 4.dp, start = 4.dp),
        style = MaterialTheme.typography.labelLarge,
        fontWeight = FontWeight.Bold,
    )
}

/**
 * Quiet list row for queues, history, offline media and playlists: a small-radius thumbnail,
 * two-line title, one-line metadata and an optional trailing control.
 */
@Composable
internal fun MediaListRow(
    title: String,
    metadata: String,
    modifier: Modifier = Modifier,
    artwork: @Composable (Modifier) -> Unit,
    square: Boolean = false,
    highlighted: Boolean = false,
    enabled: Boolean = true,
    onClick: (() -> Unit)? = null,
    extra: @Composable (ColumnScope.() -> Unit)? = null,
    trailing: @Composable (() -> Unit)? = null,
) {
    Row(
        modifier
            .fillMaxWidth()
            .heightIn(min = 64.dp)
            .background(
                if (highlighted) MaterialTheme.colorScheme.surfaceContainer
                else MediaTokens.transparent,
                MediaTokens.medium,
            )
            .then(
                if (onClick != null) Modifier.clickable(enabled = enabled, onClick = onClick)
                else Modifier
            )
            .padding(vertical = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        artwork(
            Modifier.width(if (square) 56.dp else 96.dp)
                .aspectRatio(if (square) 1f else 16f / 9f)
                .clip(MediaTokens.small)
        )
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
            Text(
                title,
                style = MaterialTheme.typography.titleSmall,
                fontWeight = FontWeight.Medium,
                maxLines = 2,
                overflow = TextOverflow.Ellipsis,
                color =
                    if (enabled) MaterialTheme.colorScheme.onSurface
                    else MaterialTheme.colorScheme.onSurfaceVariant,
            )
            if (metadata.isNotBlank())
                Text(
                    metadata,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            extra?.invoke(this)
        }
        trailing?.invoke()
    }
}

/** Neutral placeholder tile when an item has no artwork (offline playlists, transfers). */
@Composable
internal fun GlyphTile(glyph: Glyph, modifier: Modifier) {
    Box(
        modifier.background(MaterialTheme.colorScheme.surfaceContainerHigh),
        contentAlignment = Alignment.Center,
    ) {
        MediaIcon(glyph, tint = MaterialTheme.colorScheme.onSurfaceVariant)
    }
}
