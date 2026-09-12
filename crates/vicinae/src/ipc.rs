//! Talking to a running engine, and explaining it when there isn't one.
//!
//! The interesting part of this module is [`not_running`]. Every one of these
//! commands is normally typed by a person who has just bound it to a key, and
//! the most common outcome on a fresh machine is that nothing is listening yet.
//! `Connection refused (os error 111)` tells that person nothing they can act
//! on; it does not even name the path that was refused.

use anyhow::{Context, Result, bail};
use compass_ipc::{Client, Request, Response, SocketPath};

/// Sends one request to the engine and returns its response.
///
/// Connection failures are translated into [`not_running`]; a
/// [`Response::Error`] from the engine is returned as an `Err` carrying the
/// engine's own message.
pub async fn send(socket: &SocketPath, request: Request) -> Result<Response> {
    let mut client = match Client::connect(socket.as_path()).await {
        Ok(client) => client,
        Err(compass_ipc::Error::Io(err)) => return Err(not_running(socket, &err)),
        Err(err) => {
            return Err(err).with_context(|| format!("connecting to the engine at {socket}"));
        }
    };

    match client.request(request).await {
        Ok(Response::Error(err)) => bail!("the engine refused the request: {err}"),
        Ok(response) => Ok(response),
        Err(err) => Err(err).with_context(|| format!("talking to the engine at {socket}")),
    }
}

/// The message a person gets when no engine is listening.
///
/// Names the path, says what to do, and mentions the two ways the path itself
/// can be wrong — a mismatched `XDG_RUNTIME_DIR` (a sandbox, an `ssh` session,
/// a different user) and an explicit `--socket` on one side but not the other.
#[must_use]
pub fn not_running(socket: &SocketPath, cause: &std::io::Error) -> anyhow::Error {
    let fallback_note = if socket.is_fallback() {
        "\n  - XDG_RUNTIME_DIR is unset here, so this is the /tmp fallback path; an engine \
         started inside a normal desktop session is listening somewhere else"
    } else {
        ""
    };

    anyhow::anyhow!(
        "no Compass engine is listening on {socket}\n\
         \n\
         \x20 - start the engine, then run this again\n\
         \x20 - if it is running, it may be using a different socket: pass --socket <PATH> \
         (or set COMPASS_SOCKET) so both sides agree\n\
         \x20 - `vicinae doctor` prints the socket path it resolved and everything else it \
         can see{fallback_note}\n\
         \n\
         (underlying error: {cause})"
    )
}

/// Sends a request whose only successful answer is [`Response::Ack`].
pub async fn send_ack(socket: &SocketPath, request: Request) -> Result<()> {
    let described = describe(&request);
    match send(socket, request).await? {
        Response::Ack => Ok(()),
        other => bail!("the engine answered {described} with an unexpected {other:?}"),
    }
}

/// Pings the engine and renders the one-line liveness report.
pub async fn ping(socket: &SocketPath) -> Result<String> {
    match send(socket, Request::Ping).await? {
        Response::Pong {
            protocol_version,
            pid,
        } => Ok(format!(
            "engine alive on {socket}: protocol v{protocol_version}, pid {pid}"
        )),
        other => bail!("the engine answered Ping with an unexpected {other:?}"),
    }
}

/// Runs a query against the engine and returns the ranked hits.
pub async fn query(socket: &SocketPath, text: &str) -> Result<Vec<compass_ipc::QueryHit>> {
    match send(
        socket,
        Request::Query {
            text: text.to_owned(),
        },
    )
    .await?
    {
        Response::QueryResults { hits } => Ok(hits),
        other => bail!("the engine answered Query with an unexpected {other:?}"),
    }
}

fn describe(request: &Request) -> &'static str {
    match request {
        Request::Ping => "Ping",
        Request::Toggle => "Toggle",
        Request::Show => "Show",
        Request::Hide => "Hide",
        Request::Query { .. } => "Query",
        Request::Doctor => "Doctor",
        Request::Shutdown => "Shutdown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::ErrorKind;

    #[test]
    fn the_no_daemon_message_is_actionable_not_an_io_error() {
        let socket = SocketPath::exact("/run/user/1000/vicinae/ipc.sock");
        let err = not_running(&socket, &std::io::Error::from(ErrorKind::ConnectionRefused));
        let text = err.to_string();

        assert!(text.contains("/run/user/1000/vicinae/ipc.sock"));
        assert!(text.contains("start the engine"));
        assert!(text.contains("--socket"));
        assert!(text.contains("vicinae doctor"));
        assert!(!text.starts_with("Connection refused"));
    }

    #[test]
    fn the_fallback_socket_gets_its_own_warning() {
        let real = SocketPath::from_env();
        let err = not_running(&real, &std::io::Error::from(ErrorKind::NotFound));
        let text = err.to_string();
        assert_eq!(
            text.contains("XDG_RUNTIME_DIR is unset here"),
            real.is_fallback()
        );
    }

    #[tokio::test]
    async fn connecting_to_a_missing_socket_produces_the_friendly_message() {
        let dir = tempfile::tempdir().expect("tempdir");
        let socket = SocketPath::exact(dir.path().join("absent.sock"));
        let err = send(&socket, Request::Toggle)
            .await
            .expect_err("nothing is listening");
        let text = format!("{err:#}");
        assert!(text.contains("no Compass engine is listening"));
        assert!(text.contains("absent.sock"));
    }

    #[test]
    fn every_request_variant_is_describable() {
        for request in [
            Request::Ping,
            Request::Toggle,
            Request::Show,
            Request::Hide,
            Request::Query {
                text: String::new(),
            },
            Request::Doctor,
            Request::Shutdown,
        ] {
            assert!(!describe(&request).is_empty());
        }
    }
}
