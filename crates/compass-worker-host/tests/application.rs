//! `Application/*`, read against
//! `src/server/src/extension/api/application-service.hpp` and `figura/tsapi.fig`.

use std::cell::RefCell;

use compass_worker_host::application_service::{
    Application, ApplicationService, Apps, NO_DEFAULT, TerminalOptions,
};
use compass_worker_host::tsapi::Call;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Did {
    List,
    Openers(String),
    DefaultOpener(String),
    ById(String),
    Launch(String, String),
    ShowInFileBrowser(String, bool),
    RunInTerminal(Vec<String>, TerminalOptions),
}

#[derive(Default)]
struct Stub {
    all: Vec<Application>,
    openers: Vec<Application>,
    default_opener: Option<Application>,
    by_id: Option<Application>,
    terminal_ok: bool,
    did: RefCell<Vec<Did>>,
}

impl Apps for Stub {
    fn list(&self) -> Vec<Application> {
        self.did.borrow_mut().push(Did::List);
        self.all.clone()
    }
    fn openers(&self, target: &str) -> Vec<Application> {
        self.did.borrow_mut().push(Did::Openers(target.to_owned()));
        self.openers.clone()
    }
    fn default_opener(&self, target: &str) -> Option<Application> {
        self.did
            .borrow_mut()
            .push(Did::DefaultOpener(target.to_owned()));
        self.default_opener.clone()
    }
    fn by_id(&self, id: &str) -> Option<Application> {
        self.did.borrow_mut().push(Did::ById(id.to_owned()));
        self.by_id.clone()
    }
    fn launch(&self, app: &Application, target: &str) {
        self.did
            .borrow_mut()
            .push(Did::Launch(app.id.clone(), target.to_owned()));
    }
    fn show_in_file_browser(&self, target: &str, select: bool) {
        self.did
            .borrow_mut()
            .push(Did::ShowInFileBrowser(target.to_owned(), select));
    }
    fn run_in_terminal(&self, cmdline: &[String], options: &TerminalOptions) -> bool {
        self.did
            .borrow_mut()
            .push(Did::RunInTerminal(cmdline.to_vec(), options.clone()));
        self.terminal_ok
    }
}

fn app(id: &str) -> Application {
    Application {
        id: id.to_owned(),
        name: format!("{id} name"),
        icon: format!("file:///icons/{id}.png"),
        path: format!("/usr/share/applications/{id}"),
    }
}

fn call(method: &str, params: serde_json::Value) -> Call {
    serde_json::from_value(serde_json::json!({
        "jsonrpc": "2.0", "id": 11, "method": method, "params": params,
    }))
    .expect("a well-formed call")
}

fn reply(
    service: &ApplicationService<Stub>,
    method: &str,
    params: serde_json::Value,
) -> serde_json::Value {
    let answer = service.handle(&call(method, params)).expect("answered");
    let answer: serde_json::Value = serde_json::from_str(&answer).expect("a JSON reply");
    assert_eq!(answer["id"], 11, "{answer}");
    answer
}

fn did(service: &ApplicationService<Stub>) -> Vec<Did> {
    service.apps().did.borrow().clone()
}

#[test]
fn list_without_a_target_lists_everything_and_with_one_narrows() {
    // `auto apps = target ? m_appDb.findOpeners(target) : m_appDb.list();`
    let service = ApplicationService::new(Stub {
        all: vec![app("a.desktop"), app("b.desktop")],
        ..Stub::default()
    });
    let files = reply(&service, "Application/list", serde_json::json!({}));
    assert_eq!(files["result"].as_array().map(Vec::len), Some(2));
    assert_eq!(did(&service), [Did::List]);

    let service = ApplicationService::new(Stub {
        all: vec![app("a.desktop"), app("b.desktop")],
        openers: vec![app("b.desktop")],
        ..Stub::default()
    });
    let files = reply(
        &service,
        "Application/list",
        serde_json::json!({"target": "/tmp/x.pdf"}),
    );
    assert_eq!(files["result"].as_array().map(Vec::len), Some(1));
    assert_eq!(did(&service), [Did::Openers("/tmp/x.pdf".to_owned())]);
}

#[test]
fn an_application_carries_the_four_fields_app_to_tsapi_sets() {
    // `{.id, .name = displayName(), .icon = iconUrl(), .path}` -- `icon` is
    // optional in the IDL but assigned unconditionally there, so it is always
    // a key here too.
    let service = ApplicationService::new(Stub {
        all: vec![app("org.gnome.Nautilus.desktop")],
        ..Stub::default()
    });
    let answer = reply(&service, "Application/list", serde_json::json!({}));

    assert_eq!(
        answer["result"][0],
        serde_json::json!({
            "id": "org.gnome.Nautilus.desktop",
            "name": "org.gnome.Nautilus.desktop name",
            "icon": "file:///icons/org.gnome.Nautilus.desktop.png",
            "path": "/usr/share/applications/org.gnome.Nautilus.desktop",
        })
    );
}

#[test]
fn open_with_a_known_app_id_launches_it_and_asks_for_no_default() {
    let service = ApplicationService::new(Stub {
        by_id: Some(app("gimp.desktop")),
        default_opener: Some(app("eog.desktop")),
        ..Stub::default()
    });
    reply(
        &service,
        "Application/open",
        serde_json::json!({"target": "/tmp/a.png", "appId": "gimp.desktop"}),
    );

    assert_eq!(
        did(&service),
        [
            Did::ById("gimp.desktop".to_owned()),
            Did::Launch("gimp.desktop".to_owned(), "/tmp/a.png".to_owned()),
        ]
    );
}

#[test]
fn open_with_an_unknown_app_id_falls_through_to_the_default_opener() {
    // `if (auto app = findById(...)) { launch; return; }` -- the `return` is
    // inside the `if`, so a missing application is not a failed open.
    let service = ApplicationService::new(Stub {
        by_id: None,
        default_opener: Some(app("eog.desktop")),
        ..Stub::default()
    });
    reply(
        &service,
        "Application/open",
        serde_json::json!({"target": "/tmp/a.png", "appId": "not-installed.desktop"}),
    );

    assert_eq!(
        did(&service),
        [
            Did::ById("not-installed.desktop".to_owned()),
            Did::DefaultOpener("/tmp/a.png".to_owned()),
            Did::Launch("eog.desktop".to_owned(), "/tmp/a.png".to_owned()),
        ]
    );
}

#[test]
fn open_with_nothing_to_open_it_is_still_a_successful_void() {
    // No opener found: the C++ falls off the end and returns `Void::ok()`.
    // Silent, but not an error the extension can report.
    let service = ApplicationService::new(Stub::default());
    let answer = reply(
        &service,
        "Application/open",
        serde_json::json!({"target": "/tmp/mystery.zzz"}),
    );

    assert_eq!(answer["result"], serde_json::Value::Null);
    assert!(answer.get("error").is_none(), "{answer}");
    assert_eq!(
        did(&service),
        [Did::DefaultOpener("/tmp/mystery.zzz".to_owned())]
    );
}

#[test]
fn get_default_fails_with_the_cpp_message_when_there_is_none() {
    // `fail("No default application found")` -- the string an extension's
    // rejected promise carries.
    let service = ApplicationService::new(Stub::default());
    let answer = reply(
        &service,
        "Application/getDefault",
        serde_json::json!({"target": "/tmp/x.zzz"}),
    );

    assert_eq!(answer["error"], NO_DEFAULT);
    assert_eq!(NO_DEFAULT, "No default application found");

    let service = ApplicationService::new(Stub {
        default_opener: Some(app("eog.desktop")),
        ..Stub::default()
    });
    let answer = reply(
        &service,
        "Application/getDefault",
        serde_json::json!({"target": "/tmp/a.png"}),
    );
    assert_eq!(answer["result"]["id"], "eog.desktop");
}

#[test]
fn show_in_file_browser_carries_the_select_flag() {
    let service = ApplicationService::new(Stub::default());
    reply(
        &service,
        "Application/showInFileBrowser",
        serde_json::json!({"target": "/tmp/a.png", "select": true}),
    );
    assert_eq!(
        did(&service),
        [Did::ShowInFileBrowser("/tmp/a.png".to_owned(), true)]
    );

    let service = ApplicationService::new(Stub::default());
    reply(
        &service,
        "Application/showInFileBrowser",
        serde_json::json!({"target": "/tmp/a.png", "select": false}),
    );
    assert_eq!(
        did(&service),
        [Did::ShowInFileBrowser("/tmp/a.png".to_owned(), false)]
    );
}

#[test]
fn run_in_terminal_runs_nothing_and_answers_false() {
    // The engine is not sandboxed: a command line it started would run on
    // the host with the person's permissions. `HostCommand/run` is the
    // brokered way to run a host program.
    let service = ApplicationService::new(Stub {
        terminal_ok: true,
        ..Stub::default()
    });
    let answer = reply(
        &service,
        "Application/runInTerminal",
        serde_json::json!({"opts": {
            "cmdline": ["sh", "-c", "true"],
            "hold": true,
            "appId": "org.gnome.Console.desktop",
        }}),
    );

    assert_eq!(answer["result"], serde_json::Value::Bool(false));
    assert!(answer.get("error").is_none(), "{answer}");
    assert_eq!(did(&service), [], "no terminal was started");
}

#[test]
fn open_ignores_the_app_id_for_a_file_the_extension_may_have_written() {
    let support = tempfile::tempdir().unwrap();
    let jar = support.path().join("payload.jar");
    std::fs::write(&jar, b"PK").unwrap();
    let service = ApplicationService::new(Stub {
        by_id: Some(app("runner.desktop")),
        default_opener: Some(app("archive.desktop")),
        ..Stub::default()
    })
    .with_writable([support.path().to_path_buf()]);
    let target = jar.to_string_lossy().into_owned();
    let _ = reply(
        &service,
        "Application/open",
        serde_json::json!({"target": target, "appId": "runner.desktop"}),
    );

    assert!(
        !did(&service)
            .iter()
            .any(|did| matches!(did, Did::Launch(..))),
        "{:?}",
        did(&service)
    );
}

#[test]
fn open_shows_an_image_the_extension_wrote_with_the_default_viewer_only() {
    let support = tempfile::tempdir().unwrap();
    let image = support.path().join("cover.png");
    std::fs::write(&image, b"png").unwrap();
    let service = ApplicationService::new(Stub {
        by_id: Some(app("runner.desktop")),
        default_opener: Some(app("eog.desktop")),
        ..Stub::default()
    })
    .with_writable([support.path().to_path_buf()]);
    let url = url::Url::from_file_path(&image).unwrap().to_string();
    let _ = reply(
        &service,
        "Application/open",
        serde_json::json!({"target": url, "appId": "runner.desktop"}),
    );

    assert_eq!(
        did(&service),
        [
            Did::DefaultOpener(url.clone()),
            Did::Launch("eog.desktop".to_owned(), url)
        ]
    );
}

#[test]
fn open_keeps_the_app_id_for_a_file_elsewhere() {
    let support = tempfile::tempdir().unwrap();
    let service = ApplicationService::new(Stub {
        by_id: Some(app("eog.desktop")),
        ..Stub::default()
    })
    .with_writable([support.path().to_path_buf()]);
    let _ = reply(
        &service,
        "Application/open",
        serde_json::json!({"target": "/usr/share/backgrounds/hills.jpg", "appId": "eog.desktop"}),
    );

    assert_eq!(
        did(&service),
        [
            Did::ById("eog.desktop".to_owned()),
            Did::Launch(
                "eog.desktop".to_owned(),
                "/usr/share/backgrounds/hills.jpg".to_owned()
            )
        ]
    );
}

#[test]
fn another_services_call_is_not_answered_here() {
    let service = ApplicationService::new(Stub::default());
    assert!(
        service
            .handle(&call("Storage/get", serde_json::json!({})))
            .is_none()
    );
    assert!(did(&service).is_empty());
}

#[test]
fn an_event_is_not_answered_and_launches_nothing() {
    let service = ApplicationService::new(Stub {
        default_opener: Some(app("eog.desktop")),
        ..Stub::default()
    });
    let event: Call = serde_json::from_value(serde_json::json!({
        "jsonrpc": "2.0",
        "method": "Application/open",
        "params": {"target": "/tmp/a.png"},
    }))
    .unwrap();

    assert!(service.handle(&event).is_none());
    assert!(did(&service).is_empty());
}
