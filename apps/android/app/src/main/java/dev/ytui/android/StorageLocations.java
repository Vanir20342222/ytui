package dev.ytui.android;

import android.content.*;
import android.net.Uri;
import android.provider.DocumentsContract;
import java.io.*;
import java.util.*;
import org.json.*;

/** User-granted document trees and app-produced files only. Never stores stream leases here. */
final class StorageLocations {
  static SharedPreferences prefs() {
    return MediaBridge.context.getSharedPreferences("offline-folders", Context.MODE_PRIVATE);
  }

  static Uri folder(boolean audio) {
    String raw = prefs().getString(audio ? "music" : "video", "");
    return raw.isEmpty() ? null : Uri.parse(raw);
  }

  static String label(boolean audio) {
    Uri folder = folder(audio);
    if (folder == null) return "App storage";
    String name = DocumentsContract.getTreeDocumentId(folder);
    return name.substring(name.lastIndexOf(':') + 1).replace("/", " › ");
  }

  static boolean granted(Uri tree) {
    for (UriPermission permission :
        MediaBridge.context.getContentResolver().getPersistedUriPermissions())
      if (permission.getUri().equals(tree)
          && permission.isReadPermission()
          && permission.isWritePermission()) return true;
    return false;
  }

  static void select(boolean audio, Uri tree, int flags) throws Exception {
    if (!DocumentsContract.isTreeUri(tree)) throw new IOException("Choose a folder");
    int grant =
        flags & (Intent.FLAG_GRANT_READ_URI_PERMISSION | Intent.FLAG_GRANT_WRITE_URI_PERMISSION);
    MediaBridge.context.getContentResolver().takePersistableUriPermission(tree, grant);
    if (!granted(tree)) throw new IOException("The folder must allow reading and writing");
    if (!prefs().edit().putString(audio ? "music" : "video", tree.toString()).commit())
      throw new IOException("Could not save folder");
    OfflineStore.exportPending();
    MediaBridge.event("storage_updated", "");
  }

  static synchronized JSONObject records() throws JSONException {
    return new JSONObject(prefs().getString("files", "{}"));
  }

  static synchronized JSONObject record(String id) throws JSONException {
    return records().optJSONObject(id);
  }

  static synchronized void remember(String id, Uri document, JSONObject request) throws Exception {
    if (prefs().getBoolean("removed:" + id, false))
      throw new InterruptedIOException("Offline copy removed");
    JSONObject files = records();
    JSONObject media = new JSONObject(request.getJSONObject("media").toString());
    media.put("offline_id", id);
    files.put(
        id,
        new JSONObject()
            .put("id", id)
            .put("uri", document.toString())
            .put("media", media)
            .put("audio", request.optBoolean("audio"))
            .put("folder", label(request.optBoolean("audio"))));
    String encoded = files.toString();
    if (files.length() > 2000 || encoded.length() > 4 * 1024 * 1024)
      throw new IOException("Offline folder index is full");
    if (!prefs().edit().putString("files", encoded).remove("error:" + id).commit())
      throw new IOException("Could not save offline file index");
    OfflineStore.dirty = true;
  }

  static JSONObject request(String id) throws Exception {
    JSONObject file = record(id);
    if (file == null) throw new IOException("Offline file is unavailable");
    Uri uri = Uri.parse(file.getString("uri"));
    try (android.content.res.AssetFileDescriptor descriptor =
        MediaBridge.context.getContentResolver().openAssetFileDescriptor(uri, "r")) {
      if (descriptor == null) throw new IOException("Offline folder cannot be read");
    }
    return new JSONObject()
        .put("media", file.getJSONObject("media"))
        .put("audio", file.optBoolean("audio"))
        .put("stream", new JSONObject().put("url", uri.toString()))
        .put("local", true);
  }

  static synchronized void remove(String id) throws Exception {
    JSONObject files = records(), file = files.optJSONObject(id);
    if (file != null) {
      try {
        if (!DocumentsContract.deleteDocument(
            MediaBridge.context.getContentResolver(), Uri.parse(file.getString("uri"))))
          throw new IOException("Cannot remove the offline file; reconnect its folder");
      } catch (FileNotFoundException alreadyRemoved) {
        /* remove its stale index only */
      }
      files.remove(id);
    }
    if (!prefs().edit().putString("files", files.toString()).remove("error:" + id).commit())
      throw new IOException("Could not update offline library");
    OfflineStore.dirty = true;
  }

  static JSONArray rows() throws Exception {
    JSONObject files = records();
    JSONArray rows = new JSONArray();
    for (Iterator<String> ids = files.keys(); ids.hasNext(); ) {
      String id = ids.next();
      JSONObject item = files.getJSONObject(id);
      rows.put(
          new JSONObject()
              .put("id", id)
              .put("title", item.getJSONObject("media").optString("title"))
              .put("state", "Complete")
              .put("media", item.getJSONObject("media"))
              .put("audio", item.optBoolean("audio"))
              .put("folder", item.optString("folder"))
              .put("progress", 1));
    }
    return rows;
  }

  static void failed(String id) {
    prefs()
        .edit()
        .putString(
            "error:" + id,
            "Folder save failed; cached media is intact. Reconnect its folder and retry.")
        .apply();
    OfflineStore.dirty = true;
  }

  private StorageLocations() {}
}
