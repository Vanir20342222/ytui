pub mod auth;
pub mod browse;
pub mod desktop;
pub mod download;
pub mod images;
pub mod library;
pub mod migration;
pub mod native;
mod native_post;
pub mod paths;
pub mod player;
pub mod process;
pub mod profiles;
pub mod queue_import;
pub mod radio;
pub mod store;
#[cfg(test)]
mod test_http;
pub mod theme;
mod throttle;
pub mod vpn;
pub mod watch;
mod worker_process;

pub fn http_client(proxy: Option<&str>) -> anyhow::Result<reqwest::Client> {
    let mut builder = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/132.0.0.0 Safari/537.36")
        .timeout(std::time::Duration::from_secs(25))
        .connect_timeout(std::time::Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none());
    if let Some(proxy) = proxy.filter(|p| !p.is_empty()) {
        builder = builder.proxy(reqwest::Proxy::all(proxy)?);
    }
    Ok(builder.build()?)
}
