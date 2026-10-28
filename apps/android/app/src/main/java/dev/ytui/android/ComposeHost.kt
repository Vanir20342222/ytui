package dev.ytui.android

import android.graphics.Bitmap
import androidx.activity.compose.BackHandler
import androidx.activity.compose.setContent
import androidx.compose.animation.core.withInfiniteAnimationFrameNanos
import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.grid.*
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.selection.toggleable
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
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.drawscope.rotate
import androidx.compose.ui.graphics.lerp
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.font.FontWeight
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
import kotlinx.coroutines.flow.map
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
    val tab by vm.tab.collectAsState()
    val account by vm.account.collectAsState()
    val playerOpen by vm.playerOpen.collectAsState()
    val hasPlayer by
        remember(vm) { vm.player.map { it.media != null }.distinctUntilChanged() }
            .collectAsState(false)
    val status by vm.status.collectAsState()
    val ready by vm.ready.collectAsState()
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    val keyboard = LocalSoftwareKeyboardController.current
    val imeVisible = WindowInsets.ime.getBottom(LocalDensity.current) > 0
    var dialog by rememberSaveable { mutableStateOf("") }
    var menu by remember { mutableStateOf(false) }
    val snackbar = remember { SnackbarHostState() }
    val background = palette.color("background", Color(0xff101418))
    val surface = palette.color("surface", Color(0xff202830))
    val foreground = palette.color("text", Color(0xffd5dee6))
    val accent = palette.color("accent", Color(0xff80cbc4))
    val light = palette.optBoolean("light")
    val base = if (light) lightColorScheme() else darkColorScheme()
    val colors =
        base.copy(
            primary = accent,
            onPrimary = mediaOnAccent(accent),
            primaryContainer = lerp(background, accent, 0.16f),
            onPrimaryContainer = foreground,
            secondary = accent,
            secondaryContainer = lerp(surface, accent, 0.14f),
            onSecondaryContainer = foreground,
            background = background,
            onBackground = foreground,
            surface = surface,
            onSurface = foreground,
            onSurfaceVariant = lerp(background, foreground, 0.7f),
            surfaceVariant = lerp(background, surface, 0.65f),
            surfaceContainer = lerp(background, surface, 0.72f),
            surfaceContainerLow = lerp(background, surface, 0.4f),
            surfaceContainerHigh = surface,
            outline = lerp(background, foreground, 0.35f),
            outlineVariant = lerp(background, foreground, 0.12f),
        )
    LaunchedEffect(Unit) { vm.initialize(dark) }
    LaunchedEffect(dark) { vm.systemTheme(dark) }
    LaunchedEffect(status) {
        if (status.isNotBlank() && status != "Ready") snackbar.showSnackbar(status)
    }
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
        typography = Typography(),
        shapes =
            Shapes(
                small = RoundedCornerShape(12.dp),
                medium = RoundedCornerShape(20.dp),
                large = RoundedCornerShape(28.dp),
            ),
    ) {
        CompositionLocalProvider(LocalContentColor provides foreground) {
            Box(
                Modifier.fillMaxSize()
                    .background(background)
                    .testTag("app-background")
                    .semantics { stateDescription = if (ready) "Ready" else "Loading settings" }
                    .safeDrawingPadding()
                    .imePadding()
            ) {
                Column(Modifier.fillMaxSize()) {
                    Row(
                        Modifier.fillMaxWidth().heightIn(min = 64.dp).padding(horizontal = 16.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Box(
                            Modifier.size(32.dp).clip(RoundedCornerShape(10.dp)).background(accent),
                            contentAlignment = Alignment.Center,
                        ) {
                            MediaIcon(Glyph.Play, Modifier.size(22.dp), colors.onPrimary)
                        }
                        Text(
                            "ytui",
                            Modifier.padding(start = 9.dp).weight(1f),
                            style = MaterialTheme.typography.headlineSmall,
                            fontWeight = FontWeight.ExtraBold,
                            letterSpacing = (-1).sp,
                        )
                        MediaIconButton(
                            Glyph.Account,
                            if (account.optBoolean("connected")) "Your account" else "Sign in",
                            "account-button",
                        ) {
                            dialog = "account"
                        }
                        MediaIconButton(Glyph.Settings, "Settings", "settings-button") {
                            dialog = "settings"
                        }
                        Box {
                            MediaIconButton(Glyph.More, "More options", "menu-button") {
                                menu = true
                            }
                            DropdownMenu(menu, { menu = false }) {
                                for ((label, name, icon) in
                                    listOf(
                                        Triple("Playback queue", "queue", Glyph.Queue),
                                        Triple("Recently played", "history", Glyph.History),
                                        Triple("Lyrics", "lyrics", Glyph.Lyrics),
                                        Triple("Sleep timer", "sleep", Glyph.Sleep),
                                        Triple("About", "about", Glyph.Info),
                                    )) {
                                    DropdownMenuItem(
                                        text = { Text(label) },
                                        leadingIcon = { MediaIcon(icon) },
                                        onClick = {
                                            menu = false
                                            dialog = name
                                            if (name == "queue") vm.loadQueue()
                                            if (name == "history") vm.loadHistory()
                                            if (name == "lyrics")
                                                vm.player.value.media?.let(vm::lyricLoad)
                                        },
                                    )
                                }
                                if (tab > 0)
                                    DropdownMenuItem(
                                        text = { Text("Refresh feed") },
                                        onClick = {
                                            menu = false
                                            vm.navigate(
                                                request = vm.feeds[tab].value.request,
                                                remember = false,
                                            )
                                        },
                                    )
                            }
                        }
                    }
                    BoxWithConstraints(Modifier.weight(1f).fillMaxWidth()) {
                        val tablet = maxWidth >= 840.dp
                        Row(Modifier.fillMaxSize()) {
                            if (tablet) AppNavigation(tab, vm::chooseTab, vertical = true)
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
                            if (tablet && hasPlayer) {
                                Surface(
                                    Modifier.width(336.dp)
                                        .fillMaxHeight()
                                        .padding(start = 8.dp, end = 20.dp, bottom = 20.dp),
                                    shape = RoundedCornerShape(24.dp),
                                    color = colors.surfaceContainerLow,
                                ) {
                                    Column(
                                        Modifier.verticalScroll(rememberScrollState())
                                            .padding(20.dp),
                                        verticalArrangement = Arrangement.spacedBy(16.dp),
                                    ) {
                                        Text(
                                            "NOW PLAYING",
                                            style = MaterialTheme.typography.labelMedium,
                                            color = colors.onSurfaceVariant,
                                            letterSpacing = 1.8.sp,
                                        )
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
                    }
                    if (LocalConfiguration.current.screenWidthDp < 840) {
                        PlayerDock(vm)
                        AppNavigation(tab, vm::chooseTab)
                    }
                }
                SnackbarHost(
                    snackbar,
                    Modifier.align(Alignment.BottomCenter)
                        .padding(bottom = 88.dp, start = 16.dp, end = 16.dp),
                )
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
}

@Composable
private fun Action(label: String, tag: String = "", enabled: Boolean = true, action: () -> Unit) {
    FilledTonalButton(
        action,
        Modifier.heightIn(min = 48.dp)
            .then(if (tag.isNotEmpty()) Modifier.testTag(tag) else Modifier),
        enabled = enabled,
        shape = RoundedCornerShape(14.dp),
        contentPadding = PaddingValues(horizontal = 16.dp, vertical = 10.dp),
    ) {
        Text(label)
    }
}

@Composable
private fun AppNavigation(tab: Int, choose: (Int) -> Unit, vertical: Boolean = false) {
    val tabs =
        listOf(
            Triple(1, "YouTube", Glyph.Home),
            Triple(2, "Music", Glyph.Music),
            Triple(3, "Radio", Glyph.Radio),
            Triple(0, "Downloads", Glyph.Download),
        )
    if (vertical)
        NavigationRail(
            Modifier.fillMaxHeight().width(88.dp).testTag("navigation-rail"),
            containerColor = MaterialTheme.colorScheme.background,
        ) {
            Spacer(Modifier.height(12.dp))
            tabs.forEach { (index, title, icon) ->
                NavigationRailItem(
                    tab == index,
                    { choose(index) },
                    icon = { MediaIcon(icon) },
                    label = { Text(title, maxLines = 1) },
                    modifier = Modifier.padding(vertical = 8.dp).testTag("tab-$index"),
                )
            }
        }
    else
        NavigationBar(
            Modifier.testTag("bottom-navigation"),
            containerColor = MaterialTheme.colorScheme.background,
            tonalElevation = 0.dp,
            windowInsets = WindowInsets(0, 0, 0, 0),
        ) {
            tabs.forEach { (index, title, icon) ->
                NavigationBarItem(
                    tab == index,
                    { choose(index) },
                    icon = { MediaIcon(icon) },
                    label = { Text(title, maxLines = 1) },
                    modifier = Modifier.testTag("tab-$index"),
                )
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
            Modifier.widthIn(max = if (full) 1600.dp else 680.dp)
                .fillMaxWidth(if (full) 1f else 0.96f)
                .fillMaxHeight(if (full) 1f else 0.94f)
                .safeDrawingPadding()
                .imePadding(),
            shape = RoundedCornerShape(if (full) 0.dp else 28.dp),
            color = MaterialTheme.colorScheme.surface,
        ) {
            Column(Modifier.padding(24.dp)) {
                Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                    Text(
                        title,
                        Modifier.weight(1f),
                        style = MaterialTheme.typography.headlineSmall,
                        fontWeight = FontWeight.Bold,
                    )
                    TextButton(close, Modifier.heightIn(min = 48.dp)) { Text("Close") }
                }
                Spacer(Modifier.height(16.dp))
                Column(
                    Modifier.weight(1f)
                        .fillMaxWidth()
                        .then(
                            if (scroll) Modifier.verticalScroll(rememberScrollState()) else Modifier
                        ),
                    verticalArrangement = Arrangement.spacedBy(16.dp),
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
    val grid = rememberLazyGridState(initialFirstVisibleItemIndex = feed.scrollIndex)
    LaunchedEffect(grid) {
        snapshotFlow { grid.firstVisibleItemIndex }
            .distinctUntilChanged()
            .collect { vm.rememberScroll(tab, it) }
    }
    LaunchedEffect(feed.generation) {
        if (grid.firstVisibleItemIndex != feed.scrollIndex) grid.scrollToItem(feed.scrollIndex)
    }
    LaunchedEffect(shared) {
        if (shared.isNotBlank()) {
            query = shared
            vm.sharedQuery.value = ""
        }
    }
    LaunchedEffect(grid, feed.continuation, feed.loading, feed.error) {
        snapshotFlow { grid.layoutInfo.visibleItemsInfo.lastOrNull()?.index ?: 0 }
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
    Column(
        Modifier.fillMaxSize().padding(horizontal = 20.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f)) {
                Text(
                    feed.title,
                    style = MaterialTheme.typography.headlineMedium,
                    fontWeight = FontWeight.Bold,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                Text(
                    when (tab) {
                        2 -> "A soundtrack for every moment"
                        3 -> "Find your frequency"
                        else -> "Discover your next favourite"
                    },
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            TextButton(onFeeds, Modifier.heightIn(min = 48.dp).testTag("feeds-button")) {
                MediaIcon(if (tab == 3) Glyph.Radio else Glyph.Queue, Modifier.size(20.dp))
                Spacer(Modifier.width(8.dp))
                Text("Browse")
            }
        }
        TextField(
            query,
            { query = it },
            Modifier.fillMaxWidth().testTag("search-field"),
            placeholder = {
                Text(if (tab == 3) "Search stations or genres" else "Search or paste a link")
            },
            leadingIcon = { MediaIcon(Glyph.Search) },
            trailingIcon = {
                MediaIconButton(Glyph.ArrowRight, "Search", "search-button") {
                    keyboard?.hide()
                    vm.search(query)
                }
            },
            shape = RoundedCornerShape(28.dp),
            singleLine = true,
            colors =
                TextFieldDefaults.colors(
                    focusedIndicatorColor = Color.Transparent,
                    unfocusedIndicatorColor = Color.Transparent,
                    focusedContainerColor = MaterialTheme.colorScheme.surfaceContainer,
                    unfocusedContainerColor = MaterialTheme.colorScheme.surfaceContainer,
                ),
            keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
            keyboardActions =
                KeyboardActions(
                    onSearch = {
                        keyboard?.hide()
                        vm.search(query)
                    }
                ),
        )
        if (feed.categories.isNotEmpty())
            LazyRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                items(feed.categories) { category ->
                    SuggestionChip(
                        onClick = { vm.navigate(request = category.get("request")) },
                        label = { Text(category.optString("title")) },
                        shape = RoundedCornerShape(12.dp),
                        modifier = Modifier.heightIn(min = 48.dp),
                    )
                }
            }
        LazyVerticalGrid(
            columns = GridCells.Adaptive(if (tab == 2) 164.dp else 280.dp),
            modifier = Modifier.weight(1f).fillMaxWidth().testTag("feed-list"),
            state = grid,
            contentPadding = PaddingValues(bottom = 24.dp),
            horizontalArrangement = Arrangement.spacedBy(16.dp),
            verticalArrangement = Arrangement.spacedBy(24.dp),
        ) {
            if (feed.request != "Home" && feed.request != "Cliamp")
                item(span = { GridItemSpan(maxLineSpan) }) {
                    TextButton({ vm.back() }, Modifier.heightIn(min = 48.dp)) {
                        MediaIcon(Glyph.Back)
                        Spacer(Modifier.width(8.dp))
                        Text("Back to previous feed")
                    }
                }
            if (feed.notice != null)
                item(span = { GridItemSpan(maxLineSpan) }) {
                    Text(
                        feed.notice!!,
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            if (feed.error != null)
                item(span = { GridItemSpan(maxLineSpan) }) {
                    Surface(
                        color = MaterialTheme.colorScheme.surfaceContainerLow,
                        shape = RoundedCornerShape(24.dp),
                    ) {
                        Column(
                            Modifier.fillMaxWidth().padding(24.dp),
                            verticalArrangement = Arrangement.spacedBy(12.dp),
                        ) {
                            MediaIcon(
                                if (feed.errorCode in listOf("auth_required", "consent_required"))
                                    Glyph.Account
                                else Glyph.Radio,
                                Modifier.size(32.dp),
                                MaterialTheme.colorScheme.primary,
                            )
                            Text(
                                if (feed.errorCode in listOf("auth_required", "consent_required"))
                                    "Account needed"
                                else "This feed is unavailable",
                                style = MaterialTheme.typography.titleLarge,
                                fontWeight = FontWeight.SemiBold,
                            )
                            Text(feed.error!!, color = MaterialTheme.colorScheme.onSurfaceVariant)
                            FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                                if (feed.errorCode in listOf("auth_required", "consent_required"))
                                    Action("Sign in / import cookies", "feed-account-action") {
                                        onAccount()
                                    }
                                TextButton({ vm.retryFeed() }, Modifier.heightIn(min = 48.dp)) {
                                    Text("Try again")
                                }
                            }
                            Text(
                                "You can still search or open a video link.",
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                    }
                }
            items(feed.cards, key = { it.key }, contentType = { "media-card" }) { media ->
                MediaRow(vm, media, tab)
            }
            if (feed.loading)
                item(span = { GridItemSpan(maxLineSpan) }) {
                    Column(
                        Modifier.fillMaxWidth().padding(24.dp),
                        horizontalAlignment = Alignment.CenterHorizontally,
                        verticalArrangement = Arrangement.spacedBy(16.dp),
                    ) {
                        CircularProgressIndicator(Modifier.size(28.dp), strokeWidth = 2.dp)
                        Text(
                            "Finding something good…",
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
            if (!feed.loading && feed.cards.isEmpty() && feed.error == null)
                item(span = { GridItemSpan(maxLineSpan) }) {
                    EmptyMediaState(
                        if (tab == 2) Glyph.Music else if (tab == 3) Glyph.Radio else Glyph.Search,
                        if (tab == 2) "Your soundtrack starts here"
                        else if (tab == 3) "Tune into something new" else "Find something you love",
                        if (tab == 3) "Search a station, explore a genre or browse cliamp channels."
                        else "Search above or connect your YouTube account for a feed made for you.",
                    ) {
                        Action(if (tab == 3) "Browse stations" else "Explore feeds") { onFeeds() }
                    }
                }
            if (feed.continuation != null && !feed.loading && feed.error == null)
                item(span = { GridItemSpan(maxLineSpan) }) {
                    Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.Center) {
                        Action("Load more") { vm.more() }
                    }
                }
        }
    }
}

@Composable
private fun Artwork(media: MediaCard, modifier: Modifier, enabled: Boolean = true) {
    BoxWithConstraints(
        modifier
            .clip(RoundedCornerShape(16.dp))
            .background(MaterialTheme.colorScheme.surfaceContainer)
    ) {
        val pixels = with(LocalDensity.current) { maxWidth.roundToPx().coerceIn(32, 768) }
        var bitmap by remember(media.key, pixels, enabled) { mutableStateOf<Bitmap?>(null) }
        LaunchedEffect(media.key, pixels, enabled) {
            bitmap =
                if (enabled) runCatching { ArtworkCache.load(media, pixels) }.getOrNull() else null
        }
        if (bitmap != null)
            Image(
                bitmap!!.asImageBitmap(),
                media.title,
                Modifier.fillMaxSize().testTag("artwork-${media.key}").semantics {
                    stateDescription = "Ready"
                },
                contentScale = ContentScale.Crop,
            )
        else
            Box(
                Modifier.fillMaxSize()
                    .background(
                        Brush.linearGradient(
                            listOf(
                                MaterialTheme.colorScheme.primaryContainer,
                                MaterialTheme.colorScheme.surfaceContainerHigh,
                            )
                        )
                    ),
                contentAlignment = Alignment.Center,
            ) {
                MediaIcon(
                    if (media.audio || media.kind == "album") Glyph.Music else Glyph.Preview,
                    Modifier.size(36.dp),
                    MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.55f),
                )
            }
    }
}

@Composable
private fun MediaRow(vm: YtuiViewModel, media: MediaCard, tab: Int) {
    val config by vm.config.collectAsState()
    var menu by remember(media.key) { mutableStateOf(false) }
    val square = tab == 2 || media.kind in setOf("album", "song", "station")
    Column(Modifier.fillMaxWidth().testTag("media-card-${media.key}")) {
        Box(
            Modifier.fillMaxWidth()
                .aspectRatio(if (square) 1f else 16f / 9f)
                .clip(RoundedCornerShape(16.dp))
        ) {
            Artwork(
                media,
                Modifier.fillMaxSize().clickable { vm.play(media) },
                config.optBoolean("thumbnails", true),
            )
            if (media.live || !media.value.isNull("duration"))
                Surface(
                    Modifier.align(Alignment.BottomEnd).padding(10.dp),
                    color = Color.Black.copy(alpha = 0.8f),
                    shape = RoundedCornerShape(6.dp),
                ) {
                    Text(
                        if (media.live) "LIVE" else time(media.value.optLong("duration").toFloat()),
                        Modifier.padding(horizontal = 8.dp, vertical = 4.dp),
                        color = Color.White,
                        style = MaterialTheme.typography.labelSmall,
                    )
                }
            if (square)
                FilledIconButton(
                    { vm.play(media) },
                    Modifier.align(Alignment.BottomEnd).padding(12.dp).size(48.dp).semantics {
                        contentDescription = "Play ${media.title}"
                    },
                ) {
                    MediaIcon(Glyph.Play)
                }
        }
        Row(Modifier.fillMaxWidth().padding(top = 8.dp), verticalAlignment = Alignment.Top) {
            Column(
                Modifier.weight(1f).clickable { vm.play(media) }.padding(top = 3.dp),
                verticalArrangement = Arrangement.spacedBy(4.dp),
            ) {
                Text(
                    media.title,
                    style = MaterialTheme.typography.titleSmall,
                    fontWeight = FontWeight.SemiBold,
                    maxLines = 2,
                    overflow = TextOverflow.Ellipsis,
                )
                Text(
                    media.creator.ifBlank {
                        if (media.collection) media.kind.replaceFirstChar(Char::titlecase)
                        else if (tab == 3) "Radio station" else "YouTube"
                    },
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                media.value
                    .optString("section")
                    .takeIf { it.isNotBlank() }
                    ?.let {
                        Text(
                            it,
                            style = MaterialTheme.typography.labelSmall,
                            color = MaterialTheme.colorScheme.primary,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                    }
            }
            Box {
                MediaIconButton(
                    Glyph.More,
                    "More options for ${media.title}",
                    "media-menu-${media.key}",
                ) {
                    menu = true
                }
                DropdownMenu(menu, { menu = false }) {
                    fun execute(action: () -> Unit) {
                        menu = false
                        action()
                    }
                    DropdownMenuItem(
                        text = { Text(if (media.collection) "Open collection" else "Play now") },
                        leadingIcon = { MediaIcon(Glyph.Play) },
                        onClick = { execute { vm.play(media) } },
                    )
                    if (!media.collection && !media.live)
                        DropdownMenuItem(
                            text = { Text("Preview") },
                            leadingIcon = { MediaIcon(Glyph.Preview) },
                            onClick = { execute { vm.play(media, preview = true) } },
                        )
                    DropdownMenuItem(
                        text = { Text("Add to queue") },
                        leadingIcon = { MediaIcon(Glyph.Queue) },
                        onClick = { execute { vm.enqueue(media) } },
                    )
                    DropdownMenuItem(
                        text = { Text("Save to library") },
                        leadingIcon = { MediaIcon(Glyph.Save) },
                        onClick = { execute { vm.save(media) } },
                    )
                    if (!media.live)
                        DropdownMenuItem(
                            text = { Text("Download") },
                            leadingIcon = { MediaIcon(Glyph.Download) },
                            onClick = { execute { vm.play(media, download = true) } },
                        )
                    if (media.value.optString("tracks_url").let { it.isNotBlank() && it != "null" })
                        DropdownMenuItem(
                            text = { Text("Browse tracks") },
                            leadingIcon = { MediaIcon(Glyph.Music) },
                            onClick = { execute { vm.tracks(media) } },
                        )
                }
            }
        }
    }
}

@Composable
private fun PlayerDock(vm: YtuiViewModel) {
    val player by vm.player.collectAsState()
    val config by vm.config.collectAsState()
    val media = player.media?.let(::MediaCard) ?: return
    Surface(
        onClick = { vm.playerOpen.value = true },
        modifier =
            Modifier.fillMaxWidth()
                .padding(horizontal = 12.dp, vertical = 6.dp)
                .testTag("mini-player"),
        shape = RoundedCornerShape(20.dp),
        color = MaterialTheme.colorScheme.surfaceContainerHigh,
        tonalElevation = 2.dp,
    ) {
        Column {
            Row(
                Modifier.fillMaxWidth().padding(8.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(10.dp),
            ) {
                Artwork(media, Modifier.size(48.dp), config.optBoolean("thumbnails", true))
                Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(3.dp)) {
                    Text(
                        player.title,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                        style = MaterialTheme.typography.titleSmall,
                        fontWeight = FontWeight.SemiBold,
                    )
                    Text(
                        media.creator.ifBlank {
                            if (player.preview) "Muted preview"
                            else if (media.live) "Live radio" else "Now playing"
                        },
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                MediaIconButton(
                    if (player.playing) Glyph.Pause else Glyph.Play,
                    if (player.playing) "Pause" else "Play",
                    "dock-pause",
                ) {
                    vm.command("pause")
                }
                MediaIconButton(Glyph.Next, "Next track", "dock-next") { vm.command("next") }
            }
            if (player.duration > 0)
                LinearProgressIndicator(
                    progress = { (player.position / player.duration).coerceIn(0f, 1f) },
                    modifier = Modifier.fillMaxWidth().height(2.dp),
                    gapSize = 0.dp,
                    drawStopIndicator = {},
                )
        }
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
    Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.TopCenter) {
        Column(
            Modifier.widthIn(max = if (player.video) 840.dp else 480.dp).fillMaxWidth(),
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            if (player.media == null) {
                EmptyMediaState(
                    Glyph.Music,
                    "Nothing playing yet",
                    "Choose a video, song or station to get started.",
                )
                return@Column
            }
            if (player.video && showSurface && player.media != null) {
                AndroidView(
                    factory = { context ->
                        // TextureView participates in Compose dialog clipping and z-order;
                        // media decoding remains in Media3's hardware decoder.
                        android.view.LayoutInflater.from(context)
                            .inflate(
                                R.layout.ytui_player,
                                android.widget.FrameLayout(context),
                                false,
                            ) as PlayerView
                    },
                    modifier =
                        Modifier.fillMaxWidth()
                            .aspectRatio(16f / 9f)
                            .clip(RoundedCornerShape(20.dp))
                            .testTag("native-video-surface"),
                    update = { view ->
                        val actual = PlaybackService.current?.player
                        if (view.player !== actual) view.player = actual
                        view.keepScreenOn = player.playing
                    },
                    onRelease = {
                        it.keepScreenOn = false
                        it.player = null
                    },
                )
            } else if (showSurface && tab in 2..3 && player.media != null)
                Visualizer(player, config)
            else if (showSurface && player.media != null)
                Artwork(
                    MediaCard(player.media!!),
                    Modifier.fillMaxWidth().aspectRatio(16f / 9f),
                    config.optBoolean("thumbnails", true),
                )
            Text(
                player.title,
                style =
                    if (expanded) MaterialTheme.typography.headlineSmall
                    else MaterialTheme.typography.titleLarge,
                fontWeight = FontWeight.Bold,
            )
            player.media
                ?.optString("creator")
                ?.takeIf { it.isNotBlank() }
                ?.let {
                    Text(
                        it,
                        style = MaterialTheme.typography.bodyLarge,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            player.value
                .optString("station")
                .takeIf { it.isNotBlank() }
                ?.let {
                    Text(
                        "Live · $it",
                        color = MaterialTheme.colorScheme.primary,
                        style = MaterialTheme.typography.labelLarge,
                    )
                }
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
                return@Column
            }
            if (player.duration > 0 && player.media?.optBoolean("live") != true) {
                var seek by
                    remember(player.media?.optString("url")) { mutableStateOf<Float?>(null) }
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
                Row(Modifier.fillMaxWidth()) {
                    Text(
                        time(player.position),
                        style = MaterialTheme.typography.labelMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    Spacer(Modifier.weight(1f))
                    Text(
                        time(player.duration),
                        style = MaterialTheme.typography.labelMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                val segments = player.value.optJSONArray("segments")?.objects() ?: emptyList()
                if (segments.isNotEmpty()) {
                    val accent = MaterialTheme.colorScheme.primary
                    val muted = MaterialTheme.colorScheme.outline
                    Canvas(Modifier.fillMaxWidth().height(8.dp).testTag("sponsor-marks")) {
                        drawRect(muted.copy(alpha = 0.25f))
                        segments.forEach { mark ->
                            val start =
                                mark.optDouble("start").toFloat().coerceIn(0f, player.duration)
                            val end =
                                mark.optDouble("end").toFloat().coerceIn(start, player.duration)
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
            Row(
                Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.SpaceEvenly,
            ) {
                MediaIconButton(Glyph.Previous, "Previous track") { vm.command("previous") }
                TextButton(
                    { vm.command("back") },
                    Modifier.sizeIn(minWidth = 48.dp, minHeight = 48.dp),
                ) {
                    Text("−10", fontWeight = FontWeight.SemiBold)
                }
                FilledIconButton(
                    { vm.command("pause") },
                    Modifier.size(72.dp).testTag("player-pause").semantics {
                        contentDescription = if (player.playing) "Pause" else "Play"
                    },
                ) {
                    MediaIcon(if (player.playing) Glyph.Pause else Glyph.Play, Modifier.size(32.dp))
                }
                TextButton(
                    { vm.command("forward") },
                    Modifier.sizeIn(minWidth = 48.dp, minHeight = 48.dp),
                ) {
                    Text("+10", fontWeight = FontWeight.SemiBold)
                }
                MediaIconButton(Glyph.Next, "Next track") { vm.command("next") }
            }
            Row(
                Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.SpaceEvenly,
            ) {
                MediaIconButton(Glyph.Shuffle, "Shuffle queue") { vm.command("shuffle") }
                MediaIconButton(Glyph.Repeat, "Repeat mode") { vm.command("repeat") }
                MediaIconButton(Glyph.Queue, "Playback queue") { onQueue() }
                MediaIconButton(Glyph.Stop, "Stop playback") { vm.command("stop") }
            }
            Surface(
                shape = RoundedCornerShape(20.dp),
                color = MaterialTheme.colorScheme.surfaceContainer,
            ) {
                Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        MediaIcon(Glyph.Volume, Modifier.size(20.dp))
                        Text(
                            "Volume",
                            Modifier.padding(start = 10.dp).weight(1f),
                            style = MaterialTheme.typography.labelLarge,
                        )
                        Text(
                            "${(player.value.optDouble("volume", 0.7) * 100).roundToInt()}%",
                            style = MaterialTheme.typography.labelMedium,
                        )
                    }
                    Slider(
                        player.value.optDouble("volume", 0.7).toFloat().coerceIn(0f, 1f),
                        { vm.command("volume", "value" to it) },
                        modifier = Modifier.fillMaxWidth().testTag("volume-slider"),
                    )
                    FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        Box {
                            Action("${player.value.optDouble("speed", 1.0)}× speed") {
                                speedMenu = true
                            }
                            DropdownMenu(speedMenu, { speedMenu = false }) {
                                for (speed in
                                    listOf(0.5, 0.75, 1.0, 1.25, 1.5, 2.0)) DropdownMenuItem(
                                    text = { Text("$speed×") },
                                    onClick = {
                                        speedMenu = false
                                        vm.command("speed", "value" to speed)
                                    },
                                )
                            }
                        }
                        Action("Audio & subtitles") { tracks = !tracks }
                    }
                }
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
            if (player.value.optJSONArray("segments")?.length()?.let { it > 0 } == true)
                TextButton({ vm.command("sponsor_replay") }, Modifier.heightIn(min = 48.dp)) {
                    Text("Replay skipped segment")
                }
            if (!expanded) Action("Open full player") { vm.playerOpen.value = true }
        }
    }
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
        Modifier.fillMaxWidth()
            .aspectRatio(if (style == "classic") 1f else 1.35f)
            .clip(RoundedCornerShape(24.dp))
            .testTag("music-visualizer"),
        contentAlignment = Alignment.CenterStart,
    ) {
        if (style == "classic")
            Artwork(
                media,
                Modifier.fillMaxSize().align(Alignment.Center),
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
    var proxy by
        remember(config.optString("proxy")) {
            mutableStateOf(config.optString("proxy").takeUnless { it == "null" } ?: "")
        }
    Panel("Settings", close) {
        Text(
            "Make it yours.",
            style = MaterialTheme.typography.bodyLarge,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        SettingGroup("Appearance", "Colour, artwork and motion") {
            Box {
                Surface(
                    onClick = { themeMenu = true },
                    shape = RoundedCornerShape(14.dp),
                    color = MaterialTheme.colorScheme.surfaceContainerHigh,
                    modifier = Modifier.fillMaxWidth().testTag("theme-selector"),
                ) {
                    Row(
                        Modifier.padding(16.dp),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(12.dp),
                    ) {
                        MediaIcon(Glyph.Palette, tint = MaterialTheme.colorScheme.primary)
                        Column(Modifier.weight(1f)) {
                            Text(
                                "Theme: ${config.optString("theme", "system")}",
                                fontWeight = FontWeight.SemiBold,
                            )
                            Text(
                                "Applies immediately · saved automatically",
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                        MediaIcon(Glyph.Down)
                    }
                }
                DropdownMenu(themeMenu, { themeMenu = false }) {
                    themes.forEach { name ->
                        DropdownMenuItem(
                            text = { Text(name.replaceFirstChar(Char::titlecase)) },
                            modifier = Modifier.testTag("theme-$name"),
                            onClick = {
                                themeMenu = false
                                vm.updateConfig(dark) { it.put("theme", name) }
                            },
                        )
                    }
                }
            }
            TextButton({ vm.effect("pick", "kind" to "theme") }, Modifier.heightIn(min = 48.dp)) {
                MediaIcon(Glyph.Palette)
                Spacer(Modifier.width(8.dp))
                Text("Import a palette")
            }
            SettingToggle(
                "Thumbnails",
                config.optBoolean("thumbnails", true),
                "Cache artwork at the size you need",
            ) { value ->
                vm.updateConfig(dark) { it.put("thumbnails", value) }
            }
            SettingToggle(
                "Reduced motion",
                config.optBoolean("reduced_motion"),
                "Keep visualizers still",
            ) { value ->
                vm.updateConfig(dark) { it.put("reduced_motion", value) }
            }
            Choice(
                "Music player",
                config.optString("music_player_style", "classic"),
                listOf(
                    "classic",
                    "vinyl",
                    "vinyl-fullscreen",
                    "vu",
                    "vu-fullscreen",
                    "orbit",
                    "orbit-fullscreen",
                ),
            ) { value ->
                vm.updateConfig(dark) { it.put("music_player_style", value) }
            }
        }
        SettingGroup("Playback & downloads") {
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
                "Save music without the video",
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
        }
        SettingGroup("Your library") {
            SettingToggle(
                "History & resume",
                config.optBoolean("playback_history", true),
                "Remember playback locally on this device",
            ) { value ->
                vm.updateConfig(dark) { it.put("playback_history", value) }
            }
        }
        SettingGroup(
            "Network",
            "Your Android VPN works normally. Changes apply to new connections.",
        ) {
            OutlinedTextField(
                proxy,
                { proxy = it },
                Modifier.fillMaxWidth().testTag("proxy-field"),
                label = { Text("HTTP proxy (optional)") },
                singleLine = true,
                shape = RoundedCornerShape(14.dp),
            )
            Action("Apply network settings") {
                vm.updateConfig(dark) { it.put("proxy", proxy.trim().ifBlank { null }) }
            }
        }
        if (status != "Ready")
            Text(
                status,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
    }
}

@Composable
private fun SettingToggle(
    title: String,
    value: Boolean,
    description: String = "",
    changed: (Boolean) -> Unit,
) {
    Row(
        Modifier.fillMaxWidth()
            .heightIn(min = 64.dp)
            .toggleable(value, role = Role.Switch, onValueChange = changed),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(3.dp)) {
            Text(title, style = MaterialTheme.typography.bodyLarge)
            if (description.isNotBlank())
                Text(
                    description,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
        }
        Switch(value, null)
    }
}

@Composable
private fun Choice(title: String, value: String, values: List<String>, changed: (String) -> Unit) {
    var open by remember { mutableStateOf(false) }
    Box {
        Surface(
            onClick = { open = true },
            color = Color.Transparent,
            shape = RoundedCornerShape(12.dp),
        ) {
            Row(
                Modifier.fillMaxWidth().heightIn(min = 64.dp).padding(vertical = 8.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    Text(title, style = MaterialTheme.typography.bodyLarge)
                    Text(
                        value
                            .replace("-fullscreen", " · Full screen")
                            .replaceFirstChar(Char::titlecase),
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.primary,
                    )
                }
                MediaIcon(Glyph.Down)
            }
        }
        DropdownMenu(open, { open = false }) {
            values.forEach { item ->
                DropdownMenuItem(
                    text = {
                        Text(
                            item
                                .replace("-fullscreen", " · Full screen")
                                .replaceFirstChar(Char::titlecase)
                        )
                    },
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
    Panel("Your account", close) {
        Surface(
            shape = RoundedCornerShape(24.dp),
            color = MaterialTheme.colorScheme.primaryContainer,
        ) {
            Column(
                Modifier.fillMaxWidth().padding(24.dp),
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                MediaIcon(Glyph.Account, Modifier.size(40.dp), MaterialTheme.colorScheme.primary)
                Text(
                    if (account.optBoolean("connected")) "You're connected"
                    else "A feed that feels like you",
                    style = MaterialTheme.typography.headlineSmall,
                    fontWeight = FontWeight.Bold,
                )
                Text(
                    if (account.optBoolean("connected"))
                        "Your session is stored privately on this device."
                    else "Bring your subscriptions, favourites and music into one place."
                )
                Button(
                    { vm.effect("login") },
                    Modifier.heightIn(min = 48.dp).testTag("account-sign-in"),
                ) {
                    Text("Sign in to YouTube")
                }
                Text(
                    "Google may reject embedded sign-in. You can also import your session below.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
        SettingGroup(
            "Import a session",
            "Android cannot read another browser's private data. Import only a session you own.",
        ) {
            Action("Choose cookie file", "account-import") {
                vm.effect("pick", "kind" to "cookies")
            }
            Text(
                "Netscape cookie format",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            OutlinedTextField(
                header,
                { header = it },
                Modifier.fillMaxWidth().testTag("cookie-header-field"),
                label = { Text("Cookie header (optional)") },
                visualTransformation = PasswordVisualTransformation(),
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password),
                minLines = 2,
                shape = RoundedCornerShape(14.dp),
            )
            Action("Import header", enabled = header.isNotBlank()) {
                vm.changeAccount("import_cookies", "header" to header)
                header = ""
            }
        }
        SettingGroup("Session & privacy") {
            Text(
                "Encrypted with Android Keystore. Keep session files and headers private.",
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Action("Reload session") { vm.changeAccount("reload_account") }
                TextButton({ vm.changeAccount("disconnect") }, Modifier.heightIn(min = 48.dp)) {
                    Text("Disconnect", color = MaterialTheme.colorScheme.error)
                }
            }
        }
        if (status != "Ready")
            Text(
                status,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
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
        Modifier.fillMaxSize().padding(horizontal = 20.dp),
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Text(
            "Downloads",
            style = MaterialTheme.typography.headlineMedium,
            fontWeight = FontWeight.Bold,
        )
        Text(
            "Your library, wherever you go.",
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Button({ add = true }, Modifier.heightIn(min = 48.dp).testTag("add-download")) {
            MediaIcon(Glyph.Link, Modifier.size(20.dp))
            Spacer(Modifier.width(8.dp))
            Text("Add a video or playlist")
        }
        LazyColumn(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(10.dp)) {
            if (downloads.isEmpty())
                item {
                    EmptyMediaState(
                        Glyph.Download,
                        "Take it offline",
                        "Save videos, albums or whole playlists and enjoy them without a connection.",
                    )
                }
            items(downloads, key = { it.optString("id") }) { item ->
                Surface(
                    color = MaterialTheme.colorScheme.surfaceContainer,
                    shape = RoundedCornerShape(20.dp),
                ) {
                    Column(
                        Modifier.fillMaxWidth().padding(20.dp),
                        verticalArrangement = Arrangement.spacedBy(12.dp),
                    ) {
                        Text(
                            item.optString("title"),
                            style = MaterialTheme.typography.titleMedium,
                            fontWeight = FontWeight.SemiBold,
                        )
                        Text(
                            item.optString("state"),
                            style = MaterialTheme.typography.labelMedium,
                            color = MaterialTheme.colorScheme.primary,
                        )
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
