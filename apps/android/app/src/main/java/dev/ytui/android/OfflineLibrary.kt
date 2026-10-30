package dev.ytui.android

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import org.json.JSONObject

@Composable
internal fun OfflineLibrary(vm: YtuiViewModel) {
    val downloads by vm.downloads.collectAsState()
    val playlists by vm.offlinePlaylists.collectAsState()
    val config by vm.config.collectAsState()
    val playerOpen by vm.playerOpen.collectAsState()
    val error by vm.actionError.collectAsState()
    var section by rememberSaveable { mutableStateOf("Offline") }
    var add by remember { mutableStateOf(false) }
    var newPlaylist by remember { mutableStateOf(false) }
    var addTo by remember { mutableStateOf<JSONObject?>(null) }
    var opened by rememberSaveable { mutableStateOf("") }
    var removing by remember { mutableStateOf<JSONObject?>(null) }
    var deleting by remember { mutableStateOf<JSONObject?>(null) }
    val available =
        downloads.filter {
            it.optString("state") == "Complete" && it.optJSONObject("media") != null
        }
    val transfers =
        downloads.filter {
            it.optString("state") != "Complete" || it.optJSONObject("media") == null
        }
    LaunchedEffect(Unit) { vm.loadOfflinePlaylists() }
    DisposableEffect(playerOpen) {
        vm.effect("library_visible", "value" to !playerOpen)
        onDispose { vm.effect("library_visible", "value" to false) }
    }
    Column(
        Modifier.fillMaxSize().padding(horizontal = 20.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
            Text(
                "Library",
                Modifier.weight(1f),
                style = MaterialTheme.typography.headlineMedium,
                fontWeight = FontWeight.Bold,
            )
            MediaIconButton(Glyph.Link, "Save a video or playlist offline", "add-download") {
                add = true
            }
            MediaIconButton(Glyph.Queue, "Create offline playlist", "create-offline-playlist") {
                newPlaylist = true
            }
        }
        LazyRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            items(listOf("Offline", "Playlists", "Transfers")) { title ->
                MediaChip(
                    title,
                    selected = section == title,
                    modifier = Modifier.testTag("library-$title"),
                ) { section = title }
            }
        }
        if (error != null) Text(error!!, color = MaterialTheme.colorScheme.error)
        LazyColumn(
            Modifier.weight(1f).fillMaxWidth(),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            if (section == "Offline") {
                if (available.isEmpty())
                    item {
                        EmptyMediaState(
                            Glyph.Download,
                            "No offline media",
                            "Save songs or videos to play without a connection.",
                        )
                    }
                items(available, key = { it.optString("id") }) { item ->
                    var menu by remember { mutableStateOf(false) }
                    Surface(
                        shape = RoundedCornerShape(20.dp),
                        color = MaterialTheme.colorScheme.surfaceContainer,
                    ) {
                        Row(
                            Modifier.fillMaxWidth().padding(12.dp),
                            verticalAlignment = Alignment.CenterVertically,
                            horizontalArrangement = Arrangement.spacedBy(12.dp),
                        ) {
                            Artwork(
                                MediaCard(item.getJSONObject("media")),
                                Modifier.size(76.dp).clip(RoundedCornerShape(12.dp)),
                                config.optBoolean("thumbnails", true),
                            )
                            Column(
                                Modifier.weight(1f).clickable { vm.playOffline(item) },
                                verticalArrangement = Arrangement.spacedBy(4.dp),
                            ) {
                                Text(
                                    item.optString("title"),
                                    fontWeight = FontWeight.SemiBold,
                                    maxLines = 2,
                                    overflow = TextOverflow.Ellipsis,
                                )
                                Text(
                                    if (item.optBoolean("audio")) "Audio" else "Video",
                                    style = MaterialTheme.typography.labelMedium,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                                )
                                item
                                    .optString("storage_error")
                                    .takeIf { it.isNotBlank() }
                                    ?.let {
                                        Text(
                                            it,
                                            style = MaterialTheme.typography.bodySmall,
                                            color = MaterialTheme.colorScheme.error,
                                        )
                                    }
                            }
                            MediaIconButton(Glyph.Play, "Play offline ${item.optString("title")}") {
                                vm.playOffline(item)
                            }
                            Box {
                                MediaIconButton(Glyph.More, "Offline media options") { menu = true }
                                DropdownMenu(menu, { menu = false }) {
                                    DropdownMenuItem(
                                        text = { Text("Add to offline playlist") },
                                        onClick = {
                                            menu = false
                                            addTo = item
                                        },
                                    )
                                    DropdownMenuItem(
                                        text = { Text("Export file") },
                                        onClick = {
                                            menu = false
                                            vm.effect(
                                                "download_export",
                                                "id" to item.optString("id"),
                                            )
                                        },
                                        enabled = item.optBoolean("cached"),
                                    )
                                    DropdownMenuItem(
                                        text = { Text("Retry folder save") },
                                        onClick = {
                                            menu = false
                                            vm.effect("folder_retry")
                                        },
                                    )
                                    DropdownMenuItem(
                                        text = { Text("Remove offline copy") },
                                        onClick = {
                                            menu = false
                                            removing = item
                                        },
                                    )
                                }
                            }
                        }
                    }
                }
            } else if (section == "Playlists") {
                if (playlists.isEmpty())
                    item {
                        EmptyMediaState(
                            Glyph.Queue,
                            "No offline playlists",
                            "Create a playlist, then add media from the Offline tab.",
                        )
                    }
                items(playlists, key = { it.optString("id") }) { playlist ->
                    Surface(
                        onClick = { opened = playlist.optString("id") },
                        shape = RoundedCornerShape(20.dp),
                        color = MaterialTheme.colorScheme.surfaceContainer,
                    ) {
                        Row(
                            Modifier.fillMaxWidth().padding(16.dp),
                            verticalAlignment = Alignment.CenterVertically,
                            horizontalArrangement = Arrangement.spacedBy(12.dp),
                        ) {
                            val first = playlist.optJSONArray("items")?.optJSONObject(0)
                            if (first != null)
                                Artwork(
                                    MediaCard(first),
                                    Modifier.size(72.dp).clip(RoundedCornerShape(12.dp)),
                                )
                            else MediaIcon(Glyph.Queue, Modifier.size(36.dp))
                            Column(Modifier.weight(1f)) {
                                Text(playlist.optString("title"), fontWeight = FontWeight.SemiBold)
                                Text(
                                    "${playlist.optJSONArray("items")?.length() ?: 0} items",
                                    style = MaterialTheme.typography.bodySmall,
                                )
                            }
                            MediaIconButton(
                                Glyph.Play,
                                "Play offline playlist ${playlist.optString("title")}",
                            ) {
                                vm.playOfflinePlaylist(playlist)
                            }
                        }
                    }
                }
            } else {
                if (transfers.isEmpty())
                    item { EmptyMediaState(Glyph.Check, "No active transfers", "") }
                items(transfers, key = { it.optString("id") }) { item ->
                    Surface(
                        shape = RoundedCornerShape(16.dp),
                        color = MaterialTheme.colorScheme.surfaceContainer,
                    ) {
                        Column(
                            Modifier.padding(16.dp),
                            verticalArrangement = Arrangement.spacedBy(8.dp),
                        ) {
                            Text(item.optString("title"), fontWeight = FontWeight.SemiBold)
                            Text(
                                item.optString("state"),
                                style = MaterialTheme.typography.bodySmall,
                            )
                            val progress =
                                item
                                    .optDouble("progress")
                                    .toFloat()
                                    .takeIf { it.isFinite() }
                                    ?.coerceIn(0f, 1f) ?: 0f
                            LinearProgressIndicator({ progress }, Modifier.fillMaxWidth())
                            Row {
                                TextButton({
                                    vm.effect("download_toggle", "id" to item.optString("id"))
                                }) {
                                    Text("Pause / retry")
                                }
                                TextButton({ removing = item }) { Text("Remove") }
                            }
                        }
                    }
                }
            }
        }
    }
    if (add)
        AddOfflinePanel(vm) {
            add = false
            section = "Transfers"
        }
    if (addTo != null)
        Panel("Add to offline playlist", { addTo = null }) {
            Action("Create playlist") { newPlaylist = true }
            for (playlist in playlists) Action(playlist.optString("title")) {
                val item =
                    addTo!!
                        .getJSONObject("media")
                        .copyJson()
                        .put("offline_id", addTo!!.optString("id"))
                val members = playlist.optJSONArray("items")?.objects() ?: emptyList()
                if (members.none { it.optString("url") == item.optString("url") })
                    vm.updateOfflinePlaylist(playlist.optString("id"), items = members + item)
                addTo = null
            }
        }
    if (newPlaylist) {
        var name by remember { mutableStateOf("") }
        Panel("New offline playlist", { newPlaylist = false }) {
            OutlinedTextField(
                name,
                { if (it.length <= 120) name = it },
                Modifier.fillMaxWidth().testTag("offline-playlist-name"),
                label = { Text("Playlist name") },
                singleLine = true,
            )
            Action("Create", enabled = name.isNotBlank()) {
                vm.createOfflinePlaylist(name.trim())
                newPlaylist = false
                section = "Playlists"
            }
        }
    }
    playlists
        .firstOrNull { it.optString("id") == opened }
        ?.let { playlist ->
            var name by remember(opened) { mutableStateOf(playlist.optString("title")) }
            Panel("Offline playlist", { opened = "" }, scroll = false) {
                OutlinedTextField(
                    name,
                    { if (it.length <= 120) name = it },
                    Modifier.fillMaxWidth().testTag("offline-playlist-edit-name"),
                    label = { Text("Name") },
                    singleLine = true,
                )
                Row {
                    TextButton(
                        { vm.updateOfflinePlaylist(opened, title = name) },
                        enabled = name.isNotBlank(),
                    ) {
                        Text("Rename")
                    }
                    TextButton({ vm.playOfflinePlaylist(playlist) }) { Text("Play all") }
                    TextButton({ deleting = playlist }) { Text("Delete playlist") }
                }
                val items = playlist.optJSONArray("items")?.objects() ?: emptyList()
                LazyColumn(Modifier.weight(1f)) {
                    itemsIndexed(items, key = { _, it -> it.optString("offline_id") }) { index, item
                        ->
                        Column(Modifier.padding(vertical = 8.dp)) {
                            Text(
                                item.optString("title"),
                                maxLines = 2,
                                overflow = TextOverflow.Ellipsis,
                            )
                            val saved =
                                available.any { it.optString("id") == item.optString("offline_id") }
                            Row {
                                TextButton(
                                    { vm.playOfflinePlaylist(playlist, index) },
                                    enabled = saved,
                                ) {
                                    Text(if (saved) "Play" else "Copy unavailable")
                                }
                                MediaIconButton(Glyph.Up, "Move up") {
                                    if (index > 0)
                                        vm.updateOfflinePlaylist(
                                            opened,
                                            items =
                                                items.toMutableList().apply {
                                                    add(index - 1, removeAt(index))
                                                },
                                        )
                                }
                                MediaIconButton(Glyph.Down, "Move down") {
                                    if (index + 1 < items.size)
                                        vm.updateOfflinePlaylist(
                                            opened,
                                            items =
                                                items.toMutableList().apply {
                                                    add(index + 1, removeAt(index))
                                                },
                                        )
                                }
                                MediaIconButton(Glyph.Close, "Remove from playlist") {
                                    vm.updateOfflinePlaylist(
                                        opened,
                                        items = items.filterIndexed { at, _ -> at != index },
                                    )
                                }
                            }
                        }
                    }
                }
            }
        }
    if (removing != null)
        AlertDialog(
            onDismissRequest = { removing = null },
            title = { Text("Remove offline copy?") },
            text = {
                Text(
                    "${removing!!.optString("title")} will be removed from app storage and its saved folder copy. Your playlists keep their references."
                )
            },
            confirmButton = {
                TextButton({
                    vm.effect("download_remove", "id" to removing!!.optString("id"))
                    removing = null
                }) {
                    Text("Remove")
                }
            },
            dismissButton = { TextButton({ removing = null }) { Text("Cancel") } },
        )
    if (deleting != null)
        AlertDialog(
            onDismissRequest = { deleting = null },
            title = { Text("Delete playlist?") },
            text = { Text("Saved media files will stay in your library.") },
            confirmButton = {
                TextButton({
                    vm.deleteOfflinePlaylist(deleting!!.optString("id"))
                    deleting = null
                    opened = ""
                }) {
                    Text("Delete")
                }
            },
            dismissButton = { TextButton({ deleting = null }) { Text("Cancel") } },
        )
}

@Composable
private fun AddOfflinePanel(vm: YtuiViewModel, close: () -> Unit) {
    var url by rememberSaveable { mutableStateOf("") }
    Panel("Save offline", close) {
        OutlinedTextField(
            url,
            { url = it },
            Modifier.fillMaxWidth().testTag("download-url-field"),
            label = { Text("Video or playlist URL") },
            singleLine = true,
        )
        Text(
            "Choose quality, audio-only mode and Music/Video folders in Settings → Offline & storage.",
            style = MaterialTheme.typography.bodySmall,
        )
        Action("Save offline", enabled = url.isNotBlank()) {
            runCatching { vm.linked(url.trim()) }
                .onSuccess {
                    vm.play(it, download = true)
                    close()
                }
                .onFailure { vm.actionError.value = "Enter a valid HTTP(S) media URL" }
        }
    }
}
