package dev.ytui.android

import android.graphics.Bitmap
import androidx.activity.compose.BackHandler
import androidx.activity.compose.setContent
import androidx.compose.animation.core.withInfiniteAnimationFrameNanos
import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.drawscope.rotate
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.core.view.WindowCompat
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.repeatOnLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import androidx.media3.ui.PlayerView
import kotlin.math.*
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import org.json.JSONObject

/** Android presentation only. Provider/state operations go through the Rust repository. */
object ComposeHost {
    internal val signals = MutableStateFlow(0L)
    private val authWorking = MutableStateFlow(false)
    private val authStatus = MutableStateFlow("")

    @JvmStatic fun authBusy() = authWorking.value

    @JvmStatic
    fun setAuthBusy(value: Boolean) {
        authWorking.value = value
    }

    @JvmStatic
    fun authMessage(text: String) {
        authStatus.value = text
    }

    @JvmStatic
    fun resetAuth() {
        authWorking.value = false
        authStatus.value = ""
    }

    @JvmStatic
    fun attachAuth(activity: AuthActivity, web: android.webkit.WebView) {
        activity.setContent {
            val busy by authWorking.collectAsState()
            val message by authStatus.collectAsState()
            BackHandler { if (!busy) activity.cancelSession() }
            MaterialTheme {
                Column(
                    Modifier.fillMaxSize()
                        .safeDrawingPadding()
                        .imePadding()
                        .background(MaterialTheme.colorScheme.background)
                ) {
                    Text(
                        "YouTube sign-in",
                        Modifier.padding(12.dp),
                        style = MaterialTheme.typography.titleLarge,
                    )
                    Text(
                        "Use this session only after completing sign-in. If Google rejects this embedded page, cancel and import a cookie document/header from Account.",
                        Modifier.padding(horizontal = 12.dp),
                    )
                    Row(
                        Modifier.fillMaxWidth().padding(8.dp),
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
                        Action("Cancel", "sign-in-cancel", !busy) { activity.cancelSession() }
                        Action("Use this session", "sign-in-complete", !busy) {
                            activity.useSession()
                        }
                    }
                    if (busy) LinearProgressIndicator(Modifier.fillMaxWidth())
                    if (message.isNotBlank())
                        Text(
                            message,
                            Modifier.padding(12.dp),
                            color = MaterialTheme.colorScheme.error,
                        )
                    AndroidView(factory = { web }, modifier = Modifier.weight(1f).fillMaxWidth())
                }
            }
        }
    }

    @JvmStatic
    fun changed() {
        signals.update { it + 1 }
    }

    @JvmStatic
    fun attach(activity: RustActivity) {
        WindowCompat.setDecorFitsSystemWindows(activity.window, false)
        activity.setContent { YtuiApp(activity) }
    }
}

private fun JSONObject.color(key: String, fallback: Color): Color {
    val rgb = optJSONArray(key) ?: return fallback
    return Color(android.graphics.Color.rgb(rgb.optInt(0), rgb.optInt(1), rgb.optInt(2)))
}

private fun time(seconds: Float): String {
    val value = seconds.takeIf { it.isFinite() }?.toLong()?.coerceAtLeast(0) ?: 0
    return if (value >= 3600) "%d:%02d:%02d".format(value / 3600, value / 60 % 60, value % 60)
    else "%d:%02d".format(value / 60, value % 60)
}

@Composable
private fun YtuiApp(activity: RustActivity, vm: YtuiViewModel = viewModel()) {
    val dark = isSystemInDarkTheme()
    val palette by vm.palette.collectAsState()
    val config by vm.config.collectAsState()
    val tab by vm.tab.collectAsState()
    val account by vm.account.collectAsState()
    val playerOpen by vm.playerOpen.collectAsState()
    val status by vm.status.collectAsState()
    val ready by vm.ready.collectAsState()
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    val keyboard = LocalSoftwareKeyboardController.current
    val imeVisible = WindowInsets.ime.getBottom(LocalDensity.current) > 0
    var dialog by rememberSaveable { mutableStateOf("") }
    var menu by remember { mutableStateOf(false) }
    val background = palette.color("background", Color(0xff101418))
    val surface = palette.color("surface", Color(0xff202830))
    val foreground = palette.color("text", Color(0xffd5dee6))
    val accent = palette.color("accent", Color(0xff80cbc4))
    val light = palette.optBoolean("light")
    val colors =
        if (light)
            lightColorScheme(
                primary = accent,
                background = background,
                surface = surface,
                onBackground = foreground,
                onSurface = foreground,
                onPrimary = background,
                outline = palette.color("muted", foreground),
            )
        else
            darkColorScheme(
                primary = accent,
                background = background,
                surface = surface,
                onBackground = foreground,
                onSurface = foreground,
                onPrimary = background,
                outline = palette.color("muted", foreground),
            )
    LaunchedEffect(Unit) { vm.initialize(dark) }
    LaunchedEffect(dark) { vm.systemTheme(dark) }
    LaunchedEffect(lifecycle) {
        lifecycle.repeatOnLifecycle(Lifecycle.State.STARTED) {
            coroutineScope {
                launch { ComposeHost.signals.collect { vm.snapshot() } }
                launch {
                    while (isActive) {
                        delay(1000)
                        vm.snapshot()
                    }
                }
            }
        }
    }
    SideEffect {
        activity.window.statusBarColor = android.graphics.Color.TRANSPARENT
        activity.window.navigationBarColor = android.graphics.Color.TRANSPARENT
        WindowCompat.getInsetsController(activity.window, activity.window.decorView).apply {
            isAppearanceLightStatusBars = light
            isAppearanceLightNavigationBars = light
        }
    }
    BackHandler {
        if (imeVisible) keyboard?.hide()
        else if (dialog.isNotBlank()) dialog = ""
        else if (playerOpen) vm.playerOpen.value = false
        else if (!vm.back()) activity.moveTaskToBack(true)
    }
    MaterialTheme(
        colorScheme = colors,
        typography =
            Typography(
                bodyLarge =
                    androidx.compose.ui.text.TextStyle(
                        fontFamily = FontFamily.Monospace,
                        fontSize = 16.sp,
                    ),
                bodyMedium =
                    androidx.compose.ui.text.TextStyle(
                        fontFamily = FontFamily.Monospace,
                        fontSize = 14.sp,
                    ),
                labelLarge =
                    androidx.compose.ui.text.TextStyle(
                        fontFamily = FontFamily.Monospace,
                        fontSize = 14.sp,
                    ),
                titleLarge =
                    androidx.compose.ui.text.TextStyle(
                        fontFamily = FontFamily.Monospace,
                        fontSize = 22.sp,
                    ),
                titleMedium =
                    androidx.compose.ui.text.TextStyle(
                        fontFamily = FontFamily.Monospace,
                        fontSize = 18.sp,
                    ),
            ),
        shapes =
            Shapes(
                small = RoundedCornerShape(4.dp),
                medium = RoundedCornerShape(4.dp),
                large = RoundedCornerShape(6.dp),
            ),
    ) {
        Column(
            Modifier.fillMaxSize()
                .background(background)
                .testTag("app-background")
                .semantics { stateDescription = if (ready) "Ready" else "Loading settings" }
                .safeDrawingPadding()
                .imePadding()
        ) {
            Row(
                Modifier.fillMaxWidth().padding(8.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Text("ytui", color = accent, style = MaterialTheme.typography.titleMedium)
                Text(
                    listOf("Downloads", "YouTube", "Music", "Radio")[tab],
                    Modifier.weight(1f),
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                Action(
                    if (account.optBoolean("connected")) "Account ●" else "Sign in",
                    "account-button",
                ) {
                    dialog = "account"
                }
                Action("Settings", "settings-button") { dialog = "settings" }
                Box {
                    Action("⋮", "menu-button") { menu = true }
                    DropdownMenu(menu, { menu = false }) {
                        for ((label, name) in
                            listOf(
                                "Playback queue" to "queue",
                                "Recently played" to "history",
                                "Lyrics" to "lyrics",
                                "Sleep timer" to "sleep",
                                "About" to "about",
                            )) {
                            DropdownMenuItem(
                                text = { Text(label) },
                                onClick = {
                                    menu = false
                                    dialog = name
                                    if (name == "queue") vm.loadQueue()
                                    if (name == "history") vm.loadHistory()
                                    if (name == "lyrics") vm.player.value.media?.let(vm::lyricLoad)
                                },
                            )
                        }
                        DropdownMenuItem(
                            text = { Text("Refresh feed") },
                            onClick = {
                                menu = false
                                vm.navigate(request = vm.feeds[tab].value.request, remember = false)
                            },
                        )
                    }
                }
            }
            HorizontalDivider()
            BoxWithConstraints(Modifier.weight(1f).fillMaxWidth()) {
                val tablet = maxWidth >= 840.dp
                Row(Modifier.fillMaxSize()) {
                    if (tablet)
                        Column(
                            Modifier.width(132.dp).fillMaxHeight().padding(8.dp),
                            verticalArrangement = Arrangement.spacedBy(8.dp),
                        ) {
                            TabButtons(tab, vm::chooseTab, vertical = true)
                        }
                    Box(Modifier.weight(1f).fillMaxHeight()) {
                        if (tab == 0) DownloadsPane(vm)
                        else
                            key(tab) {
                                BrowsePane(
                                    vm,
                                    tab,
                                    onAccount = { dialog = "account" },
                                    onFeeds = { dialog = "feeds" },
                                )
                            }
                    }
                    if (tablet) {
                        VerticalDivider()
                        Column(
                            Modifier.widthIn(max = 360.dp)
                                .fillMaxWidth(0.34f)
                                .fillMaxHeight()
                                .verticalScroll(rememberScrollState())
                                .padding(12.dp)
                        ) {
                            PlayerPane(
                                vm,
                                expanded = false,
                                showSurface = !playerOpen,
                                onQueue = {
                                    dialog = "queue"
                                    vm.loadQueue()
                                },
                            )
                        }
                    }
                }
            }
            if (LocalConfiguration.current.screenWidthDp < 840) PlayerDock(vm)
            Text(
                status,
                Modifier.fillMaxWidth().padding(horizontal = 10.dp, vertical = 4.dp),
                style = MaterialTheme.typography.bodySmall,
                maxLines = 2,
                overflow = TextOverflow.Ellipsis,
            )
            if (LocalConfiguration.current.screenWidthDp < 840)
                Row(
                    Modifier.fillMaxWidth().padding(6.dp),
                    horizontalArrangement = Arrangement.spacedBy(4.dp),
                ) {
                    TabButtons(tab, vm::chooseTab)
                }
        }
        if (playerOpen)
            Panel("Now playing", { vm.playerOpen.value = false }, full = true) {
                PlayerPane(
                    vm,
                    expanded = true,
                    onQueue = {
                        dialog = "queue"
                        vm.loadQueue()
                    },
                )
            }
        when (dialog) {
            "settings" -> SettingsPanel(vm, dark) { dialog = "" }
            "account" -> AccountPanel(vm) { dialog = "" }
            "feeds" -> FeedPanel(vm) { dialog = "" }
            "queue" -> QueuePanel(vm) { dialog = "" }
            "history" -> HistoryPanel(vm) { dialog = "" }
            "lyrics" ->
                LyricsPanel(vm) {
                    vm.lyricClose()
                    dialog = ""
                }
            "sleep" ->
                Panel("Sleep timer", { dialog = "" }) {
                    for (minutes in listOf(15, 30, 60, 120)) Action("$minutes minutes") {
                        vm.command("sleep", "seconds" to minutes * 60)
                        dialog = ""
                    }
                    Action("Cancel timer") {
                        vm.command("sleep_cancel")
                        dialog = ""
                    }
                }
            "about" ->
                Panel("About ytui", { dialog = "" }) {
                    Text(
                        "${BuildConfig.VERSION_NAME} (${BuildConfig.VERSION_CODE})\nRust engine · native Android UI\nSource ${BuildConfig.SOURCE_REVISION}\nDevelopment preview"
                    )
                    Text(
                        "All provider, extraction, account and library operations use the shared Rust engine. Android supplies touch, keyboard, video and background media."
                    )
                    Text(
                        "Real device/network acceptance and owner release signing remain required."
                    )
                    Action("Mobile documentation") {
                        vm.effect(
                            "open",
                            "url" to
                                "https://github.com/Vanir20342222/ytui/tree/feat/android-monorepo/docs/android",
                        )
                    }
                }
        }
    }
}

@Composable
private fun Action(label: String, tag: String = "", enabled: Boolean = true, action: () -> Unit) {
    OutlinedButton(
        action,
        Modifier.heightIn(min = 48.dp)
            .then(if (tag.isNotEmpty()) Modifier.testTag(tag) else Modifier),
        enabled = enabled,
        shape = RoundedCornerShape(4.dp),
        contentPadding = PaddingValues(horizontal = 12.dp, vertical = 8.dp),
    ) {
        Text(label)
    }
}

@Composable
private fun RowScope.TabButtons(tab: Int, choose: (Int) -> Unit, vertical: Boolean = false) {
    listOf("Files", "YouTube", "Music", "Radio").forEachIndexed { index, title ->
        OutlinedButton(
            { choose(index) },
            Modifier.weight(1f).heightIn(min = 48.dp).testTag("tab-$index"),
            colors =
                if (tab == index)
                    ButtonDefaults.outlinedButtonColors(
                        containerColor = MaterialTheme.colorScheme.primary.copy(alpha = 0.18f)
                    )
                else ButtonDefaults.outlinedButtonColors(),
            shape = RoundedCornerShape(4.dp),
            contentPadding = PaddingValues(4.dp),
        ) {
            Text(title, maxLines = 1)
        }
    }
}

@Composable
private fun ColumnScope.TabButtons(tab: Int, choose: (Int) -> Unit, vertical: Boolean) {
    listOf("Files", "YouTube", "Music", "Radio").forEachIndexed { index, title ->
        OutlinedButton(
            { choose(index) },
            Modifier.fillMaxWidth().heightIn(min = 48.dp).testTag("tab-$index"),
            colors =
                ButtonDefaults.outlinedButtonColors(
                    containerColor =
                        if (tab == index) MaterialTheme.colorScheme.primary.copy(alpha = 0.18f)
                        else Color.Transparent
                ),
            shape = RoundedCornerShape(4.dp),
        ) {
            Text(title)
        }
    }
}

@Composable
private fun Panel(
    title: String,
    close: () -> Unit,
    full: Boolean = false,
    scroll: Boolean = true,
    content: @Composable ColumnScope.() -> Unit,
) {
    Dialog(close, DialogProperties(usePlatformDefaultWidth = false)) {
        Surface(
            Modifier.fillMaxWidth(if (full) 1f else 0.94f)
                .widthIn(max = if (full) 1600.dp else 720.dp)
                .fillMaxHeight(if (full) 1f else 0.94f)
                .safeDrawingPadding()
                .imePadding(),
            shape = RoundedCornerShape(6.dp),
            color = MaterialTheme.colorScheme.surface,
        ) {
            Column(Modifier.padding(16.dp)) {
                Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                    Text(title, Modifier.weight(1f), style = MaterialTheme.typography.titleLarge)
                    Action("Close") { close() }
                }
                HorizontalDivider(Modifier.padding(vertical = 8.dp))
                Column(
                    Modifier.weight(1f)
                        .fillMaxWidth()
                        .then(
                            if (scroll) Modifier.verticalScroll(rememberScrollState()) else Modifier
                        ),
                    verticalArrangement = Arrangement.spacedBy(10.dp),
                    content = content,
                )
            }
        }
    }
}

@Composable
private fun BrowsePane(vm: YtuiViewModel, tab: Int, onAccount: () -> Unit, onFeeds: () -> Unit) {
    val feed by vm.feeds[tab].collectAsState()
    val shared by vm.sharedQuery.collectAsState()
    var query by rememberSaveable(tab) { mutableStateOf("") }
    val keyboard = LocalSoftwareKeyboardController.current
    val list = rememberLazyListState(initialFirstVisibleItemIndex = feed.scrollIndex)
    LaunchedEffect(list) {
        snapshotFlow { list.firstVisibleItemIndex }
            .distinctUntilChanged()
            .collect { vm.rememberScroll(tab, it) }
    }
    LaunchedEffect(feed.generation) {
        if (list.firstVisibleItemIndex != feed.scrollIndex) list.scrollToItem(feed.scrollIndex)
    }
    LaunchedEffect(shared) {
        if (shared.isNotBlank()) {
            query = shared
            vm.sharedQuery.value = ""
        }
    }
    LaunchedEffect(list, feed.continuation, feed.loading, feed.error) {
        snapshotFlow { list.layoutInfo.visibleItemsInfo.lastOrNull()?.index ?: 0 }
            .distinctUntilChanged()
            .collect { last ->
                if (
                    feed.cards.isNotEmpty() &&
                        last >= feed.cards.size - 5 &&
                        feed.continuation != null &&
                        !feed.loading &&
                        feed.error == null
                )
                    vm.more()
            }
    }
    Column(Modifier.fillMaxSize().padding(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Row(
            Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(6.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            OutlinedTextField(
                query,
                { query = it },
                Modifier.weight(1f).testTag("search-field"),
                label = { Text("Search or URL") },
                singleLine = true,
                keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
                keyboardActions =
                    KeyboardActions(
                        onSearch = {
                            keyboard?.hide()
                            vm.search(query)
                        }
                    ),
            )
            Action("Search", "search-button", query.isNotBlank()) {
                keyboard?.hide()
                vm.search(query)
            }
            Action("Feeds", "feeds-button") { onFeeds() }
        }
        if (feed.categories.isNotEmpty())
            LazyRow(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                items(feed.categories) { category ->
                    Action(category.optString("title")) {
                        vm.navigate(request = category.get("request"))
                    }
                }
            }
        LazyColumn(
            Modifier.weight(1f).fillMaxWidth().testTag("feed-list"),
            state = list,
            verticalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            item {
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    Action("Back") { vm.back() }
                    Text(feed.title, style = MaterialTheme.typography.titleMedium)
                }
            }
            if (feed.notice != null)
                item { Text(feed.notice!!, color = MaterialTheme.colorScheme.primary) }
            if (feed.error != null)
                item {
                    Surface(
                        color = MaterialTheme.colorScheme.errorContainer.copy(alpha = 0.25f),
                        shape = RoundedCornerShape(4.dp),
                    ) {
                        Column(
                            Modifier.padding(14.dp),
                            verticalArrangement = Arrangement.spacedBy(8.dp),
                        ) {
                            Text(
                                if (feed.errorCode in listOf("auth_required", "consent_required"))
                                    "Account needed"
                                else "This feed is unavailable",
                                style = MaterialTheme.typography.titleMedium,
                            )
                            Text(feed.error!!)
                            FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                                Action("Sign in / import cookies", "feed-account-action") {
                                    onAccount()
                                }
                                Action("Retry") { vm.retryFeed() }
                            }
                            Text(
                                "Search and video URLs remain available without a personalized feed."
                            )
                        }
                    }
                }
            items(feed.cards, key = { it.key }) { media -> MediaRow(vm, media, tab) }
            if (feed.loading)
                item {
                    LinearProgressIndicator(Modifier.fillMaxWidth())
                    Text("Loading feed…")
                }
            if (!feed.loading && feed.cards.isEmpty() && feed.error == null)
                item {
                    Text(
                        "Search for a video, song or station. Sign in for your personalized feeds."
                    )
                }
            if (feed.continuation != null && !feed.loading && feed.error == null)
                item { Action("Load more") { vm.more() } }
        }
    }
}

@Composable
private fun Artwork(media: MediaCard, modifier: Modifier, enabled: Boolean = true) {
    BoxWithConstraints(
        modifier.clip(RoundedCornerShape(4.dp)).background(MaterialTheme.colorScheme.surface)
    ) {
        val pixels = with(LocalDensity.current) { maxWidth.roundToPx().coerceIn(32, 768) }
        var bitmap by remember(media.key, pixels) { mutableStateOf<Bitmap?>(null) }
        LaunchedEffect(media.key, pixels, enabled) {
            bitmap =
                if (enabled) runCatching { ArtworkCache.load(media, pixels) }.getOrNull() else null
        }
        bitmap?.let {
            Image(
                it.asImageBitmap(),
                media.title,
                Modifier.fillMaxSize(),
                contentScale = ContentScale.Crop,
            )
        }
    }
}

@Composable
private fun MediaRow(vm: YtuiViewModel, media: MediaCard, tab: Int) {
    val config by vm.config.collectAsState()
    Surface(
        shape = RoundedCornerShape(4.dp),
        border = BorderStroke(1.dp, MaterialTheme.colorScheme.outline.copy(alpha = 0.4f)),
        color = MaterialTheme.colorScheme.background,
    ) {
        Row(
            Modifier.fillMaxWidth().padding(10.dp),
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Artwork(
                media,
                Modifier.width(if (tab == 2) 104.dp else 148.dp)
                    .height(if (tab == 2) 104.dp else 92.dp)
                    .clickable { vm.play(media) },
                config.optBoolean("thumbnails", true),
            )
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                Text(
                    media.title,
                    style = MaterialTheme.typography.titleMedium,
                    maxLines = 2,
                    overflow = TextOverflow.Ellipsis,
                )
                if (media.creator.isNotBlank())
                    Text(
                        media.creator,
                        maxLines = 1,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                val section = media.value.optString("section")
                if (section.isNotBlank())
                    Text(
                        section,
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.primary,
                    )
                FlowRow(horizontalArrangement = Arrangement.spacedBy(5.dp)) {
                    Action(if (media.collection) "Open" else if (media.live) "Live" else "Play") {
                        vm.play(media)
                    }
                    if (!media.collection && !media.live)
                        Action("Preview") { vm.play(media, preview = true) }
                    Action("Queue") { vm.enqueue(media) }
                    Action("Save") { vm.save(media) }
                    if (!media.live) Action("Download") { vm.play(media, download = true) }
                    if (media.value.optString("tracks_url").let { it.isNotBlank() && it != "null" })
                        Action("Tracks") { vm.tracks(media) }
                }
            }
        }
    }
}

@Composable
private fun PlayerDock(vm: YtuiViewModel) {
    val player by vm.player.collectAsState()
    if (player.title.isBlank()) return
    Row(
        Modifier.fillMaxWidth()
            .background(MaterialTheme.colorScheme.surface)
            .clickable { vm.playerOpen.value = true }
            .padding(8.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Column(Modifier.weight(1f)) {
            Text(player.title, maxLines = 1, overflow = TextOverflow.Ellipsis)
            Text(
                "${time(player.position)} / ${time(player.duration)}",
                style = MaterialTheme.typography.bodySmall,
            )
        }
        Action(if (player.playing) "Pause" else "Play", "dock-pause") { vm.command("pause") }
        Action("Next") { vm.command("next") }
    }
}

@Composable
private fun PlayerPane(
    vm: YtuiViewModel,
    expanded: Boolean,
    showSurface: Boolean = true,
    onQueue: () -> Unit,
) {
    val player by vm.player.collectAsState()
    val config by vm.config.collectAsState()
    val tab by vm.tab.collectAsState()
    var tracks by remember { mutableStateOf(false) }
    var speedMenu by remember { mutableStateOf(false) }
    Text(
        player.title.ifBlank { "Choose a video, song or station" },
        style = MaterialTheme.typography.titleLarge,
    )
    if (player.media == null) {
        Text("Select a catalog item or enter a video URL to start.")
        return
    }
    player.value
        .optString("station")
        .takeIf { it.isNotBlank() }
        ?.let { Text("Live · $it", color = MaterialTheme.colorScheme.primary) }
    if (player.video && showSurface && player.media != null) {
        AndroidView(
            factory = { context ->
                // TextureView participates in Compose dialog clipping and z-order;
                // media decoding remains in Media3's hardware decoder.
                android.view.LayoutInflater.from(context)
                    .inflate(R.layout.ytui_player, android.widget.FrameLayout(context), false)
                    as PlayerView
            },
            modifier =
                Modifier.fillMaxWidth().aspectRatio(16f / 9f).testTag("native-video-surface"),
            update = { view ->
                val actual = PlaybackService.current?.player
                if (view.player !== actual) view.player = actual
            },
            onRelease = { it.player = null },
        )
    } else if (showSurface && tab in 2..3 && player.media != null) Visualizer(player, config)
    player.value
        .optString("error")
        .takeUnless { it.isBlank() || it == "null" }
        ?.let { Text(it, color = MaterialTheme.colorScheme.error) }
    if (player.preview) {
        Text("Muted preview")
        FlowRow {
            Action("End preview") { vm.command("preview_end") }
            Action("Play full") { vm.promotePreview() }
        }
        return
    }
    if (player.duration > 0 && player.media?.optBoolean("live") != true) {
        Row(Modifier.fillMaxWidth()) {
            Text(time(player.position))
            Spacer(Modifier.weight(1f))
            Text(time(player.duration))
        }
        var seek by remember(player.media?.optString("url")) { mutableStateOf<Float?>(null) }
        Slider(
            (seek ?: player.position).coerceIn(0f, player.duration),
            { seek = it },
            valueRange = 0f..player.duration,
            onValueChangeFinished = {
                seek?.let { vm.command("seek", "seconds" to it) }
                seek = null
            },
            modifier = Modifier.fillMaxWidth().testTag("seek-slider"),
        )
        val segments = player.value.optJSONArray("segments")?.objects() ?: emptyList()
        if (segments.isNotEmpty()) {
            val accent = MaterialTheme.colorScheme.primary
            val muted = MaterialTheme.colorScheme.outline
            Canvas(Modifier.fillMaxWidth().height(8.dp).testTag("sponsor-marks")) {
                drawRect(muted.copy(alpha = 0.25f))
                segments.forEach { mark ->
                    val start = mark.optDouble("start").toFloat().coerceIn(0f, player.duration)
                    val end = mark.optDouble("end").toFloat().coerceIn(start, player.duration)
                    drawRect(
                        accent,
                        Offset(start / player.duration * size.width, 0f),
                        androidx.compose.ui.geometry.Size(
                            (end - start) / player.duration * size.width,
                            size.height,
                        ),
                    )
                }
            }
            Text(
                "Marked segments · tap to watch without skipping",
                style = MaterialTheme.typography.bodySmall,
            )
            LazyRow(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                items(segments) { mark ->
                    Action(
                        "${mark.optString("category")} ${time(mark.optDouble("start").toFloat())}"
                    ) {
                        vm.command(
                            "sponsor_watch",
                            "start" to mark.optDouble("start"),
                            "end" to mark.optDouble("end"),
                        )
                    }
                }
            }
        }
    }
    FlowRow(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
        Action("Previous") { vm.command("previous") }
        Action("−10s") { vm.command("back") }
        Action(if (player.playing) "Pause" else "Play", "player-pause") { vm.command("pause") }
        Action("+10s") { vm.command("forward") }
        Action("Next") { vm.command("next") }
    }
    Text("Volume ${(player.value.optDouble("volume", 0.7) * 100).roundToInt()}%")
    Slider(
        player.value.optDouble("volume", 0.7).toFloat().coerceIn(0f, 1f),
        { vm.command("volume", "value" to it) },
        modifier = Modifier.testTag("volume-slider"),
    )
    FlowRow(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
        Box {
            Action("Speed ${player.value.optDouble("speed", 1.0)}×") { speedMenu = true }
            DropdownMenu(speedMenu, { speedMenu = false }) {
                for (speed in listOf(0.5, 0.75, 1.0, 1.25, 1.5, 2.0)) DropdownMenuItem(
                    text = { Text("$speed×") },
                    onClick = {
                        speedMenu = false
                        vm.command("speed", "value" to speed)
                    },
                )
            }
        }
        Action("Queue") { onQueue() }
        Action("Shuffle") { vm.command("shuffle") }
        Action("Repeat") { vm.command("repeat") }
        Action("Tracks / captions") { tracks = !tracks }
        Action("Stop") { vm.command("stop") }
        Action("Replay skipped segment") { vm.command("sponsor_replay") }
    }
    if (
        player.video &&
            !player.value.optBoolean("offline") &&
            player.media?.optBoolean("live") != true
    ) {
        Choice(
            "Video quality",
            player.value.optString("quality", "1080"),
            listOf("360", "480", "720", "1080", "best"),
        ) {
            vm.command("quality", "value" to it)
        }
    }
    if (tracks) {
        Action("Toggle captions") { vm.command("captions_toggle") }
        player.value.optJSONArray("tracks")?.objects()?.forEach { track ->
            Action(track.optString("label")) {
                vm.command(
                    "track",
                    "group" to track.optInt("group"),
                    "index" to track.optInt("index"),
                )
            }
        }
    }
    if (!expanded) Action("Expand player") { vm.playerOpen.value = true }
}

@Composable
private fun Visualizer(player: PlayerState, config: JSONObject) {
    val style = config.optString("music_player_style", "classic")
    val reduced = config.optBoolean("reduced_motion")
    val lifecycleState by LocalLifecycleOwner.current.lifecycle.currentStateFlow.collectAsState()
    val visible = lifecycleState.isAtLeast(Lifecycle.State.STARTED)
    var phase by remember { mutableFloatStateOf(0f) }
    LaunchedEffect(player.playing, reduced, style, visible) {
        if (player.playing && !reduced && style != "classic" && visible) {
            var previous = 0L
            while (true) withInfiniteAnimationFrameNanos { now ->
                if (previous != 0L) phase = (phase + (now - previous) / 1_000_000_000f * 90) % 360
                previous = now
            }
        }
    }
    DisposableEffect(player.playing, style, reduced, visible) {
        AudioTap.enabled =
            visible &&
                player.playing &&
                !reduced &&
                (style.startsWith("vu") || style.startsWith("orbit"))
        onDispose { AudioTap.enabled = false }
    }
    val accent = MaterialTheme.colorScheme.primary
    val ink = MaterialTheme.colorScheme.onBackground
    val media = player.media?.let(::MediaCard) ?: return
    Box(
        Modifier.fillMaxWidth().height(240.dp).testTag("music-visualizer"),
        contentAlignment = Alignment.CenterStart,
    ) {
        if (style == "classic")
            Artwork(
                media,
                Modifier.size(228.dp).align(Alignment.Center),
                config.optBoolean("thumbnails", true),
            )
        else if (style.startsWith("vinyl")) {
            Canvas(Modifier.fillMaxSize()) {
                val radius = min(size.height * 0.48f, size.width * 0.32f)
                val center = Offset(size.width * 0.65f, size.height / 2)
                drawCircle(Color(0xff131516), radius, center)
                for (ring in 1..12) drawCircle(
                    ink.copy(alpha = 0.13f),
                    radius * ring / 12,
                    center,
                    style = Stroke(1.5f),
                )
                rotate(phase, center) {
                    drawLine(
                        ink.copy(alpha = 0.25f),
                        center,
                        center + Offset(radius * 0.85f, 0f),
                        5f,
                    )
                    drawCircle(accent, radius * 0.24f, center)
                    drawCircle(Color.Black, radius * 0.025f, center)
                }
            }
            Artwork(
                media,
                Modifier.fillMaxHeight(0.94f).aspectRatio(1f),
                config.optBoolean("thumbnails", true),
            )
        } else if (style.startsWith("vu"))
            Canvas(Modifier.fillMaxSize()) {
                val packed = if (AudioTap.enabled) AudioTap.levels else 0L
                val left = (packed and 65535) / 65535f
                val right = ((packed ushr 16) and 65535) / 65535f
                for (i in 0..23) {
                    val level = if (i < 12) left else right
                    val height = (0.08f + level * (0.5f + 0.5f * sin(phase / 30 + i))) * size.height
                    drawRect(
                        accent.copy(alpha = 0.5f + 0.5f * level),
                        Offset(i * size.width / 24, (size.height - height) / 2),
                        androidx.compose.ui.geometry.Size(size.width / 30, height),
                    )
                }
            }
        else
            Canvas(Modifier.fillMaxSize()) {
                val packed = if (AudioTap.enabled) AudioTap.levels else 0L
                val left = (packed and 65535) / 65535f
                val right = ((packed ushr 16) and 65535) / 65535f
                val center = Offset(size.width / 2, size.height / 2)
                val radius = size.minDimension * 0.28f
                drawCircle(accent.copy(alpha = 0.2f), radius, center, style = Stroke(3f))
                for (i in 0..11) {
                    val angle = phase / 180 * PI + i * PI / 6
                    val r = radius * (1 + (left + right) * 0.4f)
                    drawCircle(
                        accent,
                        4 + (left + right) * 10,
                        center + Offset((cos(angle) * r).toFloat(), (sin(angle) * r).toFloat()),
                    )
                }
            }
    }
}

@Composable
private fun SettingsPanel(vm: YtuiViewModel, dark: Boolean, close: () -> Unit) {
    val config by vm.config.collectAsState()
    val themes by vm.themes.collectAsState()
    val status by vm.status.collectAsState()
    var themeMenu by remember { mutableStateOf(false) }
    var styleMenu by remember { mutableStateOf(false) }
    var proxy by
        remember(config.optString("proxy")) {
            mutableStateOf(config.optString("proxy").takeUnless { it == "null" } ?: "")
        }
    Panel("Settings", close) {
        Text("Appearance", style = MaterialTheme.typography.titleMedium)
        Box {
            Action("Theme: ${config.optString("theme", "system")}", "theme-selector") {
                themeMenu = true
            }
            DropdownMenu(themeMenu, { themeMenu = false }) {
                themes.forEach { name ->
                    DropdownMenuItem(
                        text = { Text(name) },
                        modifier = Modifier.testTag("theme-$name"),
                        onClick = {
                            themeMenu = false
                            vm.updateConfig(dark) { it.put("theme", name) }
                        },
                    )
                }
            }
        }
        Text("Theme changes apply immediately and are saved.")
        Action("Import Omarchy / Caelestia palette") { vm.effect("pick", "kind" to "theme") }
        SettingToggle("Cached thumbnails", config.optBoolean("thumbnails", true)) { value ->
            vm.updateConfig(dark) { it.put("thumbnails", value) }
        }
        SettingToggle("Reduced motion", config.optBoolean("reduced_motion")) { value ->
            vm.updateConfig(dark) { it.put("reduced_motion", value) }
        }
        SettingToggle("Local history and resume", config.optBoolean("playback_history", true)) {
            value ->
            vm.updateConfig(dark) { it.put("playback_history", value) }
        }
        Box {
            Action("Music player: ${config.optString("music_player_style", "classic")}") {
                styleMenu = true
            }
            DropdownMenu(styleMenu, { styleMenu = false }) {
                for (style in
                    listOf(
                        "classic",
                        "vinyl",
                        "vinyl-fullscreen",
                        "vu",
                        "vu-fullscreen",
                        "orbit",
                        "orbit-fullscreen",
                    )) DropdownMenuItem(
                    text = { Text(style) },
                    onClick = {
                        styleMenu = false
                        vm.updateConfig(dark) { it.put("music_player_style", style) }
                    },
                )
            }
        }
        Text("Playback / downloads", style = MaterialTheme.typography.titleMedium)
        Choice(
            "Maximum video quality",
            config.optJSONObject("download")?.optString("quality", "1080") ?: "1080",
            listOf("360", "480", "720", "1080", "best"),
        ) { value ->
            vm.updateConfig(dark) { it.getJSONObject("download").put("quality", value) }
        }
        SettingToggle(
            "Audio-only downloads",
            config.optJSONObject("download")?.optBoolean("audio") == true,
        ) { value ->
            vm.updateConfig(dark) { it.getJSONObject("download").put("audio", value) }
        }
        Choice(
            "SponsorBlock",
            config.optString("playback_sponsorblock", "skip"),
            listOf("skip", "mark", "off"),
        ) { value ->
            vm.updateConfig(dark) { it.put("playback_sponsorblock", value) }
        }
        Text("Network", style = MaterialTheme.typography.titleMedium)
        OutlinedTextField(
            proxy,
            { proxy = it },
            Modifier.fillMaxWidth().testTag("proxy-field"),
            label = { Text("HTTP proxy (optional)") },
            singleLine = true,
        )
        Action("Apply network settings") {
            vm.updateConfig(dark) { it.put("proxy", proxy.trim().ifBlank { null }) }
        }
        Text("Your Android VPN works normally. Network changes apply to new connections.")
        Text(status)
    }
}

@Composable
private fun SettingToggle(title: String, value: Boolean, changed: (Boolean) -> Unit) {
    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
        Text(title, Modifier.weight(1f))
        Switch(value, changed)
    }
}

@Composable
private fun Choice(title: String, value: String, values: List<String>, changed: (String) -> Unit) {
    var open by remember { mutableStateOf(false) }
    Box {
        Action("$title: $value") { open = true }
        DropdownMenu(open, { open = false }) {
            values.forEach { item ->
                DropdownMenuItem(
                    text = { Text(item) },
                    onClick = {
                        open = false
                        changed(item)
                    },
                )
            }
        }
    }
}

@Composable
private fun AccountPanel(vm: YtuiViewModel, close: () -> Unit) {
    val account by vm.account.collectAsState()
    var header by remember { mutableStateOf("") }
    val status by vm.status.collectAsState()
    Panel("YouTube account", close) {
        Text(
            if (account.optBoolean("connected")) "Connected" else "Not connected",
            style = MaterialTheme.typography.titleMedium,
        )
        Text(
            "Android cannot read another browser's private cookies. Use this app's sign-in page, or explicitly import a cookie document/header."
        )
        Action("Open sign-in page", "account-sign-in") { vm.effect("login") }
        Text(
            "Google may reject embedded sign-in. Cookie document import is available independently."
        )
        Action("Import Netscape cookie file", "account-import") {
            vm.effect("pick", "kind" to "cookies")
        }
        OutlinedTextField(
            header,
            { header = it },
            Modifier.fillMaxWidth().testTag("cookie-header-field"),
            label = { Text("Cookie header (optional)") },
            visualTransformation = PasswordVisualTransformation(),
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password),
            minLines = 2,
        )
        Action("Import entered header", enabled = header.isNotBlank()) {
            vm.changeAccount("import_cookies", "header" to header)
            header = ""
        }
        Action("Reload stored account") { vm.changeAccount("reload_account") }
        Action("Disconnect") { vm.changeAccount("disconnect") }
        Text("Sessions are encrypted using Android Keystore. Never send cookies in a bug report.")
        Text(status)
    }
}

@Composable
private fun FeedPanel(vm: YtuiViewModel, close: () -> Unit) {
    val tab by vm.tab.collectAsState()
    var country by remember { mutableStateOf("") }
    var tag by remember { mutableStateOf("") }
    Panel("Feeds and library", close) {
        val choices =
            if (tab == 3) listOf("Cliamp channels" to "Cliamp", "Saved stations" to "Favorites")
            else
                listOf(
                    "For you" to "Home",
                    "Subscriptions" to "Subscriptions",
                    "Liked" to "Liked",
                    "Watch later" to "WatchLater",
                    "Library" to "Library",
                    "Saved locally" to "Favorites",
                )
        for ((title, request) in choices) Action(title) {
            vm.navigate(request = request)
            close()
        }
        if (tab == 3) {
            OutlinedTextField(
                country,
                { country = it },
                Modifier.fillMaxWidth(),
                label = { Text("Country name or two-letter code") },
            )
            Action("Browse country", enabled = country.isNotBlank()) {
                vm.navigate(request = json("RadioCountry" to country.trim()))
                close()
            }
            OutlinedTextField(
                tag,
                { tag = it },
                Modifier.fillMaxWidth(),
                label = { Text("Radio genre / tag") },
            )
            Action("Browse genre", enabled = tag.isNotBlank()) {
                vm.navigate(request = json("RadioTag" to tag.trim()))
                close()
            }
        }
    }
}

@Composable
private fun DownloadsPane(vm: YtuiViewModel) {
    val downloads by vm.downloads.collectAsState()
    var add by remember { mutableStateOf(false) }
    var url by rememberSaveable { mutableStateOf("") }
    Column(
        Modifier.fillMaxSize().padding(10.dp),
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Action("Add video / playlist URL", "add-download") { add = true }
        LazyColumn(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(10.dp)) {
            if (downloads.isEmpty())
                item { Text("Downloaded media and playlist preparation jobs appear here.") }
            items(downloads, key = { it.optString("id") }) { item ->
                Surface(
                    border = BorderStroke(1.dp, MaterialTheme.colorScheme.outline),
                    shape = RoundedCornerShape(4.dp),
                ) {
                    Column(Modifier.fillMaxWidth().padding(12.dp)) {
                        Text(item.optString("title"), style = MaterialTheme.typography.titleMedium)
                        Text(item.optString("state"))
                        LinearProgressIndicator(
                            progress = {
                                item.optDouble("progress", 0.0).toFloat().coerceIn(0f, 1f)
                            },
                            Modifier.fillMaxWidth(),
                        )
                        FlowRow(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                            if (item.optString("state") == "Complete") {
                                Action("Play offline") {
                                    vm.effect("offline", "id" to item.optString("id"))
                                }
                                Action("Export file…") {
                                    vm.effect("download_export", "id" to item.optString("id"))
                                }
                            }
                            Action("Pause / retry") {
                                vm.effect("download_toggle", "id" to item.optString("id"))
                            }
                            Action("Remove") {
                                vm.effect("download_remove", "id" to item.optString("id"))
                            }
                        }
                    }
                }
            }
        }
    }
    if (add)
        Panel("Add download", { add = false }) {
            OutlinedTextField(
                url,
                { url = it },
                Modifier.fillMaxWidth().testTag("download-url-field"),
                label = { Text("Video or dedicated playlist URL") },
                singleLine = true,
            )
            Text(
                "Audio-only and quality preferences are in Settings. A watch URL with list= keeps single-video intent."
            )
            Action("Download", enabled = url.isNotBlank()) {
                runCatching { vm.linked(url.trim()) }
                    .onSuccess {
                        vm.play(it, download = true)
                        add = false
                    }
                    .onFailure { vm.status.value = "Enter a valid HTTP(S) media URL" }
            }
        }
}

@Composable
private fun QueuePanel(vm: YtuiViewModel, close: () -> Unit) {
    val queue by vm.queue.collectAsState()
    Panel("Playback queue", close, scroll = false) {
        if (queue.isEmpty()) Text("Queue is empty")
        LazyColumn(
            Modifier.weight(1f).fillMaxWidth(),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            itemsIndexed(queue) { index, media ->
                Column {
                    Text(media.title)
                    FlowRow(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                        Action("Play") {
                            vm.play(media, queued = true)
                            close()
                        }
                        Action("↑", enabled = index > 0) { vm.moveQueue(index, index - 1, media) }
                        Action("↓", enabled = index + 1 < queue.size) {
                            vm.moveQueue(index, index + 1, media)
                        }
                        Action("Remove") { vm.removeQueue(index, media) }
                    }
                }
            }
        }
    }
}

@Composable
private fun HistoryPanel(vm: YtuiViewModel, close: () -> Unit) {
    val history by vm.recent.collectAsState()
    Panel("Recently played", close, scroll = false) {
        Action("Clear history and resume positions") { vm.command("history_clear") }
        if (history.isEmpty()) Text("Local history is empty")
        LazyColumn(Modifier.weight(1f).fillMaxWidth()) {
            items(history) { media ->
                Action(media.title) {
                    vm.play(media, resume = true)
                    close()
                }
            }
        }
    }
}

@Composable
private fun LyricsPanel(vm: YtuiViewModel, close: () -> Unit) {
    val player by vm.player.collectAsState()
    val stored by vm.lyricData.collectAsState()
    val versions by vm.lyricVersions.collectAsState()
    val imported by vm.importedLyrics.collectAsState()
    val media = remember { player.media?.copyJson() ?: JSONObject() }
    var title by remember { mutableStateOf(media.optString("title")) }
    var artist by remember { mutableStateOf(media.optString("creator")) }
    var text by remember { mutableStateOf("") }
    var offset by remember { mutableStateOf("0") }
    var selected by remember { mutableIntStateOf(0) }
    var follow by remember { mutableStateOf(false) }
    val list = rememberLazyListState()
    val lines =
        remember(stored, versions, selected) {
            (stored.optJSONObject("track") ?: versions.getOrNull(selected))
                ?.optJSONArray("lines")
                ?.objects() ?: emptyList()
        }
    val matching = player.media?.optString("url") == media.optString("url")
    val now = player.position * 1000 - (offset.toLongOrNull() ?: 0)
    val active =
        if (matching) lines.indexOfLast { !it.isNull("at_ms") && it.optLong("at_ms") <= now }
        else -1
    DisposableEffect(Unit) { onDispose { vm.lyricClose() } }
    LaunchedEffect(active, follow, matching) {
        if (follow && matching && active >= 0) list.animateScrollToItem(active + 1)
    }
    LaunchedEffect(stored) {
        stored.optJSONObject("document")?.let {
            title = it.optString("title")
            artist = it.optString("artist")
            text = it.optString("text")
            offset = it.optLong("offset_ms").toString()
        }
    }
    LaunchedEffect(imported) {
        imported?.let {
            text = it
            vm.importedLyrics.value = null
        }
    }
    Panel("Lyrics", close, scroll = false) {
        SettingToggle("Follow playing line", follow) { follow = it }
        if (!matching) Text("Timing paused: a different song is playing.")
        LazyColumn(
            Modifier.weight(1f).fillMaxWidth(),
            state = list,
            verticalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            item {
                Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                    if (media.length() == 0) Text("Play a song first")
                    OutlinedTextField(
                        title,
                        { title = it },
                        Modifier.fillMaxWidth(),
                        label = { Text("Song title") },
                    )
                    OutlinedTextField(
                        artist,
                        { artist = it },
                        Modifier.fillMaxWidth(),
                        label = { Text("Artist") },
                    )
                    Text(
                        "Only the reviewed title and artist are sent to LRCLIB when you choose Find."
                    )
                    FlowRow(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                        Action("Find lyrics", enabled = title.isNotBlank() && artist.isNotBlank()) {
                            vm.lyricLookup(media, title, artist)
                        }
                        Action("Import LRC / text") { vm.lyricImport() }
                    }
                    versions.forEachIndexed { index, track ->
                        Action(
                            "${index + 1}. ${track.optString("artist")} · ${track.optString("album")}"
                        ) {
                            selected = index
                            vm.lyricData.value = JSONObject()
                            text =
                                track.optJSONArray("lines")?.objects()?.joinToString("\n") { line ->
                                    if (line.isNull("at_ms")) line.optString("text")
                                    else
                                        "[%02d:%02d.%03d]%s"
                                            .format(
                                                line.optLong("at_ms") / 60000,
                                                line.optLong("at_ms") / 1000 % 60,
                                                line.optLong("at_ms") % 1000,
                                                line.optString("text"),
                                            )
                                } ?: ""
                        }
                    }
                    OutlinedTextField(
                        text,
                        { if (it.toByteArray().size <= 512 * 1024) text = it },
                        Modifier.fillMaxWidth().height(220.dp),
                        label = { Text("LRC or plain lyrics") },
                    )
                    OutlinedTextField(
                        offset,
                        { offset = it },
                        label = { Text("Delay in milliseconds (positive = later)") },
                        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
                    )
                    FlowRow(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                        Action("Save locally", enabled = media.length() > 0 && text.isNotBlank()) {
                            vm.lyricSave(
                                media,
                                json(
                                    "title" to title,
                                    "artist" to artist,
                                    "text" to text,
                                    "offset_ms" to (offset.toLongOrNull() ?: 0),
                                ),
                            )
                        }
                        Action("Remove saved lyrics") { vm.lyricRemove(media) }
                    }
                }
            }
            itemsIndexed(lines) { index, line ->
                Text(
                    line.optString("text"),
                    Modifier.fillMaxWidth()
                        .background(
                            if (index == active)
                                MaterialTheme.colorScheme.primary.copy(alpha = 0.2f)
                            else Color.Transparent
                        )
                        .clickable(enabled = matching && !line.isNull("at_ms")) {
                            vm.command(
                                "seek",
                                "seconds" to
                                    ((line.optLong("at_ms") + (offset.toLongOrNull() ?: 0))
                                        .coerceAtLeast(0) / 1000.0),
                            )
                        }
                        .padding(10.dp),
                )
            }
        }
    }
}
