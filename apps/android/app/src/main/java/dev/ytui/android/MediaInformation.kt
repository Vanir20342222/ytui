package dev.ytui.android

import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import org.json.JSONObject

internal fun mediaInformation(media: JSONObject, settings: JSONObject): String {
    val details = mutableListOf<String>()
    if (settings.optBoolean("show_views", true)) {
        val count = media.optLong("view_count", -1)
        val views =
            if (count >= 0) "${java.text.NumberFormat.getIntegerInstance().format(count)} views"
            else media.optString("views_text").takeUnless { it == "null" } ?: ""
        if (views.isNotBlank()) details += views
    }
    if (settings.optBoolean("show_upload_date", true)) {
        val date = media.optString("upload_date").takeIf { it.matches(Regex("[0-9]{8}")) }
        val text =
            date?.let { "${it.substring(0,4)}-${it.substring(4,6)}-${it.substring(6,8)}" }
                ?: media.optString("published_text").takeUnless { it == "null" }
                ?: ""
        if (text.isNotBlank()) details += text
    }
    if (settings.optBoolean("show_likes") && media.optLong("like_count", -1) >= 0)
        details +=
            "${java.text.NumberFormat.getIntegerInstance().format(media.optLong("like_count"))} likes"
    return details.joinToString(" · ")
}

@Composable
internal fun MediaInformation(media: JSONObject, settings: JSONObject) {
    val text = mediaInformation(media, settings)
    if (text.isNotBlank())
        Text(
            text,
            Modifier.testTag("video-information"),
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            maxLines = 2,
            overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis,
        )
}
