use serde_json::json;
use ytui_mobile::Engine;
fn engine() -> (Engine, tempfile::TempDir) {
    let root = tempfile::tempdir().unwrap();
    let engine = Engine::new(&root.path().join("files"), &root.path().join("cache")).unwrap();
    (engine, root)
}
fn media(id: &str) -> ytui_core::Media {
    ytui_core::Media {
        url: format!("https://www.youtube.com/watch?v={id}"),
        title: format!("Fixture {id}"),
        ..Default::default()
    }
}
#[tokio::test]
async fn mobile_queue_saved_and_resume_are_durable_and_checked() {
    let (e, root) = engine();
    let m = media("abcdefghijk");
    e.request(json!({"op":"enqueue","media":m})).await.unwrap();
    e.request(json!({"op":"save_media","media":m}))
        .await
        .unwrap();
    e.request(json!({"op":"position","url":m.url,"milliseconds":12345}))
        .await
        .unwrap();
    let restored = Engine::new(&root.path().join("files"), &root.path().join("cache")).unwrap();
    assert_eq!(
        restored
            .request(json!({"op":"queue"}))
            .await
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        restored
            .request(json!({"op":"position","url":m.url}))
            .await
            .unwrap()["milliseconds"],
        12345
    );
    assert!(
        restored
            .request(json!({"op":"queue_remove","index":9}))
            .await
            .is_err()
    );
    assert!(
        restored
            .request(json!({"op":"position","url":m.url,"milliseconds":u64::MAX}))
            .await
            .is_err()
    );
}
#[tokio::test]
async fn queue_reordering_is_durable_and_stale_actions_cannot_remove_the_wrong_item() {
    let (e, root) = engine();
    let a = media("abcdefghijk");
    let b = media("lmnopqrstuv");
    let c = media("zyxwvutsrqp");
    for item in [&a, &b, &c] {
        e.request(json!({"op":"enqueue","media":item}))
            .await
            .unwrap();
    }
    let changed = e
        .request(json!({"op":"queue_move","from":2,"to":0,"url":c.url}))
        .await
        .unwrap();
    assert_eq!(changed["items"][0]["url"], c.url);
    e.request(json!({"op":"queue_commit","url":c.url}))
        .await
        .unwrap();
    assert!(
        e.request(json!({"op":"queue_remove","index":1,"url":a.url}))
            .await
            .is_err()
    );
    assert!(
        e.request(json!({"op":"queue_move","from":0,"to":99,"url":a.url}))
            .await
            .is_err()
    );
    let restored = Engine::new(&root.path().join("files"), &root.path().join("cache")).unwrap();
    let queue = restored.request(json!({"op":"queue"})).await.unwrap();
    assert_eq!(queue[0]["url"], a.url);
    assert_eq!(queue[1]["url"], b.url);
    // A failed persistence attempt must not move the visible in-memory queue.
    std::fs::remove_file(root.path().join("files/library.json")).unwrap();
    std::fs::create_dir(root.path().join("files/library.json")).unwrap();
    assert!(
        e.request(json!({"op":"queue_move","from":0,"to":1,"url":a.url}))
            .await
            .is_err()
    );
    assert_eq!(e.request(json!({"op":"queue"})).await.unwrap(), queue);
}
#[tokio::test]
async fn failed_disk_write_never_publishes_an_in_memory_queue_change() {
    let (e, root) = engine();
    std::fs::create_dir(root.path().join("files/library.json")).unwrap();
    assert!(
        e.request(json!({"op":"enqueue","media":media("abcdefghijk")}))
            .await
            .is_err()
    );
    assert_eq!(e.request(json!({"op":"queue"})).await.unwrap(), json!([]));
}
#[tokio::test]
async fn queue_peek_keeps_a_failed_playable_item_until_matching_commit() {
    let (e, _root) = engine();
    let a = media("abcdefghijk");
    let b = media("lmnopqrstuv");
    for m in [&a, &b] {
        e.request(json!({"op":"enqueue","media":m})).await.unwrap();
    }
    assert_eq!(
        e.request(json!({"op":"queue_peek"})).await.unwrap()["url"],
        a.url
    );
    e.request(json!({"op":"queue_commit","url":"https://wrong.test"}))
        .await
        .unwrap();
    assert_eq!(
        e.request(json!({"op":"queue"}))
            .await
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        2
    );
    e.request(json!({"op":"queue_commit","url":a.url}))
        .await
        .unwrap();
    assert_eq!(
        e.request(json!({"op":"queue_peek"})).await.unwrap()["url"],
        b.url
    );
}
#[tokio::test]
async fn settings_reject_invalid_modes_without_overwriting_previous_values() {
    let (e, _root) = engine();
    let old = e.request(json!({"op":"settings"})).await.unwrap();
    let mut bad = old.clone();
    bad["music_player_style"] = json!("unknown");
    assert!(
        e.request(json!({"op":"save_settings","config":bad}))
            .await
            .is_err()
    );
    assert_eq!(e.request(json!({"op":"settings"})).await.unwrap(), old);
    for name in ytui_core::theme::THEMES {
        let mut config = old.clone();
        config["theme"] = json!(name);
        e.request(json!({"op":"save_settings","config":config}))
            .await
            .unwrap();
        let theme = e.request(json!({"op":"theme","dark":false})).await.unwrap();
        assert!(theme["text"].is_array());
    }
}
#[tokio::test]
async fn authentication_import_is_scoped_and_disconnect_preserves_local_media() {
    let (e, root) = engine();
    assert!(
        e.request(json!({"op":"import_cookies","header":"SID=invalid"}))
            .await
            .is_err()
    );
    e.request(
        json!({"op":"import_cookies","header":"SAPISID=synthetic-session; SID=synthetic-only"}),
    )
    .await
    .unwrap();
    assert_eq!(
        e.request(json!({"op":"account"})).await.unwrap()["connected"],
        true
    );
    e.request(json!({"op":"save_media","media":media("abcdefghijk")}))
        .await
        .unwrap();
    e.request(json!({"op":"disconnect"})).await.unwrap();
    assert!(!root.path().join("files/account.json").exists());
    assert_eq!(
        e.request(json!({"op":"account"})).await.unwrap()["connected"],
        false
    );
    assert_eq!(
        e.request(json!({"op":"saved"}))
            .await
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        1
    );
}
#[tokio::test]
async fn unknown_requests_and_oversized_media_fail_with_bounded_state() {
    let (e, _root) = engine();
    assert!(e.request(json!({"op":"unknown"})).await.is_err());
    let mut m = media("abcdefghijk");
    m.title = "x".repeat(300 * 1024);
    assert!(e.request(json!({"op":"enqueue","media":m})).await.is_err());
    assert_eq!(e.request(json!({"op":"queue"})).await.unwrap(), json!([]));
}

#[tokio::test]
async fn imported_palette_is_private_durable_and_invalid_import_does_not_replace_it() {
    let (e, root) = engine();
    let source = "background = '#101418'\nforeground = '#e5edf4'\naccent = '#6ddbc1'";
    let palette = e
        .request(json!({"op":"import_theme","text":source}))
        .await
        .unwrap();
    assert_eq!(palette["source"], "Omarchy");
    assert!(
        e.request(json!({"op":"import_theme","text":"invalid"}))
            .await
            .is_err()
    );
    let restored = Engine::new(&root.path().join("files"), &root.path().join("cache")).unwrap();
    assert_eq!(
        restored.request(json!({"op":"theme"})).await.unwrap(),
        palette
    );
    for proxy in [
        "socks5://localhost:9000",
        "http://user:password@localhost:9000",
    ] {
        let mut config = restored.request(json!({"op":"settings"})).await.unwrap();
        config["proxy"] = json!(proxy);
        assert!(
            restored
                .request(json!({"op":"save_settings","config":config}))
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn disabled_sponsorblock_does_not_contact_the_api() {
    let (e, _root) = engine();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut config = e.request(json!({"op":"settings"})).await.unwrap();
    config["playback_sponsorblock"] = json!("off");
    config["proxy"] = json!(format!("http://{}", listener.local_addr().unwrap()));
    e.request(json!({"op":"save_settings","config":config}))
        .await
        .unwrap();
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        e.request(json!({"op":"sponsor","media":media("abcdefghijk")})),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(result, json!([]));
    assert!(listener.into_std().unwrap().accept().is_err());
}
