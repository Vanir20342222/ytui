package dev.ytui.android;

import android.app.Application;

public final class YtuiApplication extends Application {
  @Override
  public void onCreate() {
    super.onCreate();
    NativeBridge.init(getFilesDir().getAbsolutePath(), getCacheDir().getAbsolutePath());
    MediaBridge.attach(this);
  }
}
