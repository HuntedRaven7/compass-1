//! The hardened engine: what a script can see, and nothing else.
//!
//! Built from `Engine::new_raw()`, which has no functions at all, then given
//! an explicit list of pure packages. Deliberately left out is Rhai's
//! `LanguageCorePackage`, whose `sleep` would park a blocking thread where
//! `on_progress` cannot reach it; its one useful function, `parse_json`, is
//! re-registered here. `eval` is disabled as a symbol, `import` resolves
//! nothing (a dummy resolver *and* a module limit of zero), and every
//! `set_max_*` limit is set from [`Limits`].
//!
//! Host functions are registered per capability, and only for capabilities
//! the registry says are held. A capability that is not held has no function:
//! calling it is a compile error (`clipboard::copy` names a module that does
//! not exist) or an "unknown function", never an error returned by a stub.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use compass_extension_api::{Capability, CapabilityGrant, CapabilityRegistry, Denial, ExtensionId};
use rhai::packages::{
    ArithmeticPackage, BasicArrayPackage, BasicFnPackage, BasicIteratorPackage, BasicMapPackage,
    BasicMathPackage, BasicStringPackage, BitFieldPackage, LogicPackage, MoreStringPackage,
    Package,
};
use rhai::{
    Dynamic, Engine, EvalAltResult, FuncRegistration, INT, ImmutableString, Module,
    module_resolvers::DummyModuleResolver,
};

use crate::error::{TERMINATE_BUDGET, TERMINATE_TIMEOUT};
use crate::host::{ProcessOutcome, ScriptHost};
use crate::limits::Limits;

type RhaiResult<T> = Result<T, Box<EvalAltResult>>;

/// How often, in operations, `on_progress` reads the clock.
const CLOCK_CHECK_INTERVAL: u64 = 256;

/// The capabilities this tier binds to script functions.
pub(crate) const BOUND: &[Capability] = &[
    Capability::CLIPBOARD_WRITE,
    Capability::CLIPBOARD_READ,
    Capability::CLIPBOARD_PASTE,
    Capability::APPLICATION_OPEN,
    Capability::STORAGE_READ,
    Capability::STORAGE_WRITE,
    Capability::NOTIFICATION_SEND,
    Capability::PROCESS_EXECUTE,
];

/// The wall-clock deadline of the call currently running, shared with
/// `on_progress`. Calls on one engine are serialised, so one slot is enough.
#[derive(Debug)]
pub(crate) struct Deadline {
    origin: Instant,
    /// Nanoseconds after `origin`; zero when no call is running.
    at: AtomicU64,
}

impl Deadline {
    fn new() -> Self {
        Self {
            origin: Instant::now(),
            at: AtomicU64::new(0),
        }
    }

    pub(crate) fn arm(&self, timeout: Duration) {
        let at = (self.origin.elapsed() + timeout).as_nanos();
        self.at.store(
            u64::try_from(at).unwrap_or(u64::MAX).max(1),
            Ordering::Release,
        );
    }

    pub(crate) fn disarm(&self) {
        self.at.store(0, Ordering::Release);
    }

    fn passed(&self) -> bool {
        let at = self.at.load(Ordering::Acquire);
        at != 0 && self.origin.elapsed().as_nanos() >= u128::from(at)
    }

    /// How long the running call has left; `None` when no call is running.
    fn remaining(&self) -> Option<Duration> {
        let at = self.at.load(Ordering::Acquire);
        (at != 0).then(|| Duration::from_nanos(at).saturating_sub(self.origin.elapsed()))
    }
}

/// The outcome of checking each bound capability once, when the engine was
/// built. Kept so views can be checked against the same answer the engine's
/// function table was built from.
#[derive(Debug, Clone)]
pub(crate) struct Grants {
    checked: BTreeMap<Capability, Result<CapabilityGrant, Denial>>,
    extension: ExtensionId,
}

impl Grants {
    pub(crate) fn check(registry: &CapabilityRegistry, extension: &ExtensionId) -> Self {
        Self {
            checked: BOUND
                .iter()
                .map(|cap| (cap.clone(), registry.check(extension, cap)))
                .collect(),
            extension: extension.clone(),
        }
    }

    pub(crate) fn get(&self, cap: &Capability) -> Result<CapabilityGrant, Denial> {
        self.checked.get(cap).cloned().unwrap_or_else(|| {
            Err(Denial {
                extension: self.extension.clone(),
                capability: cap.clone(),
                reason: compass_extension_api::DenialReason::UnknownCapability,
            })
        })
    }

    fn held(&self, cap: &Capability) -> Option<CapabilityGrant> {
        self.get(cap).ok()
    }
}

/// Builds the engine for one script.
pub(crate) fn build(
    limits: &Limits,
    deadline: Arc<Deadline>,
    grants: &Grants,
    host: &Arc<dyn ScriptHost>,
) -> Engine {
    let mut engine = Engine::new_raw();

    ArithmeticPackage::new().register_into_engine(&mut engine);
    LogicPackage::new().register_into_engine(&mut engine);
    BitFieldPackage::new().register_into_engine(&mut engine);
    BasicStringPackage::new().register_into_engine(&mut engine);
    MoreStringPackage::new().register_into_engine(&mut engine);
    BasicIteratorPackage::new().register_into_engine(&mut engine);
    BasicFnPackage::new().register_into_engine(&mut engine);
    BasicMathPackage::new().register_into_engine(&mut engine);
    BasicArrayPackage::new().register_into_engine(&mut engine);
    BasicMapPackage::new().register_into_engine(&mut engine);

    engine.set_module_resolver(DummyModuleResolver::new());
    engine.set_max_modules(0);
    engine.disable_symbol("eval");
    engine.set_strict_variables(true);

    engine.set_max_operations(limits.max_operations.saturating_add(1));
    engine.set_max_call_levels(limits.max_call_levels);
    engine.set_max_expr_depths(limits.max_expr_depth, limits.max_function_expr_depth);
    engine.set_max_string_size(limits.max_string_size);
    engine.set_max_array_size(limits.max_array_size);
    engine.set_max_map_size(limits.max_map_size);
    engine.set_max_variables(limits.max_variables);

    let budget = limits.max_operations;
    let progress = deadline.clone();
    engine.on_progress(move |ops| {
        if ops > budget {
            return Some(TERMINATE_BUDGET.into());
        }
        if ops % CLOCK_CHECK_INTERVAL == 0 && progress.passed() {
            return Some(TERMINATE_TIMEOUT.into());
        }
        None
    });

    engine.on_print(|text| tracing::info!(target: "compass_script", "{text}"));
    engine.on_debug(|text, _, pos| tracing::debug!(target: "compass_script", %pos, "{text}"));

    register_pure(&mut engine);
    register_capabilities(&mut engine, grants, host, &deadline);
    engine
}

fn volatile(name: &str) -> FuncRegistration {
    FuncRegistration::new(name).with_volatility(true)
}

fn fail<T>(message: impl Into<String>) -> RhaiResult<T> {
    Err(message.into().into())
}

/// Functions every script has: logging, JSON, time, randomness and URL
/// encoding. None of
/// them reads anything the user has not typed into the launcher.
fn register_pure(engine: &mut Engine) {
    volatile("log").register_into_engine(engine, |message: ImmutableString| {
        tracing::info!(target: "compass_script", "{message}");
    });
    volatile("parse_json").register_into_engine(
        engine,
        |text: ImmutableString| -> RhaiResult<Dynamic> {
            let value: serde_json::Value =
                serde_json::from_str(&text).map_err(|error| format!("invalid JSON: {error}"))?;
            rhai::serde::to_dynamic(value)
        },
    );

    let mut time = Module::new();
    volatile("now").set_into_module(&mut time, || -> INT {
        time::OffsetDateTime::now_utc().unix_timestamp()
    });
    volatile("now_ms").set_into_module(&mut time, || -> INT {
        let now = time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000;
        INT::try_from(now).unwrap_or(INT::MAX)
    });
    volatile("format").set_into_module(&mut time, |seconds: INT| -> RhaiResult<String> {
        let at = utc(seconds)?;
        at.format(&time::format_description::well_known::Rfc3339)
            .map_err(|error| error.to_string().into())
    });
    volatile("format").set_into_module(
        &mut time,
        |seconds: INT, pattern: ImmutableString| -> RhaiResult<String> {
            let at = utc(seconds)?;
            let description = time::format_description::parse_borrowed::<2>(&pattern)
                .map_err(|error| format!("invalid format: {error}"))?;
            at.format(&description)
                .map_err(|error| error.to_string().into())
        },
    );
    volatile("parse").set_into_module(&mut time, |text: ImmutableString| -> RhaiResult<INT> {
        time::OffsetDateTime::parse(text.trim(), &time::format_description::well_known::Rfc3339)
            .map(time::OffsetDateTime::unix_timestamp)
            .map_err(|error| format!("not an RFC 3339 date: {error}").into())
    });
    volatile("parts").set_into_module(&mut time, |seconds: INT| -> RhaiResult<rhai::Map> {
        let at = utc(seconds)?;
        let mut parts = rhai::Map::new();
        parts.insert("year".into(), INT::from(at.year()).into());
        parts.insert("month".into(), INT::from(u8::from(at.month())).into());
        parts.insert("day".into(), INT::from(at.day()).into());
        parts.insert("hour".into(), INT::from(at.hour()).into());
        parts.insert("minute".into(), INT::from(at.minute()).into());
        parts.insert("second".into(), INT::from(at.second()).into());
        parts.insert("weekday".into(), at.weekday().to_string().into());
        parts.insert("ordinal".into(), INT::from(at.ordinal()).into());
        Ok(parts)
    });
    engine.register_static_module("time", time.into());

    let mut random = Module::new();
    volatile("uuid").set_into_module(&mut random, || -> RhaiResult<String> {
        let mut bytes = [0_u8; 16];
        getrandom::fill(&mut bytes).map_err(|error| error.to_string())?;
        Ok(uuid::Builder::from_random_bytes(bytes)
            .into_uuid()
            .hyphenated()
            .to_string())
    });
    volatile("int").set_into_module(&mut random, |low: INT, high: INT| -> RhaiResult<INT> {
        if high <= low {
            return fail("random::int(low, high) needs low < high");
        }
        let span = high.abs_diff(low);
        let mut bytes = [0_u8; 8];
        getrandom::fill(&mut bytes).map_err(|error| error.to_string())?;
        let offset = u64::from_le_bytes(bytes) % span;
        Ok(low.wrapping_add_unsigned(offset))
    });
    volatile("float").set_into_module(&mut random, || -> RhaiResult<rhai::FLOAT> {
        let mut bytes = [0_u8; 8];
        getrandom::fill(&mut bytes).map_err(|error| error.to_string())?;
        // 53 random bits: every representable value in [0, 1) at that spacing.
        #[allow(clippy::cast_precision_loss)]
        let value = (u64::from_le_bytes(bytes) >> 11) as rhai::FLOAT / (1_u64 << 53) as rhai::FLOAT;
        Ok(value)
    });
    engine.register_static_module("random", random.into());

    let mut url = Module::new();
    volatile("encode").set_into_module(&mut url, |text: ImmutableString| -> String {
        percent_encoding::utf8_percent_encode(&text, percent_encoding::NON_ALPHANUMERIC).to_string()
    });
    engine.register_static_module("url", url.into());
}

fn utc(seconds: INT) -> RhaiResult<time::OffsetDateTime> {
    time::OffsetDateTime::from_unix_timestamp(seconds)
        .map_err(|error| format!("timestamp out of range: {error}").into())
}

fn host_error(error: crate::host::HostError) -> Box<EvalAltResult> {
    error.0.into()
}

/// Registers one module per capability area, holding only the functions whose
/// capability is granted. An area with nothing granted gets no module at all.
fn register_capabilities(
    engine: &mut Engine,
    grants: &Grants,
    host: &Arc<dyn ScriptHost>,
    deadline: &Arc<Deadline>,
) {
    let mut clipboard = Module::new();
    if let Some(grant) = grants.held(&Capability::CLIPBOARD_WRITE) {
        let host = host.clone();
        volatile("copy").set_into_module(&mut clipboard, move |text: ImmutableString| {
            host.clipboard_write(grant.clone(), &text)
                .map_err(host_error)
        });
    }
    if let Some(grant) = grants.held(&Capability::CLIPBOARD_READ) {
        let host = host.clone();
        volatile("read").set_into_module(&mut clipboard, move || -> RhaiResult<Dynamic> {
            Ok(host
                .clipboard_read(grant.clone())
                .map_err(host_error)?
                .map_or(Dynamic::UNIT, Dynamic::from))
        });
    }
    if let Some(grant) = grants.held(&Capability::CLIPBOARD_PASTE) {
        let host = host.clone();
        volatile("paste").set_into_module(&mut clipboard, move |text: ImmutableString| {
            host.clipboard_paste(grant.clone(), &text)
                .map_err(host_error)
        });
    }
    register_if_any(engine, "clipboard", clipboard);

    let mut application = Module::new();
    if let Some(grant) = grants.held(&Capability::APPLICATION_OPEN) {
        let host = host.clone();
        volatile("open").set_into_module(&mut application, move |target: ImmutableString| {
            host.open(grant.clone(), &target).map_err(host_error)
        });
    }
    register_if_any(engine, "application", application);

    let mut storage = Module::new();
    if let Some(grant) = grants.held(&Capability::STORAGE_READ) {
        let get_host = host.clone();
        let get_grant = grant.clone();
        volatile("get").set_into_module(
            &mut storage,
            move |key: ImmutableString| -> RhaiResult<Dynamic> {
                match get_host
                    .storage_get(get_grant.clone(), &key)
                    .map_err(host_error)?
                {
                    None => Ok(Dynamic::UNIT),
                    Some(text) => {
                        let value: serde_json::Value = serde_json::from_str(&text)
                            .map_err(|error| format!("stored value is not JSON: {error}"))?;
                        rhai::serde::to_dynamic(value)
                    }
                }
            },
        );
        let host = host.clone();
        volatile("keys").set_into_module(&mut storage, move || -> RhaiResult<rhai::Array> {
            Ok(host
                .storage_keys(grant.clone())
                .map_err(host_error)?
                .into_iter()
                .map(Dynamic::from)
                .collect())
        });
    }
    if let Some(grant) = grants.held(&Capability::STORAGE_WRITE) {
        let set_host = host.clone();
        let set_grant = grant.clone();
        volatile("set").set_into_module(
            &mut storage,
            move |key: ImmutableString, value: Dynamic| -> RhaiResult<()> {
                let json: serde_json::Value = rhai::serde::from_dynamic(&value)?;
                set_host
                    .storage_set(set_grant.clone(), &key, &json.to_string())
                    .map_err(host_error)
            },
        );
        let host = host.clone();
        volatile("remove").set_into_module(&mut storage, move |key: ImmutableString| {
            host.storage_remove(grant.clone(), &key).map_err(host_error)
        });
    }
    register_if_any(engine, "storage", storage);

    let mut notification = Module::new();
    if let Some(grant) = grants.held(&Capability::NOTIFICATION_SEND) {
        let host = host.clone();
        volatile("send").set_into_module(
            &mut notification,
            move |title: ImmutableString, body: ImmutableString| {
                host.notify(grant.clone(), &title, &body)
                    .map_err(host_error)
            },
        );
    }
    register_if_any(engine, "notification", notification);

    let mut process = Module::new();
    if let Some(grant) = grants.held(&Capability::PROCESS_EXECUTE) {
        let host = host.clone();
        let deadline = deadline.clone();
        volatile("run").set_into_module(
            &mut process,
            move |program: ImmutableString, args: rhai::Array, timeout_ms: INT| {
                run_process(&*host, &grant, &deadline, &program, args, timeout_ms)
            },
        );
    }
    register_if_any(engine, "process", process);
}

/// `process::run(program, args, timeout_ms)`: `()` when the program is not
/// installed, otherwise `#{ code, stdout, stderr, timed_out }`.
///
/// The timeout is the script's to choose but never outlasts the call it is
/// made from: `on_progress` cannot interrupt a host function, so a program
/// allowed to run past the call's deadline would hold the script's thread
/// past it too.
fn run_process(
    host: &dyn ScriptHost,
    grant: &CapabilityGrant,
    deadline: &Deadline,
    program: &str,
    args: rhai::Array,
    timeout_ms: INT,
) -> RhaiResult<Dynamic> {
    let args = args
        .into_iter()
        .map(|arg| arg.into_immutable_string().map(|arg| arg.to_string()))
        .collect::<Result<Vec<_>, _>>()
        .or_else(|_| fail("process::run takes its arguments as an array of text"))?;
    let Some(asked) = u64::try_from(timeout_ms).ok().filter(|&ms| ms > 0) else {
        return fail("process::run needs a timeout of at least 1 millisecond");
    };
    let asked = Duration::from_millis(asked);
    let timeout = deadline.remaining().map_or(asked, |left| asked.min(left));
    let outcome = if timeout.is_zero() {
        Some(ProcessOutcome::TimedOut)
    } else {
        host.run(grant.clone(), program, &args, timeout)
            .map_err(host_error)?
    };
    let Some(outcome) = outcome else {
        return Ok(Dynamic::UNIT);
    };
    let mut map = rhai::Map::new();
    let (code, stdout, stderr, timed_out) = match outcome {
        ProcessOutcome::Exited {
            code,
            stdout,
            stderr,
        } => (code, stdout, stderr, false),
        ProcessOutcome::TimedOut => (None, String::new(), String::new(), true),
    };
    map.insert(
        "code".into(),
        code.map_or(Dynamic::UNIT, |code| Dynamic::from(INT::from(code))),
    );
    map.insert("stdout".into(), stdout.into());
    map.insert("stderr".into(), stderr.into());
    map.insert("timed_out".into(), timed_out.into());
    Ok(map.into())
}

fn register_if_any(engine: &mut Engine, name: &str, module: Module) {
    if !module.is_empty() {
        engine.register_static_module(name, module.into());
    }
}

/// A fresh deadline slot for [`build`].
pub(crate) fn deadline() -> Arc<Deadline> {
    Arc::new(Deadline::new())
}
