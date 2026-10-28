package dev.ytui.android;

import android.os.Bundle;
import android.webkit.*;
import androidx.activity.ComponentActivity;

/** The only web content is the explicit account sign-in page, never the app UI. */
public final class AuthActivity extends ComponentActivity {
  WebView web;
  private String flow;
  private static String activeFlow = "";
  private static String completedFlow = "";
  private static java.lang.ref.WeakReference<AuthActivity> active =
      new java.lang.ref.WeakReference<>(null);

  // Explicit account web flow only; no JavaScript host interface is exposed.
  @android.annotation.SuppressLint("SetJavaScriptEnabled")
  @Override
  public void onCreate(Bundle state) {
    super.onCreate(state);
    flow = state == null ? java.util.UUID.randomUUID().toString() : state.getString("auth-flow");
    active = new java.lang.ref.WeakReference<>(this);
    activeFlow = flow;
    if (state == null) {
      ComposeHost.resetAuth();
      NativeBridge.beginAccountChange();
    }
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
    ComposeHost.attachAuth(this, web);
    if (state != null) web.restoreState(state);
    else if (BuildConfig.DEBUG && getIntent().getBooleanExtra("ytui.offline-auth", false))
      web.loadData(
          "<p>Offline sign-in UI fixture. No account is contacted.</p>", "text/html", "UTF-8");
    else web.loadUrl("https://www.youtube.com/feed/you");
  }

  void useSession() {
    if (ComposeHost.authBusy()) return;
    String header = CookieManager.getInstance().getCookie("https://www.youtube.com");
    if (header == null || header.isEmpty()) {
      ComposeHost.authMessage(
          "No signed-in YouTube session found. Complete sign-in or use cookie-file import.");
      return;
    }
    ComposeHost.setAuthBusy(true);
    long intent = NativeBridge.beginAccountChange();
    OfflineStore.IO.execute(
        () -> {
          try {
            org.json.JSONObject request =
                new org.json.JSONObject()
                    .put("op", "import_cookies")
                    .put("account_intent", intent)
                    .put("header", header);
            org.json.JSONObject response =
                new org.json.JSONObject(NativeBridge.request(request.toString()));
            MediaBridge.main.post(
                () -> {
                  if (!flow.equals(activeFlow)) return;
                  ComposeHost.setAuthBusy(false);
                  if (response.optBoolean("ok")) {
                    MediaBridge.event("account_updated", "");
                    clearAndFinish();
                  } else
                    ComposeHost.authMessage(
                        response.optString("error", "Could not import this session"));
                });
          } catch (Exception error) {
            MediaBridge.main.post(
                () -> {
                  if (!flow.equals(activeFlow)) return;
                  ComposeHost.setAuthBusy(false);
                  ComposeHost.authMessage(
                      "Could not import this session; use a cookie document instead");
                });
          }
        });
  }

  void cancelSession() {
    if (!ComposeHost.authBusy()) clearAndFinish();
  }

  private void clearAndFinish() {
    completedFlow = flow;
    CookieManager.getInstance()
        .removeAllCookies(
            ignored -> {
              CookieManager.getInstance().flush();
              AuthActivity target = active.get();
              if (target != null && flow.equals(target.flow)) target.finish();
            });
  }

  @Override
  protected void onResume() {
    super.onResume();
    if (flow.equals(completedFlow)) finish();
  }

  @Override
  protected void onSaveInstanceState(Bundle state) {
    state.putString("auth-flow", flow);
    web.saveState(state);
    super.onSaveInstanceState(state);
  }

  @Override
  protected void onDestroy() {
    if (active.get() == this) active.clear();
    if (web != null) web.destroy();
    super.onDestroy();
  }
}
