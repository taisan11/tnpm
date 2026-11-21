use super::*;

const KEY: &str = "https://registry.test/shared#install-v1";

pub(super) fn policy() -> Policy {
    let now = SystemTime::now();
    Policy::new(
        &reqwest::header::HeaderMap::new(),
        &http_cache::request_headers(),
        now,
        now,
    )
}

fn document(text: &str) -> Document {
    Document::open(
        cache::encode(KEY, &Value::parse(text).unwrap(), &policy()).unwrap(),
        KEY,
    )
    .unwrap()
    .unwrap()
}

#[test]
fn borrowed_selection_matches_indexed_selection() {
    let documents = [
        r#"{"dist-tags":{"latest":"2.0.0","alias":"1.0.0+z"},"versions":{"1.0.0+z":{"version":"1.0.0+z"},"1.0.0+a":{"version":"1.0.0+a"},"1.0.0":{"version":"1.0.0","dependencies":{"x":"1"}},"2.0.0-beta.1":{"version":"2.0.0-beta.1"},"2.0.0":{"version":"2.0.0"}}}"#,
        r#"{"dist-tags":{"latest":"1.0.0"},"versions":{"1.0.0":{"version":"first"},"1.0.0":{"version":"second"}},"versions":{"2.0.0":{}}}"#,
        r#"{"dist-tags":{"ali\u0061s":"1.0.0"},"versions":{"1.0.\u0030":{"version":"1.0.0","bin":"日本語\\cli.js"}}}"#,
        r#"{"versions":{}}"#,
        r#"{"versions":{"broken":{},"1.0.0":{"version":"1.0.0"}}}"#,
        r#"{"versions":{"1.0.0":null}}"#,
        r#"{"versions":[]}"#,
        r#"{}"#,
    ];
    for text in documents {
        let indexed = document(text);
        for selector in [
            "latest",
            "alias",
            "v1.0.0",
            "1.0.0",
            "*",
            "^1",
            ">=1",
            "^2",
            "2.0.0-beta.1",
            "missing",
            "9",
        ] {
            let expected = resolve_binary(&indexed, Query::versions(selector, true));
            let actual =
                resolve_raw(text, selector, &mut Profile::new("test")).map(|(package, _)| package);
            assert_eq!(actual.is_ok(), expected.is_ok(), "{text}, {selector}");
            if let (Ok(actual), Ok(expected)) = (actual, expected) {
                assert_eq!(actual.metadata, expected.metadata, "{text}, {selector}");
                assert_eq!(actual.versions, expected.versions, "{text}, {selector}");
            }
        }
    }
}

#[test]
fn borrowed_selection_validates_unselected_json() {
    for text in [
        r#"{"versions":{"1.0.0":{}},"unused":"\ud800"}"#,
        r#"{"versions":{"1.0.0":{},"2.0.0":{"unused":[1,]}}}"#,
        r#"{"versions":{"1.0.0":{}}} trailing"#,
    ] {
        assert!(Value::parse(text).is_err());
        assert!(resolve_raw(text, "1.0.0", &mut Profile::new("test")).is_err());
    }
}

#[test]
fn pending_documents_share_waiters_replace_old_entries_and_drain() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            remember(KEY, Rc::new(document(r#"{"versions":{"1.0.0":{}}}"#)));
            let (sender, ready) = tokio::sync::oneshot::channel::<Option<Document>>();
            let result = async move { ready.await.unwrap().map(Rc::new) }
                .boxed_local()
                .shared();
            PENDING.with(|pending| {
                pending.borrow_mut().push_back(PendingDocument {
                    key: KEY.into(),
                    bytes: MAX_DOCUMENT_BYTES,
                    result,
                })
            });
            let (a, b, ()) = futures_util::future::join3(
                load_cached(None, KEY),
                load_cached(None, KEY),
                async {
                    tokio::task::yield_now().await;
                    assert!(
                        sender
                            .send(Some(document(r#"{"versions":{"2.0.0":{}}}"#)))
                            .is_ok()
                    );
                },
            )
            .await;
            let (a, b) = (a.unwrap().unwrap(), b.unwrap().unwrap());
            assert!(Rc::ptr_eq(&a, &b));
            assert!(resolve_binary(&a, Query::versions("2", true)).is_ok());
            assert!(
                resolve_binary(
                    &load_cached(None, KEY).await.unwrap().unwrap(),
                    Query::versions("2", true)
                )
                .is_ok()
            );
            assert!(PENDING.with(|pending| pending.borrow().is_empty()));
            let result = async { None }.boxed_local().shared();
            PENDING.with(|pending| {
                pending.borrow_mut().push_back(PendingDocument {
                    key: "failed".into(),
                    bytes: MAX_DOCUMENT_BYTES,
                    result,
                })
            });
            pending_capacity(1).await;
            assert!(PENDING.with(|pending| pending.borrow().is_empty()));
            pending_capacity(MAX_BODY).await;
            finish_pending().await;
            DOCUMENTS.with(|documents| *documents.borrow_mut() = DocumentCache::default());
        });
}
