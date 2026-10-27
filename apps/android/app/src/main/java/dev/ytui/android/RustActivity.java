package dev.ytui.android;

import android.content.Intent;
import android.content.pm.PackageManager;
import android.os.Build;
import android.os.Bundle;
import com.google.androidgamesdk.GameActivity;
import java.io.InputStream;

public final class RustActivity extends GameActivity {
  private String exportDownload;
  static volatile java.lang.ref.WeakReference<RustActivity> reference =
      new java.lang.ref.WeakReference<>(null);

  static RustActivity current() {
    return reference.get();
  }

  @Override
  protected void onCreate(Bundle state) {
    reference = new java.lang.ref.WeakReference<>(this);
    super.onCreate(state);
    if (state != null) exportDownload = state.getString("ytui-export-download");
    OfflineStore.resetSnapshotClient();
    getWindow()
        .getDecorView()
        .setOnApplyWindowInsetsListener(
            (view, insets) -> {
              if (Build.VERSION.SDK_INT >= 30) {
                android.graphics.Insets bars =
                    insets.getInsets(
                        android.view.WindowInsets.Type.systemBars()
                            | android.view.WindowInsets.Type.displayCutout());
                MediaBridge.topInset = bars.top;
                MediaBridge.bottomInset = bars.bottom;
                MediaBridge.leftInset = bars.left;
                MediaBridge.rightInset = bars.right;
              } else {
                MediaBridge.topInset = insets.getSystemWindowInsetTop();
                MediaBridge.bottomInset = insets.getSystemWindowInsetBottom();
                MediaBridge.leftInset = insets.getSystemWindowInsetLeft();
                MediaBridge.rightInset = insets.getSystemWindowInsetRight();
              }
              return insets;
            });
    getWindow().getDecorView().requestApplyInsets();
    if (Build.VERSION.SDK_INT >= 33
        && checkSelfPermission("android.permission.POST_NOTIFICATIONS")
            != PackageManager.PERMISSION_GRANTED)
      requestPermissions(new String[] {"android.permission.POST_NOTIFICATIONS"}, 21);
    onNewIntent(getIntent());
    if (Build.VERSION.SDK_INT >= 33)
      getOnBackInvokedDispatcher()
          .registerOnBackInvokedCallback(
              android.window.OnBackInvokedDispatcher.PRIORITY_DEFAULT,
              () -> {
                MediaBridge.event("back", "");
                NativeBridge.frameAvailable();
              });
  }

  // Rust handles overlays/navigation; calling the base would finish the activity.
  @android.annotation.SuppressLint("MissingSuperCall")
  @Override
  public void onBackPressed() {
    MediaBridge.event("back", "");
    NativeBridge.frameAvailable();
  }

  @Override
  protected void onResume() {
    super.onResume();
    MediaBridge.uiVisible = true;
    MediaBridge.notifyUi();
  }

  @Override
  protected void onStop() {
    MediaBridge.uiVisible = false;
    super.onStop();
  }

  @Override
  protected void onDestroy() {
    if (reference.get() == this) reference.clear();
    super.onDestroy();
  }

  @Override
  protected void onNewIntent(Intent intent) {
    super.onNewIntent(intent);
    if ("dev.ytui.OPEN_PLAYER".equals(intent.getAction())) MediaBridge.event("show_player", "");
    if (Intent.ACTION_SEND.equals(intent.getAction()))
      MediaBridge.event("shared", intent.getStringExtra(Intent.EXTRA_TEXT));
  }

  @Override
  protected void onSaveInstanceState(Bundle state) {
    state.putString("ytui-export-download", exportDownload);
    super.onSaveInstanceState(state);
  }

  void exportDownload(String id) {
    if (exportDownload != null) {
      MediaBridge.event("error", "Finish or cancel the open export picker first");
      return;
    }
    exportDownload = id;
    OfflineStore.IO.execute(
        () -> {
          try {
            OfflineExport.Plan plan = OfflineExport.inspect(id);
            MediaBridge.main.post(
                () -> {
                  if (isFinishing() || isDestroyed()) {
                    exportDownload = null;
                    return;
                  }
                  try {
                    startActivityForResult(
                        new Intent(Intent.ACTION_CREATE_DOCUMENT)
                            .setType(plan.mime)
                            .addCategory(Intent.CATEGORY_OPENABLE)
                            .addFlags(
                                Intent.FLAG_GRANT_WRITE_URI_PERMISSION
                                    | Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION)
                            .putExtra(
                                Intent.EXTRA_TITLE,
                                PlaybackPolicy.exportName(plan.title, plan.extension)),
                        43);
                  } catch (RuntimeException error) {
                    exportDownload = null;
                    MediaBridge.event("error", "No document provider is available for export");
                  }
                });
          } catch (Exception error) {
            MediaBridge.main.post(() -> exportDownload = null);
            MediaBridge.event("error", OfflineExport.safeReason(error));
          }
        });
  }

  @Override
  protected void onActivityResult(int request, int result, Intent data) {
    super.onActivityResult(request, result, data);
    if (request == 43) {
      String id = exportDownload;
      exportDownload = null;
      if (id == null || result != RESULT_OK || data == null || data.getData() == null) return;
      android.net.Uri destination = data.getData();
      if ((data.getFlags() & Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION) != 0) {
        try {
          getContentResolver()
              .takePersistableUriPermission(destination, Intent.FLAG_GRANT_WRITE_URI_PERMISSION);
        } catch (SecurityException ignored) {
        }
      }
      OfflineExport.enqueue(id, destination);
      return;
    }
    if (result != RESULT_OK || data == null || data.getData() == null) return;
    final android.net.Uri uri = data.getData();
    final android.content.ContentResolver resolver = getApplicationContext().getContentResolver();
    OfflineStore.IO.execute(
        () -> {
          try (InputStream input = resolver.openInputStream(uri)) {
            if (input == null) throw new java.io.IOException("No document data");
            java.io.ByteArrayOutputStream bytes = new java.io.ByteArrayOutputStream();
            byte[] buffer = new byte[4096];
            int count;
            while ((count = input.read(buffer)) != -1) {
              if (bytes.size() + count > 2 * 1024 * 1024)
                throw new java.io.IOException("Document exceeds 2 MiB");
              bytes.write(buffer, 0, count);
            }
            MediaBridge.event(
                request == 41 ? "cookie_file" : request == 42 ? "theme_file" : "document",
                new String(bytes.toByteArray(), java.nio.charset.StandardCharsets.UTF_8));
            NativeBridge.frameAvailable();
          } catch (Exception error) {
            MediaBridge.event("error", "Could not read selected document");
          }
        });
  }
}
