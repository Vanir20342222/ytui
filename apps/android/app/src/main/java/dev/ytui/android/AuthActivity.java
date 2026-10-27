package dev.ytui.android;

import android.app.Activity;
import android.os.Bundle;
import android.webkit.*;

/** The only web content is the explicit account sign-in page, never the app UI. */
public final class AuthActivity extends Activity {
  WebView web;

  // Explicit account web flow only; no JavaScript host interface is exposed.
  @android.annotation.SuppressLint("SetJavaScriptEnabled")
  @Override
  public void onCreate(Bundle state) {
    super.onCreate(state);
    web = new WebView(this);
    web.getSettings().setJavaScriptEnabled(true);
    web.getSettings().setDomStorageEnabled(true);
    web.setWebViewClient(
        new WebViewClient() {
          @Override
          public boolean shouldOverrideUrlLoading(WebView view, WebResourceRequest request) {
            String host = request.getUrl().getHost();
            return host == null
                || !(host.equals("youtube.com")
                    || host.endsWith(".youtube.com")
                    || host.equals("google.com")
                    || host.endsWith(".google.com"));
          }
        });
    setContentView(web);
    web.loadUrl("https://www.youtube.com/feed/you");
  }

  @Override
  protected void onDestroy() {
    String cookies = CookieManager.getInstance().getCookie("https://www.youtube.com");
    if (cookies != null && cookies.contains("SAPISID=")) {
      MediaBridge.event("cookies", cookies);
      CookieManager.getInstance().flush();
    }
    if (web != null) web.destroy();
    super.onDestroy();
  }
}
