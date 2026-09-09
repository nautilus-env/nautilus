//! Helpers shared by the module tests: a live server and schema files on disk.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::time::Duration;

use tower_lsp::lsp_types::Url;
use tower_lsp::{ClientSocket, LspService};
use tower_service::Service;

use crate::backend::Backend;

/// A server wired to a socket the test reads notifications from.
pub(crate) fn service() -> (LspService<Backend>, ClientSocket) {
    LspService::new(Backend::new)
}

/// The same, after the `initialize` handshake, so that notifications the client
/// only accepts from an initialized server reach the socket.
pub(crate) async fn initialized_service() -> (LspService<Backend>, ClientSocket) {
    let (mut service, socket) = service();
    service
        .call(
            tower_lsp::jsonrpc::Request::build("initialize")
                .id(1)
                .params(serde_json::json!({ "capabilities": {} }))
                .finish(),
        )
        .await
        .expect("initialize service")
        .expect("initialize response");
    (service, socket)
}

/// Run `future` under a deadline.
///
/// The client channel holds a single message, so a handler publishing fewer
/// notifications than the test reads would deadlock both sides: the deadline
/// turns that into a failure.
pub(crate) async fn within<T>(future: impl Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(10), future)
        .await
        .expect("the server published every notification the test expects")
}

/// An empty directory of its own for `name`, recreated on every run.
pub(crate) fn schema_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "nautilus-lsp-backend-{}-{}",
        name,
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

pub(crate) fn file_uri(path: &Path) -> Url {
    Url::from_file_path(std::fs::canonicalize(path).expect("canonical path")).expect("file uri")
}

/// The percent-encoded drive letter VS Code sends, which has to resolve to the
/// same file as [`file_uri`].
#[cfg(windows)]
pub(crate) fn vscode_file_uri(path: &Path) -> Url {
    let mut serialized = file_uri(path).to_string();
    let path_start = serialized.find(":///").expect("file URI path") + 4;
    let drive_colon = path_start
        + serialized[path_start..]
            .find(':')
            .expect("Windows drive colon");
    serialized.replace_range(drive_colon..=drive_colon, "%3A");
    Url::parse(&serialized).expect("VS Code file URI")
}

#[cfg(not(windows))]
pub(crate) fn vscode_file_uri(path: &Path) -> Url {
    file_uri(path)
}
