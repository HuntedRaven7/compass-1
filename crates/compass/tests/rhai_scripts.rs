//! Rhai scripts through the engine: root search, the view the launcher
//! follows, actions, consent, and hot reload.
//!
//! In-process, against `serve::handle`, so the scripts' services can be a
//! `MemoryHost` (a fake clipboard, opener and store) and nothing reaches the
//! real clipboard, keyring or session bus. Every path — the packaged and user
//! script directories, the consent file — is in a tempdir; nothing reads the
//! environment. `engine_end_to_end.rs` covers the real process and the XDG
//! variables.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use compass::rhai_scripts::{Config, RhaiScripts, SEARCH_HANDLER};
use compass::serve::{EngineState, handle};
use compass_core::{AppIndex, JsonFrecencyStore, SystemClock};
use compass_extension_api::View;
use compass_ipc::{ErrorKind, Request, Response, SocketPath};
use compass_script::{HostCall, MemoryHost, ProcessOutcome};
use tokio::sync::RwLock;

const EXAMPLES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../extensions/rhai-examples"
);

struct Fixture {
    dir: tempfile::TempDir,
    host: Arc<MemoryHost>,
    state: Arc<RwLock<EngineState>>,
}

impl Fixture {
    fn user(&self) -> PathBuf {
        self.dir.path().join("data-home/compass/scripts")
    }

    fn packaged(&self) -> PathBuf {
        self.dir.path().join("usr/share/compass/scripts")
    }

    fn consent_file(&self) -> PathBuf {
        self.dir.path().join("config/compass/script-grants.json")
    }

    /// A fresh engine over the same directories, as after a restart.
    async fn restart(&mut self) {
        let config = Config {
            search_paths: vec![self.user(), self.packaged()],
            user_dir: Some(self.user()),
            consent_file: Some(self.consent_file()),
        };
        self.state = engine(config, &self.host).await;
    }

    async fn ask(&self, request: Request) -> Response {
        handle(&self.state, request).await
    }

    async fn open(&self, id: &str) -> u64 {
        match self
            .ask(Request::RunExtensionCommand {
                id: format!("rhai:{id}"),
                arguments_json: None,
            })
            .await
        {
            Response::ExtensionStarted { session } => session,
            other => panic!("{id} did not open: {other:?}"),
        }
    }

    async fn event(&self, session: u64, handler: &str, args: serde_json::Value) -> Response {
        self.ask(Request::ExtensionEvent {
            session,
            handler: handler.to_owned(),
            args_json: args.to_string(),
        })
        .await
    }

    /// Polls `session` until `done` accepts what it shows.
    async fn wait(&self, session: u64, done: impl Fn(&Shown) -> bool) -> Shown {
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut after = 0;
        loop {
            let answer = self.ask(Request::ExtensionView { session, after }).await;
            let Response::ExtensionView {
                version,
                view_json,
                problem,
                ended,
                alert,
                toast,
                ..
            } = answer
            else {
                panic!("no view answer: {answer:?}");
            };
            let shown = Shown {
                view: view_json.map(|json| serde_json::from_str(&json).expect("a View")),
                problem,
                ended,
                alert: alert.map(|alert| alert.title),
                toast: toast.map(|toast| toast.title),
            };
            if done(&shown) {
                return shown;
            }
            assert!(Instant::now() < deadline, "never got there: {shown:?}");
            after = version;
        }
    }

    async fn titles(&self, query: &str) -> Vec<String> {
        match self
            .ask(Request::Query {
                text: query.to_owned(),
            })
            .await
        {
            Response::QueryResults { hits } => hits.into_iter().map(|hit| hit.title).collect(),
            other => panic!("no results: {other:?}"),
        }
    }
}

#[derive(Debug)]
struct Shown {
    view: Option<View>,
    problem: Option<String>,
    ended: bool,
    alert: Option<String>,
    toast: Option<String>,
}

impl Shown {
    fn rows(&self) -> Vec<&compass_extension_api::ListItem> {
        match &self.view {
            Some(View::List(list)) => list.sections.iter().flat_map(|s| &s.items).collect(),
            _ => Vec::new(),
        }
    }
}

async fn engine(config: Config, host: &Arc<MemoryHost>) -> Arc<RwLock<EngineState>> {
    let mut state = EngineState::with_index(
        AppIndex::builder().build(),
        Box::new(JsonFrecencyStore::in_memory(Arc::new(SystemClock))),
        SocketPath::exact("/nonexistent/compass-test/ipc.sock"),
        50,
    );
    let host: Arc<dyn compass_script::ScriptHost> = host.clone();
    state.set_rhai_scripts(RhaiScripts::new(config, host));
    Arc::new(RwLock::new(state))
}

async fn fixture(prepare: impl FnOnce(&Path, &Path)) -> Fixture {
    let dir = tempfile::tempdir().expect("tempdir");
    let host = Arc::new(MemoryHost::new());
    let user = dir.path().join("data-home/compass/scripts");
    let packaged = dir.path().join("usr/share/compass/scripts");
    std::fs::create_dir_all(&user).unwrap();
    std::fs::create_dir_all(&packaged).unwrap();
    prepare(&user, &packaged);
    let config = Config {
        search_paths: vec![user.clone(), packaged],
        user_dir: Some(user),
        consent_file: Some(dir.path().join("config/compass/script-grants.json")),
    };
    let state = engine(config, &host).await;
    Fixture { dir, host, state }
}

fn copy_example(name: &str, to: &Path) {
    let from = Path::new(EXAMPLES).join(name);
    let to = to.join(name);
    std::fs::create_dir_all(&to).unwrap();
    for entry in std::fs::read_dir(&from).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(entry.path(), to.join(entry.file_name())).unwrap();
    }
}

fn script(root: &Path, name: &str, manifest: &str, source: &str) {
    let dir = root.join(name);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("script.toml"), manifest).unwrap();
    std::fs::write(dir.join("main.rhai"), source).unwrap();
}

const COPIER: &str = r#"
fn search(query) {
    [#{ id: "row", title: `Copy ${query}`, actions: [#{ title: "Copy", copy: `copied ${query}` }] }]
}
"#;

#[tokio::test(flavor = "multi_thread")]
async fn a_packaged_script_is_in_root_search_and_its_view_searches_and_copies() {
    let fx = fixture(|_, packaged| {
        for example in [
            "unit-converter",
            "web-search",
            "epoch-converter",
            "generators",
            "quick-notes",
            "system-health",
        ] {
            copy_example(example, packaged);
        }
    })
    .await;

    let Response::RhaiScripts { scripts } = fx.ask(Request::ListRhaiScripts).await else {
        panic!("no script list");
    };
    let ids: Vec<&str> = scripts.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "script.epoch-converter",
            "script.generators",
            "script.quick-notes",
            "script.system-health",
            "script.unit-converter",
            "script.web-search",
        ]
    );

    // Found by title and by a manifest keyword, next to everything else.
    assert!(
        fx.titles("unit conv")
            .await
            .contains(&"Unit Converter".to_owned())
    );
    assert!(
        fx.titles("fahrenheit")
            .await
            .contains(&"Unit Converter".to_owned())
    );
    let Response::QueryResults { hits } = fx
        .ask(Request::Query {
            text: "unit converter".into(),
        })
        .await
    else {
        panic!()
    };
    assert_eq!(hits[0].id, "rhai:script.unit-converter");

    // Packaged: granted what it declares, so it opens without a prompt.
    let session = fx.open("script.unit-converter").await;
    let empty = fx.wait(session, |s| s.view.is_some()).await;
    assert_eq!(empty.alert, None);
    let Some(View::List(list)) = &empty.view else {
        panic!("not a list: {empty:?}");
    };
    assert_eq!(list.search.placeholder.as_deref(), Some("10 km to mi"));
    assert_eq!(
        list.search.on_change.as_ref().map(|h| h.0.as_str()),
        Some(SEARCH_HANDLER),
        "the script filters, so the launcher sends it the text"
    );

    assert_eq!(
        fx.event(
            session,
            SEARCH_HANDLER,
            serde_json::json!(["10 km to mi", 1])
        )
        .await,
        Response::Ack
    );
    let converted = fx.wait(session, |s| !s.rows().is_empty()).await;
    let first = converted.rows()[0].clone();
    assert!(first.title.ends_with(" mi"), "{first:?}");

    let copy = first.actions.as_ref().expect("actions").actions()[0].clone();
    assert_eq!(
        fx.event(session, &copy.handler.0, serde_json::json!([]))
            .await,
        Response::Ack
    );
    let calls = fx.host.calls();
    let [HostCall::Copy(copied)] = calls.as_slice() else {
        panic!("not one copy: {calls:?}");
    };
    assert!(
        first.title.starts_with(copied.as_str()),
        "{copied} vs {first:?}"
    );

    // A token from a view that is gone is refused, not run.
    assert_eq!(
        fx.event(
            session,
            SEARCH_HANDLER,
            serde_json::json!(["3 lb in kg", 2])
        )
        .await,
        Response::Ack
    );
    fx.wait(session, |s| {
        s.rows().first().is_some_and(|r| r.title.ends_with(" kg"))
    })
    .await;
    let Response::Error(stale) = fx
        .event(session, &copy.handler.0, serde_json::json!([]))
        .await
    else {
        panic!("a stale action ran");
    };
    assert_eq!(stale.kind, ErrorKind::BadRequest);
    assert_eq!(fx.host.calls().len(), 1);

    assert_eq!(
        fx.ask(Request::CloseExtension { session }).await,
        Response::Ack
    );
    let Response::Error(closed) = fx.ask(Request::ExtensionView { session, after: 0 }).await else {
        panic!("a closed view still answers");
    };
    assert_eq!(closed.kind, ErrorKind::BadRequest);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_keystroke_that_arrives_late_does_not_replace_the_newer_search() {
    // Each keystroke is its own request, so "3 lb" can land after
    // "3 lb in kg"; the launcher's count says which is newer.
    let fx = fixture(|_, packaged| copy_example("unit-converter", packaged)).await;
    let session = fx.open("script.unit-converter").await;
    fx.wait(session, |s| s.view.is_some()).await;

    for (text, count) in [("3 lb in kg", 9), ("3 lb", 4), ("3 lb in", 7)] {
        assert_eq!(
            fx.event(session, SEARCH_HANDLER, serde_json::json!([text, count]))
                .await,
            Response::Ack
        );
    }
    fx.wait(session, |s| {
        s.rows().first().is_some_and(|r| r.title.ends_with(" kg"))
    })
    .await;
    // Long enough for a dropped search to have rendered, had it run.
    tokio::time::sleep(Duration::from_millis(500)).await;
    let later = fx.wait(session, |_| true).await;
    assert!(
        later.rows()[0].title.ends_with(" kg"),
        "an older text replaced the newest: {:?}",
        later.rows()[0]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_user_script_asks_before_it_gets_a_capability_and_the_answer_is_kept() {
    let mut fx = fixture(|user, _| {
        script(
            user,
            "copier",
            "title = \"Copier\"\ncapabilities = [\"clipboard.write\"]",
            COPIER,
        );
    })
    .await;

    // Opening asks first, and nothing of the script runs meanwhile.
    let session = fx.open("script.copier").await;
    let asked = fx.wait(session, |s| s.alert.is_some()).await;
    assert_eq!(asked.alert.as_deref(), Some("Allow Copier to:"));
    assert!(asked.view.is_none());
    let Response::Error(waiting) = fx
        .event(session, SEARCH_HANDLER, serde_json::json!(["x", 1]))
        .await
    else {
        panic!("a script ran before it was allowed");
    };
    assert!(waiting.message.contains("waiting"), "{waiting:?}");

    // Refusing ends the view, grants nothing, and records nothing.
    assert_eq!(
        fx.ask(Request::ExtensionAlertAnswer {
            session,
            confirmed: false,
        })
        .await,
        Response::Ack
    );
    let refused = fx.wait(session, |s| s.ended).await;
    assert!(
        refused
            .problem
            .as_deref()
            .is_some_and(|p| p.contains("not allowed")),
        "{refused:?}"
    );
    assert!(!fx.consent_file().exists());
    assert!(fx.host.calls().is_empty());

    // So the next open asks again; allowing renders it, and its copy works.
    let session = fx.open("script.copier").await;
    fx.wait(session, |s| s.alert.is_some()).await;
    assert_eq!(
        fx.ask(Request::ExtensionAlertAnswer {
            session,
            confirmed: true,
        })
        .await,
        Response::Ack
    );
    let shown = fx.wait(session, |s| !s.rows().is_empty()).await;
    assert_eq!(shown.alert, None);
    let copy = shown.rows()[0].actions.as_ref().unwrap().actions()[0].clone();
    assert_eq!(
        fx.event(session, &copy.handler.0, serde_json::json!([]))
            .await,
        Response::Ack
    );
    assert_eq!(fx.host.calls(), [HostCall::Copy("copied ".to_owned())]);
    let kept: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(fx.consent_file()).unwrap()).unwrap();
    assert_eq!(
        kept["scripts"]["script.copier"],
        serde_json::json!(["clipboard.write"])
    );

    // After a restart it is still allowed: no prompt.
    fx.restart().await;
    let session = fx.open("script.copier").await;
    let shown = fx.wait(session, |s| !s.rows().is_empty()).await;
    assert_eq!(shown.alert, None);

    // A capability it starts declaring later is asked for; the old one is
    // not asked again.
    script(
        &fx.user(),
        "copier",
        "title = \"Copier\"\ncapabilities = [\"clipboard.write\", \"storage.read\"]",
        COPIER,
    );
    let session = fx.open("script.copier").await;
    let asked = fx.wait(session, |s| s.alert.is_some()).await;
    assert_eq!(asked.alert.as_deref(), Some("Allow Copier to:"));
}

#[tokio::test(flavor = "multi_thread")]
async fn script_permissions_are_listed_and_revoking_asks_again() {
    let fx = fixture(|user, _| {
        script(
            user,
            "copier",
            "title = \"Copier\"\ncapabilities = [\"clipboard.write\"]",
            COPIER,
        );
    })
    .await;
    assert_eq!(
        fx.ask(Request::ListScriptGrants).await,
        Response::ScriptGrants { grants: vec![] },
        "nothing allowed yet"
    );
    let session = fx.open("script.copier").await;
    fx.wait(session, |s| s.alert.is_some()).await;
    assert_eq!(
        fx.ask(Request::ExtensionAlertAnswer {
            session,
            confirmed: true,
        })
        .await,
        Response::Ack
    );
    fx.wait(session, |s| !s.rows().is_empty()).await;
    let Response::ScriptGrants { grants } = fx.ask(Request::ListScriptGrants).await else {
        panic!("no permissions listed");
    };
    assert_eq!(
        grants,
        [compass_ipc::ScriptGrantEntry {
            id: "script.copier".into(),
            title: "Copier".into(),
            capabilities: vec!["clipboard.write".into()],
            descriptions: vec!["copy to the clipboard".into()],
        }]
    );

    // Revoking empties the file's entry, ends the open view, and the next
    // open asks again.
    assert_eq!(
        fx.ask(Request::RevokeScriptGrant {
            id: "script.copier".into()
        })
        .await,
        Response::ScriptGrants { grants: vec![] }
    );
    let ended = fx.wait(session, |s| s.ended).await;
    assert!(
        ended
            .problem
            .as_deref()
            .is_some_and(|p| p.contains("revoked")),
        "{ended:?}"
    );
    let kept: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(fx.consent_file()).unwrap()).unwrap();
    assert_eq!(kept["scripts"], serde_json::json!({}));
    let session = fx.open("script.copier").await;
    let asked = fx.wait(session, |s| s.alert.is_some()).await;
    assert_eq!(asked.alert.as_deref(), Some("Allow Copier to:"));

    let Response::Error(err) = fx
        .ask(Request::RevokeScriptGrant {
            id: "script.copier".into(),
        })
        .await
    else {
        panic!("revoking what is not recorded was not refused");
    };
    assert_eq!(err.kind, ErrorKind::BadRequest);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_capability_that_is_not_granted_is_refused_by_name() {
    let fx = fixture(|user, packaged| {
        // Uses the clipboard without declaring it: the function does not
        // exist, so the script does not load.
        script(
            packaged,
            "undeclared",
            "title = \"Undeclared\"",
            "fn search(q) { clipboard::copy(q); [] }",
        );
        // Copies from an action without declaring it: the view is refused
        // when it is built, naming the capability.
        script(user, "ungranted", "title = \"Ungranted\"", COPIER);
    })
    .await;

    let session = fx.open("script.undeclared").await;
    let shown = fx.wait(session, |s| s.problem.is_some()).await;
    let problem = shown.problem.unwrap();
    assert!(
        problem.contains("clipboard") && problem.contains("is not available to this script"),
        "{problem}"
    );

    // Nothing declared, so nothing to ask; the copy action names what is
    // missing instead of running.
    let session = fx.open("script.ungranted").await;
    let shown = fx
        .wait(session, |s| s.problem.is_some() || s.view.is_some())
        .await;
    assert_eq!(shown.alert, None);
    let problem = shown.problem.expect("the view is refused");
    assert!(problem.contains("clipboard.write"), "{problem}");
    assert!(fx.host.calls().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn an_edit_on_disk_reaches_root_search_and_an_open_view() {
    let fx = fixture(|user, _| {
        script(
            user,
            "hello",
            "title = \"Hello There\"",
            r#"fn search(q) { [#{ title: "first" }] }"#,
        );
    })
    .await;
    tokio::spawn(compass::rhai_scripts::watch(Arc::clone(&fx.state)));
    // Give the watcher a moment to arm before the edit it has to see.
    tokio::time::sleep(Duration::from_millis(300)).await;

    let session = fx.open("script.hello").await;
    fx.wait(session, |s| {
        s.rows().first().is_some_and(|r| r.title == "first")
    })
    .await;

    script(
        &fx.user(),
        "hello",
        "title = \"Hello Again\"",
        r#"fn search(q) { [#{ title: "second" }] }"#,
    );
    // The open view re-renders with the new source …
    fx.wait(session, |s| {
        s.rows().first().is_some_and(|r| r.title == "second")
    })
    .await;
    // … and root search has the new title, without a rescan request.
    let deadline = Instant::now() + Duration::from_secs(20);
    while !fx.titles("hello").await.contains(&"Hello Again".to_owned()) {
        assert!(Instant::now() < deadline, "root search never saw the edit");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    // A new script, and a broken one, appear too; the broken one says why
    // when opened.
    script(
        &fx.user(),
        "broken",
        "title = \"Broken Script\"",
        "fn search(q) {",
    );
    let deadline = Instant::now() + Duration::from_secs(20);
    while !fx
        .titles("broken")
        .await
        .contains(&"Broken Script".to_owned())
    {
        assert!(
            Instant::now() < deadline,
            "root search never saw the new script"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let broken = fx.open("script.broken").await;
    let shown = fx.wait(broken, |s| s.problem.is_some()).await;
    assert!(shown.problem.unwrap().contains("Broken Script cannot run"));

    // Removing a script ends its view and drops it from root search.
    std::fs::remove_dir_all(fx.user().join("hello")).unwrap();
    let gone = fx.wait(session, |s| s.ended).await;
    assert!(gone.problem.unwrap().contains("removed"));
    let deadline = Instant::now() + Duration::from_secs(20);
    while fx.titles("hello").await.contains(&"Hello Again".to_owned()) {
        assert!(
            Instant::now() < deadline,
            "root search kept a removed script"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_action_can_toast_rerender_and_leave() {
    let fx = fixture(|user, _| {
        script(
            user,
            "counter",
            "title = \"Counter\"",
            r#"
            fn bump() { #{ rerender: true, toast: "Bumped", style: "success" } }
            fn leave() { #{ pop: true } }
            fn search(q) {
                [
                    #{ title: `Query: ${q}`, actions: [#{ title: "Bump", run: "bump" }] },
                    #{ title: "Leave", actions: [#{ title: "Leave", run: "leave" }] },
                ]
            }
            "#,
        );
    })
    .await;
    let action = |shown: &Shown, row: usize| {
        shown.rows()[row].actions.as_ref().unwrap().actions()[0]
            .handler
            .0
            .clone()
    };
    let session = fx.open("script.counter").await;
    fx.wait(session, |s| s.rows().len() == 2).await;
    fx.event(session, SEARCH_HANDLER, serde_json::json!(["abc", 1]))
        .await;
    let shown = fx
        .wait(session, |s| {
            s.rows().first().is_some_and(|r| r.title == "Query: abc")
        })
        .await;

    assert_eq!(
        fx.event(session, &action(&shown, 0), serde_json::json!([]))
            .await,
        Response::Ack
    );
    let shown = fx.wait(session, |s| s.toast.is_some()).await;
    assert_eq!(shown.toast.as_deref(), Some("Bumped"));
    assert_eq!(
        shown.rows()[0].title,
        "Query: abc",
        "re-rendered with the same query"
    );

    assert_eq!(
        fx.event(session, &action(&shown, 1), serde_json::json!([]))
            .await,
        Response::Ack
    );
    fx.wait(session, |s| s.ended && s.problem.is_none()).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn system_health_probes_once_when_opened_and_copies_a_follow_up() {
    let fx = fixture(|_, packaged| copy_example("system-health", packaged)).await;
    let exited = |stdout: &str| ProcessOutcome::Exited {
        code: Some(0),
        stdout: stdout.to_owned(),
        stderr: String::new(),
    };
    fx.host
        .set_program(&["systemctl", "is-system-running"], exited("running\n"));
    fx.host.set_program(
        &[
            "systemctl",
            "list-units",
            "--failed",
            "--plain",
            "--no-legend",
            "--no-pager",
        ],
        exited("cups.service loaded failed failed CUPS Scheduler\n"),
    );
    let runs = |host: &MemoryHost| {
        host.calls()
            .iter()
            .filter(|call| matches!(call, HostCall::Run { .. }))
            .count()
    };

    // Root search lists it from its manifest alone: no program runs.
    assert!(
        fx.titles("system health")
            .await
            .contains(&"System Health".to_owned())
    );
    assert_eq!(runs(&fx.host), 0, "searching never probes the host");

    let session = fx.open("script.system-health").await;
    let shown = fx.wait(session, |s| !s.rows().is_empty()).await;
    assert_eq!(shown.alert, None, "packaged, so granted what it declares");
    let Some(View::List(list)) = &shown.view else {
        panic!("not a list: {shown:?}");
    };
    assert_eq!(
        list.search.on_change, None,
        "the launcher filters, so typing never reaches the script"
    );
    let unit = shown.rows()[0].clone();
    assert_eq!(unit.title, "cups.service");
    assert_eq!(unit.subtitle.as_deref(), Some("CUPS Scheduler"));
    let probes = runs(&fx.host);
    assert_eq!(probes, 6, "{:?}", fx.host.calls());

    let copy = unit.actions.as_ref().expect("actions").actions()[0].clone();
    assert_eq!(copy.title, "Copy Status Command");
    assert_eq!(
        fx.event(session, &copy.handler.0, serde_json::json!([]))
            .await,
        Response::Ack
    );
    assert_eq!(
        fx.host.calls().last(),
        Some(&HostCall::Copy("systemctl status cups.service".to_owned()))
    );
    assert_eq!(runs(&fx.host), probes, "copying runs nothing");

    // Only asking for it checks again.
    let refresh = list.actions.as_ref().expect("list actions").actions()[0].clone();
    assert_eq!(refresh.title, "Refresh");
    assert_eq!(
        fx.event(session, &refresh.handler.0, serde_json::json!([]))
            .await,
        Response::Ack
    );
    let deadline = Instant::now() + Duration::from_secs(20);
    while runs(&fx.host) < probes * 2 {
        assert!(Instant::now() < deadline, "{:?}", fx.host.calls());
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        runs(&fx.host),
        probes * 2,
        "one refresh, one round of probes"
    );
}
