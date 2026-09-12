//! Capability tests. These are the ones that matter: everything else in this crate is
//! shape, and this is the part that decides whether an extension can touch the machine.

mod common;

use common::*;
use compass_extension_api::*;

fn ext() -> ExtensionId {
    ExtensionId::new("com.example.demo")
}

#[test]
fn granted_capability_is_allowed() {
    let e = ext();
    let r = registry_with(
        &e,
        &[Capability::CLIPBOARD_READ],
        &[Capability::CLIPBOARD_READ],
    );
    let grant = r.check(&e, &Capability::CLIPBOARD_READ).expect("granted");
    assert_eq!(grant.extension(), &e);
    assert_eq!(grant.capability(), &Capability::CLIPBOARD_READ);
    assert!(r.holds(&e, &Capability::CLIPBOARD_READ));
}

#[test]
fn declared_but_ungranted_capability_is_denied() {
    let e = ext();
    let r = registry_with(
        &e,
        &[Capability::CLIPBOARD_READ, Capability::NETWORK_REQUEST],
        &[],
    );
    let denial = r.check(&e, &Capability::NETWORK_REQUEST).unwrap_err();
    assert_eq!(denial.reason, DenialReason::NotGranted);
    assert!(!r.holds(&e, &Capability::NETWORK_REQUEST));
}

#[test]
fn undeclared_capability_is_denied_and_cannot_be_granted() {
    let e = ext();
    let mut r = registry_with(&e, &[Capability::CLIPBOARD_READ], &[]);
    let denial = r.check(&e, &Capability::PROCESS_EXECUTE).unwrap_err();
    assert_eq!(denial.reason, DenialReason::NotDeclared);

    // A host cannot quietly hand out something the manifest never asked for.
    let refused = r.grant(&e, &Capability::PROCESS_EXECUTE).unwrap_err();
    assert_eq!(refused.reason, DenialReason::NotDeclared);
    assert!(!r.holds(&e, &Capability::PROCESS_EXECUTE));
}

#[test]
fn nothing_is_ambient_for_an_unregistered_extension() {
    let r = CapabilityRegistry::new();
    let stranger = ExtensionId::new("com.example.stranger");
    for cap in Capability::KNOWN {
        let denial = r.check(&stranger, cap).unwrap_err();
        assert_eq!(denial.reason, DenialReason::UnknownExtension);
        assert_eq!(&denial.capability, cap);
    }
}

#[test]
fn denial_names_the_missing_capability_in_data_and_in_prose() {
    let e = ext();
    let r = registry_with(&e, &[Capability::WINDOW_MANAGE], &[]);
    let denial = r.check(&e, &Capability::WINDOW_MANAGE).unwrap_err();

    // Inspectable as data...
    assert_eq!(denial.capability, Capability::WINDOW_MANAGE);
    assert_eq!(denial.capability.as_str(), "window.manage");
    assert_eq!(denial.extension, e);

    // ...and legible as a message the user can act on.
    let text = denial.to_string();
    assert!(text.contains("window.manage"), "{text}");
    assert!(text.contains("com.example.demo"), "{text}");
    assert!(text.contains("not been granted"), "{text}");

    // ...and it is an error, not a panic.
    let as_error: &dyn std::error::Error = &denial;
    assert!(as_error.to_string().contains("window.manage"));

    // ...and it survives being carried to another process.
    let json = serde_json::to_string(&denial).unwrap();
    assert!(json.contains("window.manage"), "{json}");
    let back: Denial = serde_json::from_str(&json).unwrap();
    assert_eq!(back, denial);
}

#[test]
fn dispatch_errors_carry_the_missing_capability_through() {
    let e = ext();
    let r = registry_with(&e, &[Capability::CLIPBOARD_WRITE], &[]);
    let error: DispatchError = r
        .check(&e, &Capability::CLIPBOARD_WRITE)
        .unwrap_err()
        .into();
    assert_eq!(
        error.missing_capability(),
        Some(&Capability::CLIPBOARD_WRITE)
    );
    assert!(error.to_string().contains("clipboard.write"));
    assert_eq!(
        DispatchError::UnknownAction {
            handler: HandlerId::new("x")
        }
        .missing_capability(),
        None
    );
}

#[test]
fn revocation_is_distinguishable_from_never_granted() {
    let e = ext();
    let mut r = registry_with(&e, &[Capability::OAUTH_TOKENS], &[Capability::OAUTH_TOKENS]);
    assert!(r.holds(&e, &Capability::OAUTH_TOKENS));

    assert!(r.revoke(&e, &Capability::OAUTH_TOKENS));
    let denial = r.check(&e, &Capability::OAUTH_TOKENS).unwrap_err();
    assert_eq!(denial.reason, DenialReason::Revoked);

    // Revoking twice is not an error, it is just a no-op.
    assert!(!r.revoke(&e, &Capability::OAUTH_TOKENS));

    // And a grant restores it.
    r.grant(&e, &Capability::OAUTH_TOKENS).unwrap();
    assert!(r.holds(&e, &Capability::OAUTH_TOKENS));
}

#[test]
fn an_unknown_capability_from_a_newer_extension_does_not_crash_an_older_host() {
    let e = ext();
    let future = Capability::new("timeline.rewind");
    assert!(!future.is_known());

    let mut r = CapabilityRegistry::new();
    // The manifest of an extension built against a newer host: two capabilities we know,
    // one we have never heard of.
    r.declare(
        &e,
        [
            Capability::CLIPBOARD_READ,
            future.clone(),
            Capability::STORAGE_READ,
        ],
    );

    // The declaration is kept verbatim, so upgrading the host is enough to make it work.
    let state = r.state(&e).unwrap();
    assert!(state.declared.contains(&future));
    assert_eq!(state.unknown_declarations(), vec![&future]);

    // The known ones still work.
    r.grant(&e, &Capability::CLIPBOARD_READ).unwrap();
    assert!(r.holds(&e, &Capability::CLIPBOARD_READ));

    // The unknown one can never be granted, and says why rather than panicking.
    let refused = r.grant(&e, &future).unwrap_err();
    assert_eq!(refused.reason, DenialReason::UnknownCapability);
    let denial = r.check(&e, &future).unwrap_err();
    assert_eq!(denial.reason, DenialReason::UnknownCapability);
    assert!(denial.to_string().contains("timeline.rewind"));

    // And a manifest that is entirely unknown is still merely useless, not fatal.
    let alien = ExtensionId::new("com.example.alien");
    r.declare(&alien, [Capability::new("x.y"), Capability::new("z")]);
    assert!(r.check(&alien, &Capability::new("x.y")).is_err());
    assert_eq!(r.state(&alien).unwrap().granted.len(), 0);
}

#[test]
fn a_registry_written_by_a_newer_host_still_deserialises() {
    // Exactly the shape a newer host would persist: an unknown capability, granted.
    let json = r#"{
        "extensions": {
            "com.example.demo": {
                "declared": ["clipboard.read", "timeline.rewind"],
                "granted": ["clipboard.read", "timeline.rewind"],
                "revoked": []
            }
        }
    }"#;
    let r: CapabilityRegistry = serde_json::from_str(json).expect("older host parses newer state");
    let e = ext();
    assert!(r.holds(&e, &Capability::CLIPBOARD_READ));
    // We honour the persisted grant rather than second-guessing it; what we must not do
    // is fail to load, and what a host must not do is act on a name it cannot interpret.
    assert!(r.check(&e, &Capability::new("timeline.rewind")).is_ok());
    assert!(!Capability::new("timeline.rewind").is_known());
}

#[test]
fn redeclaring_drops_grants_that_are_no_longer_asked_for() {
    let e = ext();
    let mut r = registry_with(
        &e,
        &[Capability::CLIPBOARD_READ, Capability::NETWORK_REQUEST],
        &[Capability::CLIPBOARD_READ, Capability::NETWORK_REQUEST],
    );
    // The extension updates and stops asking for the network.
    r.declare(&e, [Capability::CLIPBOARD_READ]);
    assert!(r.holds(&e, &Capability::CLIPBOARD_READ));
    let denial = r.check(&e, &Capability::NETWORK_REQUEST).unwrap_err();
    assert_eq!(denial.reason, DenialReason::NotDeclared);
}

#[test]
fn forgetting_an_extension_removes_every_grant() {
    let e = ext();
    let mut r = registry_with(
        &e,
        &[Capability::STORAGE_WRITE],
        &[Capability::STORAGE_WRITE],
    );
    r.forget(&e);
    assert_eq!(r.extensions().count(), 0);
    assert_eq!(
        r.check(&e, &Capability::STORAGE_WRITE).unwrap_err().reason,
        DenialReason::UnknownExtension
    );
}

#[test]
fn check_all_reports_every_missing_capability_not_just_the_first() {
    let e = ext();
    let r = registry_with(
        &e,
        &[
            Capability::CLIPBOARD_READ,
            Capability::NETWORK_REQUEST,
            Capability::WINDOW_READ,
        ],
        &[Capability::CLIPBOARD_READ],
    );
    let wanted = [
        Capability::CLIPBOARD_READ,
        Capability::NETWORK_REQUEST,
        Capability::WINDOW_READ,
    ];
    let denials = r.check_all(&e, wanted.iter()).unwrap_err();
    let missing: Vec<&str> = denials.iter().map(|d| d.capability.as_str()).collect();
    assert_eq!(missing, vec!["network.request", "window.read"]);

    r.check_all(&e, [&Capability::CLIPBOARD_READ])
        .expect("all held");
}

#[test]
fn every_known_capability_is_namespaced_and_unique() {
    let mut seen = std::collections::BTreeSet::new();
    for cap in Capability::KNOWN {
        assert!(seen.insert(cap.as_str()), "duplicate capability {cap}");
        assert!(
            cap.as_str().contains('.'),
            "capability {cap} is not namespaced"
        );
        assert!(cap.is_known());
        assert_eq!(
            &Capability::new(cap.as_str()),
            cap,
            "constant and parsed form disagree"
        );
    }
}
