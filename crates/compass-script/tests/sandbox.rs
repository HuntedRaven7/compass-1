//! Negative sandbox tests (PLAN §8.2, "Rhai, from Phase 5").
//!
//! Each test names one way a script could escape or exhaust its sandbox and
//! asserts that it fails *closed*: with the specific error that says which
//! wall it hit, promptly, and without taking the caller down. A test that
//! only asserted "some error" would pass for the wrong reason the day the
//! engine stopped compiling scripts at all, so most of these also run a
//! positive control first.

mod common;

use std::time::{Duration, Instant};

use common::{load, runtime, try_load};
use compass_extension_api::{Capability, DenialReason};
use compass_script::{HostCall, LimitKind, Limits, ProcessOutcome, ScriptError};

/// Runs `search` expecting failure. The wall clock is pushed far out so that
/// a test about the budget or a size limit cannot pass or fail on the speed of
/// the machine; the timeout tests set their own.
fn search_err(source: &str, limits: Limits) -> ScriptError {
    let limits = Limits {
        timeout: Duration::from_secs(60),
        ..limits
    };
    let fixture = try_load(source, &[], &[], limits).expect("compiles");
    runtime()
        .block_on(fixture.instance.search(""))
        .expect_err("must fail closed")
}

fn load_err(source: &str, declared: &[&str], granted: &[&str]) -> ScriptError {
    try_load(source, declared, granted, Limits::default())
        .err()
        .expect("must not load")
}

// ---------------------------------------------------------------- budget/time

#[test]
fn an_infinite_loop_is_terminated_by_the_operation_budget() {
    let error = search_err("fn search(q) { loop { } }", Limits::default());
    assert!(
        matches!(error, ScriptError::BudgetExceeded { budget } if budget == Limits::DEFAULT.max_operations),
        "{error:?}"
    );
}

#[test]
fn a_loop_inside_nested_calls_is_still_budgeted() {
    let error = search_err(
        "fn spin(n) { let i = 0; while true { i += n; } } fn search(q) { spin(1) }",
        Limits::default(),
    );
    assert!(
        matches!(error, ScriptError::BudgetExceeded { .. }),
        "{error:?}"
    );
}

#[test]
fn a_script_past_the_wall_clock_is_timed_out_and_its_thread_is_reclaimed() {
    let limits = Limits {
        max_operations: u64::MAX - 1,
        timeout: Duration::from_millis(200),
        ..Limits::default()
    };
    let fixture = try_load(
        r#"fn search(q) { if q == "ok" { return []; } loop { } }"#,
        &[],
        &[],
        limits,
    )
    .expect("compiles");
    let rt = runtime();

    let started = Instant::now();
    let error = rt.block_on(fixture.instance.search("spin")).unwrap_err();
    assert!(matches!(error, ScriptError::Timeout(_)), "{error:?}");
    assert!(started.elapsed() < Duration::from_secs(2));

    // Calls on one script are serialised, so this only returns if the
    // spinning call was actually terminated by `on_progress` — abandoning it
    // would leave it holding the script forever.
    let started = Instant::now();
    let tree = rt
        .block_on(fixture.instance.search("ok"))
        .expect("the script is usable again");
    assert!(common::titles(&tree).is_empty());
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "took {:?}",
        started.elapsed()
    );
}

#[test]
fn keystrokes_queued_behind_a_hung_script_do_not_each_hang_in_turn() {
    // One search per keystroke, all against a script that never returns. Calls
    // on one script are serialised, so without a shared clock the fifth would
    // start only after four full timeouts.
    let timeout = Duration::from_millis(300);
    let limits = Limits {
        max_operations: u64::MAX - 1,
        timeout,
        ..Limits::default()
    };
    let fixture = try_load(
        r#"fn search(q) { if q == "ok" { return []; } loop { } }"#,
        &[],
        &[],
        limits,
    )
    .expect("compiles");
    let rt = runtime();
    let started = Instant::now();
    let results = rt.block_on(async {
        let calls: Vec<_> = (0..5)
            .map(|i| {
                let instance = fixture.instance.clone();
                tokio::spawn(async move { instance.search(&format!("spin {i}")).await })
            })
            .collect();
        let mut results = Vec::new();
        for call in calls {
            results.push(call.await.unwrap());
        }
        results
    });
    assert!(
        results
            .iter()
            .all(|r| matches!(r, Err(ScriptError::Timeout(_)))),
        "{results:?}"
    );
    assert!(
        started.elapsed() < timeout * 3,
        "took {:?}",
        started.elapsed()
    );

    let started = Instant::now();
    rt.block_on(fixture.instance.search("ok"))
        .expect("usable again");
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[test]
fn a_hung_script_does_not_stall_the_callers_thread() {
    let limits = Limits {
        max_operations: u64::MAX - 1,
        timeout: Duration::from_millis(400),
        ..Limits::default()
    };
    let fixture = try_load("fn search(q) { loop { } }", &[], &[], limits).expect("compiles");
    // A current-thread runtime is the render loop's situation: if the script
    // ran on it, nothing else could.
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let ticks = rt.block_on(async {
        let ticker = tokio::spawn(async {
            let mut ticks = 0_u32;
            let mut interval = tokio::time::interval(Duration::from_millis(10));
            loop {
                interval.tick().await;
                ticks += 1;
                if ticks >= 10 {
                    return ticks;
                }
            }
        });
        let error = fixture.instance.search("").await.unwrap_err();
        assert!(matches!(error, ScriptError::Timeout(_)), "{error:?}");
        ticker.await.unwrap()
    });
    assert_eq!(ticks, 10, "the caller's loop kept running");
}

// ---------------------------------------------------------------- limits

#[test]
fn unbounded_recursion_hits_the_call_depth_limit() {
    let error = search_err(
        "fn down(n) { down(n + 1) } fn search(q) { down(0) }",
        Limits::default(),
    );
    assert!(
        matches!(
            error,
            ScriptError::LimitExceeded {
                limit: LimitKind::CallDepth,
                ..
            }
        ),
        "{error:?}"
    );
}

#[test]
fn a_string_that_doubles_forever_hits_the_string_size_limit() {
    let error = search_err(
        r#"fn search(q) { let s = "x"; loop { s += s; } }"#,
        Limits::default(),
    );
    assert!(
        matches!(
            error,
            ScriptError::LimitExceeded {
                limit: LimitKind::StringSize,
                ..
            }
        ),
        "{error:?}"
    );
}

#[test]
fn a_single_huge_string_allocation_is_refused_before_it_happens() {
    let started = Instant::now();
    let error = search_err(
        r#"fn search(q) { let s = ""; s.pad(4_000_000_000, 'x'); s }"#,
        Limits::default(),
    );
    assert!(
        matches!(
            error,
            ScriptError::LimitExceeded {
                limit: LimitKind::StringSize,
                ..
            }
        ),
        "{error:?}"
    );
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[test]
fn a_huge_array_is_refused_by_the_array_size_limit() {
    for source in [
        "fn search(q) { let a = []; a.pad(4_000_000_000, 0); a }",
        "fn search(q) { let a = []; loop { a.push(1); } }",
        "fn search(q) { let a = [1]; loop { a += a; } }",
    ] {
        let error = search_err(source, Limits::default());
        assert!(
            matches!(
                error,
                ScriptError::LimitExceeded {
                    limit: LimitKind::ArraySize,
                    ..
                }
            ),
            "{source}: {error:?}"
        );
    }
}

#[test]
fn a_huge_map_is_refused_by_the_map_size_limit() {
    let limits = Limits {
        max_map_size: 100,
        ..Limits::default()
    };
    let error = search_err(
        "fn search(q) { let m = #{}; let i = 0; while i < 1000 { m[`k${i}`] = i; i += 1; } m.len() }",
        limits,
    );
    assert!(
        matches!(
            error,
            ScriptError::LimitExceeded {
                limit: LimitKind::MapSize,
                ..
            }
        ),
        "{error:?}"
    );

    // Rhai checks a map's size when it is next used as a value (here, by
    // `len`), not on every indexed insert. A loop that only ever
    // inserts is therefore stopped by the operation budget instead — which
    // bounds its memory just the same.
    let error = search_err(
        "fn search(q) { let m = #{}; let i = 0; loop { m[`k${i}`] = i; i += 1; } }",
        limits,
    );
    assert!(
        matches!(error, ScriptError::BudgetExceeded { .. }),
        "{error:?}"
    );
}

#[test]
fn a_deeply_nested_expression_is_rejected_at_compile_time() {
    let depth = Limits::DEFAULT.max_function_expr_depth * 4;
    let source = format!(
        "fn search(q) {{ let x = {}1{}; [] }}",
        "(".repeat(depth),
        ")".repeat(depth)
    );
    let error = try_load(&source, &[], &[], Limits::default())
        .err()
        .expect("rejected");
    assert!(
        matches!(
            error,
            ScriptError::LimitExceeded {
                limit: LimitKind::ExpressionDepth,
                ..
            }
        ),
        "{error:?}"
    );
    let shallow = "fn search(q) { let x = ((((1)))); [] }";
    assert!(try_load(shallow, &[], &[], Limits::default()).is_ok());
}

// ---------------------------------------------------------------- escape hatches

#[test]
fn import_resolves_nothing_not_even_a_file_next_to_the_script() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("helper.rhai"), "export const X = 1;").unwrap();
    let helper = dir.path().join("helper.rhai");
    for source in [
        "import \"helper\" as h; fn search(q) { [] }".to_owned(),
        format!(
            "import \"{}\" as h; fn search(q) {{ [] }}",
            helper.display()
        ),
        "import \"/etc/passwd\" as h; fn search(q) { [] }".to_owned(),
        "fn search(q) { import \"helper\" as h; [] }".to_owned(),
    ] {
        let error = match try_load(&source, &[], &[], Limits::default()) {
            Err(error) => error,
            Ok(fixture) => runtime().block_on(fixture.instance.search("")).unwrap_err(),
        };
        assert!(
            matches!(
                error,
                ScriptError::LimitExceeded {
                    limit: LimitKind::Modules,
                    ..
                } | ScriptError::Undefined(_)
            ),
            "{source}: {error:?}"
        );
    }
}

#[test]
fn eval_is_not_part_of_the_language() {
    let error = load_err(r#"fn search(q) { eval("40 + 2") }"#, &[], &[]);
    assert!(matches!(error, ScriptError::Compile(_)), "{error:?}");
}

#[test]
fn nothing_reaches_the_filesystem_the_environment_processes_or_the_scheduler() {
    // Names a script author might reach for, from Rhai's own optional
    // packages and from habit. None of them exists.
    for call in [
        r#"open_file("/etc/passwd")"#,
        r#"read_file("/etc/passwd")"#,
        r#"fs::read("/etc/passwd")"#,
        r#"env("HOME")"#,
        r#"getenv("HOME")"#,
        r#"system("id")"#,
        r#"exec("id")"#,
        r#"command("id")"#,
        "sleep(10)",
        "exit()",
        "timestamp()",
        r#"http_get("https://example.invalid")"#,
        r#"process::run("id", [], 100)"#,
    ] {
        let source = format!("fn search(q) {{ {call}; [] }}");
        let error = match try_load(&source, &[], &[], Limits::default()) {
            Err(error) => error,
            Ok(fixture) => runtime().block_on(fixture.instance.search("")).unwrap_err(),
        };
        assert!(
            matches!(error, ScriptError::Undefined(_)),
            "{call}: {error:?}"
        );
    }
}

#[test]
fn print_and_debug_are_harmless() {
    let fixture = load(r#"fn search(q) { print("hello"); debug(q); [] }"#, &[]);
    runtime()
        .block_on(fixture.instance.search("x"))
        .expect("runs");
}

// ---------------------------------------------------------------- capabilities

const COPIES: &str = r#"fn search(q) { clipboard::copy(q); [] }"#;

#[test]
fn a_granted_capability_is_callable() {
    // The positive control for the tests below.
    let fixture = load(COPIES, &["clipboard.write"]);
    runtime().block_on(fixture.instance.search("hi")).unwrap();
    assert_eq!(fixture.host.clipboard().as_deref(), Some("hi"));
}

#[test]
fn an_undeclared_capability_is_absent_not_refused() {
    let error = load_err(COPIES, &[], &[]);
    assert!(
        matches!(&error, ScriptError::Undefined(name) if name == "clipboard"),
        "{error:?}"
    );
}

#[test]
fn a_declared_but_ungranted_capability_is_absent_too() {
    let error = load_err(COPIES, &["clipboard.write"], &[]);
    assert!(matches!(error, ScriptError::Undefined(_)), "{error:?}");
}

#[test]
fn one_capability_in_an_area_does_not_bring_its_neighbours() {
    // Granted `clipboard.write`, so the module exists; `read` is still absent.
    let fixture = load(
        "fn search(q) { let c = clipboard::read(); [] }",
        &["clipboard.write"],
    );
    let error = runtime().block_on(fixture.instance.search("")).unwrap_err();
    assert!(matches!(error, ScriptError::Undefined(_)), "{error:?}");
}

#[test]
fn a_revoked_grant_is_absent_from_the_next_instance() {
    let fixture = load(COPIES, &["clipboard.write"]);
    let mut registry = fixture.registry.clone();
    let id = fixture.instance.manifest().id.clone();
    assert!(registry.revoke(&id, &Capability::CLIPBOARD_WRITE));
    let rebuilt = compass_script::ScriptInstance::compile(
        fixture.instance.manifest().clone(),
        COPIES,
        &registry,
        fixture.host.clone(),
        Limits::default(),
    );
    assert!(
        matches!(rebuilt, Err(ScriptError::Undefined(_))),
        "{rebuilt:?}"
    );
}

#[test]
fn an_action_needing_an_ungranted_capability_is_refused_with_its_name() {
    let fixture = load(
        r#"fn search(q) { [#{ title: "x", actions: [#{ title: "Copy", copy: "secret" }] }] }"#,
        &[],
    );
    let error = runtime().block_on(fixture.instance.search("")).unwrap_err();
    let ScriptError::CapabilityDenied(denial) = error else {
        panic!("{error:?}");
    };
    assert_eq!(denial.capability, Capability::CLIPBOARD_WRITE);
    assert_eq!(denial.reason, DenialReason::NotDeclared);
    assert!(fixture.host.calls().is_empty());
}

#[test]
fn a_programs_timeout_never_outlasts_the_call_it_is_run_from() {
    let fixture = try_load(
        r#"fn search(q) { process::run("slow", [q], 60000); [] }"#,
        &["process.execute"],
        &["process.execute"],
        Limits {
            timeout: Duration::from_millis(300),
            ..Limits::default()
        },
    )
    .expect("compiles");
    runtime().block_on(fixture.instance.search("x")).unwrap();
    let calls = fixture.host.calls();
    let [HostCall::Run { argv, timeout }] = &calls[..] else {
        panic!("{calls:?}");
    };
    assert_eq!(argv, &["slow", "x"]);
    assert!(*timeout <= Duration::from_millis(300), "{timeout:?}");
}

#[test]
fn a_program_reads_as_missing_timed_out_or_exited() {
    let fixture = load(
        r#"
        fn search(q) {
            let missing = process::run("nope", [], 100);
            let slow = process::run("slow", [], 100);
            let done = process::run("done", ["a b"], 100);
            [
                #{ title: type_of(missing) },
                #{ title: slow.timed_out, subtitle: type_of(slow.code) },
                #{ title: done.code, subtitle: `${done.stdout}|${done.stderr}|${done.timed_out}` },
            ]
        }
        "#,
        &["process.execute"],
    );
    fixture
        .host
        .set_program(&["slow"], ProcessOutcome::TimedOut);
    fixture.host.set_program(
        &["done", "a b"],
        ProcessOutcome::Exited {
            code: Some(3),
            stdout: "out".to_owned(),
            stderr: "err".to_owned(),
        },
    );
    let tree = runtime().block_on(fixture.instance.search("")).unwrap();
    assert_eq!(common::titles(&tree), ["()", "true", "3"]);
    let compass_extension_api::View::List(list) = tree.root() else {
        panic!("not a list");
    };
    let subtitles: Vec<_> = list.sections[0]
        .items
        .iter()
        .map(|i| i.subtitle.as_deref())
        .collect();
    assert_eq!(subtitles, [None, Some("()"), Some("out|err|false")]);
}

#[test]
fn a_program_needs_text_arguments_and_a_positive_timeout() {
    for (call, says) in [
        (r#"process::run("x", [1], 100)"#, "array of text"),
        (r#"process::run("x", [], 0)"#, "at least 1 millisecond"),
    ] {
        let source = format!("fn search(q) {{ {call}; [] }}");
        let fixture = load(&source, &["process.execute"]);
        let error = runtime().block_on(fixture.instance.search("")).unwrap_err();
        assert!(error.to_string().contains(says), "{call}: {error}");
        assert!(fixture.host.calls().is_empty(), "{call}");
    }
}

#[test]
fn host_functions_never_run_at_compile_time() {
    // Rhai folds calls to non-volatile functions with constant arguments while
    // optimising. A host function folded that way would run on load, before
    // anyone asked the script for anything.
    let fixture = load(
        r#"const X = clipboard::copy("at compile time"); fn search(q) { clipboard::copy("at search"); [] }"#,
        &["clipboard.write"],
    );
    assert!(
        fixture.host.calls().is_empty(),
        "{:?}",
        fixture.host.calls()
    );
}

#[test]
fn a_script_error_is_a_value_not_a_panic() {
    let error = search_err(r#"fn search(q) { throw "nope" }"#, Limits::default());
    assert!(matches!(error, ScriptError::Runtime(_)), "{error:?}");
    let error = search_err("fn search(q) { 1 / 0 }", Limits::default());
    assert!(matches!(error, ScriptError::Runtime(_)), "{error:?}");
}

#[test]
fn a_script_without_search_does_not_load() {
    let error = load_err("fn other(q) { [] }", &[], &[]);
    assert!(
        matches!(error, ScriptError::MissingEntryPoint(_)),
        "{error:?}"
    );
}

#[test]
fn try_catch_cannot_swallow_a_limit() {
    type Check = fn(&ScriptError) -> bool;
    let cases: [(&str, Check); 3] = [
        ("fn search(q) { try { loop { } } catch { } [] }", |e| {
            matches!(e, ScriptError::BudgetExceeded { .. })
        }),
        (
            "fn down(n) { down(n + 1) } fn search(q) { try { down(0) } catch { } [] }",
            |e| {
                matches!(
                    e,
                    ScriptError::LimitExceeded {
                        limit: LimitKind::CallDepth,
                        ..
                    }
                )
            },
        ),
        (
            r#"fn search(q) { try { let s = "x"; loop { s += s; } } catch { } [] }"#,
            |e| {
                matches!(
                    e,
                    ScriptError::LimitExceeded {
                        limit: LimitKind::StringSize,
                        ..
                    }
                )
            },
        ),
    ];
    for (source, check) in cases {
        let error = search_err(source, Limits::default());
        assert!(check(&error), "{source}: {error:?}");
    }
}

#[test]
fn an_action_is_held_to_the_same_budget_as_a_search() {
    let fixture = try_load(
        r#"fn spin() { loop { } } fn search(q) { [#{ title: "x", actions: [#{ title: "Spin", run: "spin" }] }] }"#,
        &[],
        &[],
        Limits {
            timeout: Duration::from_secs(60),
            ..Limits::default()
        },
    )
    .expect("compiles");
    let rt = runtime();
    let tree = rt.block_on(fixture.instance.search("")).unwrap();
    let response = rt.block_on(common::fire(&fixture, &tree, "Spin"));
    let compass_extension_api::ActionResponse::Failed {
        error: compass_extension_api::DispatchError::Failed { message, .. },
        ..
    } = response
    else {
        panic!("{response:?}");
    };
    assert!(message.contains("budget"), "{message}");
}
