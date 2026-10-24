//! Integration review regressions through real loopback HTTP and the browser parser.
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use ytui_core::watch::CollectionWatch;

fn watch(music: bool) -> CollectionWatch {
    CollectionWatch {
        id: uuid::Uuid::new_v4(),
        title: "Fixture collection".into(),
        request: BrowseRequest::Collection("VLfixture".into()),
        music,
        options: ytui_core::DownloadOptions::default(),
        interval_seconds: 60,
        next_check: 0,
        enabled: true,
        generation: 0,
        owner: "anonymous".into(),
        seen: None,
        failures: 0,
        error: None,
    }
}
async fn provider(
    handler: impl Fn(usize) -> Value + Send + 'static,
) -> (Browser, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let (base, task) = crate::test_http::serve_request(move |request| {
        let body = if request.starts_with("GET") {
            r#"ytcfg.set({"INNERTUBE_CLIENT_VERSION":"fixture"});"#.to_owned()
        } else {
            handler(count.fetch_add(1, Ordering::SeqCst)).to_string()
        };
        (200, String::new(), body.into_bytes())
    })
    .await;
    let mut browser = Browser::new(
        reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(20))
            .build()
            .unwrap(),
        None,
        0,
    );
    browser.origins = [base.clone(), base];
    (browser, calls, task)
}
fn page(count: usize, token: Option<&str>) -> Value {
    let rows = (0..count)
        .map(|n| {
            json!({"videoRenderer":{
                "videoId":format!("fixture-{n}"),"title":{"simpleText":format!("Video {n}")}
            }})
        })
        .collect::<Vec<_>>();
    let mut data = json!({"contents":rows});
    if let Some(token) = token {
        data["continuationCommand"] = json!({"token":token});
    }
    data
}
async fn collect(browser: Browser, music: bool) -> Result<Vec<Media>> {
    crate::watch::collect(
        browser,
        watch(music),
        tokio_util::sync::CancellationToken::new(),
    )
    .await
}
#[tokio::test]
async fn collection_reads_all_pages_for_video_and_music_before_returning() {
    for music in [false, true] {
        let (browser, calls, task) =
            provider(|n| page(if n == 0 { 2 } else { 1 }, (n == 0).then_some("next"))).await;
        let result = collect(browser, music).await;
        task.abort();
        assert_eq!(result.unwrap().len(), 3);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
}
#[tokio::test]
async fn collection_continuation_cycles_fail_without_returning_partial_items() {
    let (browser, calls, task) = provider(|_| page(1, Some("repeated"))).await;
    let result = collect(browser, false).await;
    task.abort();
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("repeated a continuation")
    );
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}
#[tokio::test]
async fn collection_entry_limit_accepts_2000_but_rejects_2001() {
    for count in [2000, 2001] {
        let (browser, _, task) = provider(move |_| page(count, None)).await;
        let result = collect(browser, false).await;
        task.abort();
        if count == 2000 {
            assert_eq!(result.unwrap().len(), 2000);
        } else {
            assert!(result.unwrap_err().to_string().contains("2,000-entry"));
        }
    }
}
#[tokio::test]
async fn collection_page_limit_accepts_a_terminal_32nd_page() {
    for terminal in [false, true] {
        let (browser, calls, task) = provider(move |n| {
            page(
                1,
                (!(terminal && n == 31))
                    .then(|| format!("next-{n}"))
                    .as_deref(),
            )
        })
        .await;
        let result = collect(browser, false).await;
        task.abort();
        assert_eq!(calls.load(Ordering::SeqCst), 32);
        if terminal {
            assert_eq!(result.unwrap().len(), 32);
        } else {
            assert!(result.unwrap_err().to_string().contains("32-page"));
        }
    }
}
#[tokio::test]
async fn collection_cancellation_before_start_never_contacts_the_provider() {
    let (browser, calls, task) = provider(|_| panic!("Cancelled collection contacted API")).await;
    let cancel = tokio_util::sync::CancellationToken::new();
    cancel.cancel();
    let result = crate::watch::collect(browser, watch(false), cancel).await;
    task.abort();
    assert!(result.unwrap_err().to_string().contains("cancelled"));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn collection_budget_counts_all_thumbnail_choices_and_the_selected_copy() {
    let (browser, _, task) = provider(|_| {
        let mut data = page(1, None);
        data["contents"][0]["videoRenderer"]["thumbnail"] = json!({"thumbnails":[
            {"url":format!("https://images.test/{}", "a".repeat(6 * 1024 * 1024)),"width":160,"height":90},
            {"url":format!("https://images.test/{}", "b".repeat(6 * 1024 * 1024)),"width":320,"height":180}
        ]}); data
    }).await;
    let result = collect(browser, false).await;
    task.abort();
    match result {
        Err(error) => assert!(error.to_string().contains("16 MiB"), "{error}"),
        Ok(_) => panic!("Nested thumbnail data bypassed the collection memory budget"),
    }
}
