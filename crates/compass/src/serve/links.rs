//! Consent for commands a link from outside would run.
//!
//! A link handed to Compass by another program (`compass deeplink`, via the
//! URL-handler desktop entry) can reach an installed extension command or a
//! script command. The launcher asks before running one and keeps "Always
//! Allow" per command id, separate from the host-program grants so revoking
//! one set cannot touch the other.

use compass_ipc::{ErrorKind, ProtocolError, Request, Response};

/// The only request this module serves: read or remember a link's grant.
pub(super) async fn handle(request: Request) -> Response {
    let Request::LinkCommandGrant { id, remember } = request else {
        return Response::Error(ProtocolError::new(
            ErrorKind::BadRequest,
            "not a link request",
        ));
    };
    let grants = crate::host_commands::Grants::link_grants();
    let outcome = tokio::task::spawn_blocking(move || {
        if remember {
            grants.allow_link(&id).map(|()| true)
        } else {
            Ok(grants.link_allows(&id))
        }
    })
    .await;
    match outcome {
        Ok(Ok(allowed)) => Response::LinkCommandGrant { allowed },
        Ok(Err(err)) => Response::Error(ProtocolError::new(
            ErrorKind::Internal,
            format!("Compass could not remember the link permission: {err}"),
        )),
        Err(err) => Response::Error(ProtocolError::new(
            ErrorKind::Internal,
            format!("reading link permissions failed: {err}"),
        )),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_remembered_command_is_allowed_without_asking_again() {
        let dir = tempfile::tempdir().unwrap();
        let grants = crate::host_commands::Grants::new(Some(dir.path().join("grants.json")));
        assert!(!grants.link_allows("@author/ext:cmd"));
        grants.allow_link("@author/ext:cmd").unwrap();
        assert!(grants.link_allows("@author/ext:cmd"));
        let listed = grants.link_entries();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, crate::host_commands::LINK_GRANTS_ID);
        assert_eq!(listed[0].capabilities, ["link.run:@author/ext:cmd"]);
        assert!(!grants.link_allows("@author/ext:other"));
    }
}
