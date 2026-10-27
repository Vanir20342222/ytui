package dev.ytui.android;

import android.content.Intent;
import android.content.pm.PackageManager;
import android.os.Build;
import android.os.Bundle;
import com.google.androidgamesdk.GameActivity;
import java.io.InputStream;

public final class RustActivity extends GameActivity {
  static volatile java.lang.ref.WeakReference<RustActivity> reference =
      new java.lang.ref.WeakReference<>(null);

  static RustActivity current() {
    return reference.get();
  }

  @Override
  protected void onCreate(Bundle state) {
    reference = new java.lang.ref.WeakReference<>(this);
    super.onCreate(state);
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
              } else {
                MediaBridge.topInset = insets.getSystemWindowInsetTop();
                MediaBridge.bottomInset = insets.getSystemWindowInsetBottom();
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
  protected void onActivityResult(int request, int result, Intent data) {
    super.onActivityResult(request, result, data);
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
