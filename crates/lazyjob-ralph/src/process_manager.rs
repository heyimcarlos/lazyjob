use std::collections::HashMap;
use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::RalphError;
use crate::error::Result;
use crate::protocol::{NdjsonCodec, WorkerCommand, WorkerEvent};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RunId(Uuid);

impl RunId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn as_uuid(&self) -> &Uuid {
        &self.0
    }
}

impl Default for RunId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for RunId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

struct ProcessHandle {
    child: Child,
    stdin: ChildStdin,
}

pub struct RalphProcessManager {
    binary_path: PathBuf,
    binary_args: Vec<OsString>,
    running: HashMap<RunId, ProcessHandle>,
    event_tx: broadcast::Sender<(RunId, WorkerEvent)>,
}

impl RalphProcessManager {
    pub fn new() -> Self {
        let binary_path =
            std::env::current_exe().unwrap_or_else(|_| PathBuf::from("lazyjob-ralph"));
        Self::with_binary(binary_path)
    }

    pub fn with_binary(binary_path: PathBuf) -> Self {
        Self::with_binary_and_args(binary_path, vec![])
    }

    pub fn with_binary_and_args(binary_path: PathBuf, args: Vec<OsString>) -> Self {
        let (event_tx, _) = broadcast::channel(100);
        Self {
            binary_path,
            binary_args: args,
            running: HashMap::new(),
            event_tx,
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<(RunId, WorkerEvent)> {
        self.event_tx.subscribe()
    }

    pub fn active_runs(&self) -> Vec<RunId> {
        self.running.keys().copied().collect()
    }

    pub async fn spawn(&mut self, loop_type: &str, params: serde_json::Value) -> Result<RunId> {
        let run_id = RunId::new();

        let mut cmd = Command::new(&self.binary_path);
        for arg in &self.binary_args {
            cmd.arg(arg);
        }
        let mut child = cmd
            .arg("worker")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()?;

        let mut stdin = child.stdin.take().expect("stdin was piped");
        let stdout = child.stdout.take().expect("stdout was piped");

        let start_cmd = WorkerCommand::Start {
            loop_type: loop_type.to_string(),
            params,
        };
        let encoded = NdjsonCodec::encode(&start_cmd);
        stdin.write_all(encoded.as_bytes()).await?;
        stdin.flush().await?;

        let event_tx = self.event_tx.clone();
        tokio::spawn(async move {
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if line.trim().is_empty() {
                    continue;
                }
                if let Ok(event) = NdjsonCodec::decode(&line) {
                    let _ = event_tx.send((run_id, event));
                }
            }
        });

        self.running.insert(run_id, ProcessHandle { child, stdin });
        Ok(run_id)
    }

    pub async fn cancel(&mut self, run_id: &RunId) -> Result<()> {
        let handle = self
            .running
            .get_mut(run_id)
            .ok_or_else(|| RalphError::NotFound(run_id.to_string()))?;

        let cancel_line = NdjsonCodec::encode(&WorkerCommand::Cancel);
        let _ = handle.stdin.write_all(cancel_line.as_bytes()).await;
        let _ = handle.stdin.flush().await;

        let wait = tokio::time::timeout(Duration::from_secs(3), handle.child.wait()).await;

        if wait.is_err() {
            let _ = handle.child.kill().await;
        }

        self.running.remove(run_id);
        Ok(())
    }
}

impl Default for RalphProcessManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // learning test: verifies tokio::process::Command with piped stdout allows async line reading
    #[tokio::test]
    async fn tokio_process_piped_stdout() {
        let mut child = Command::new("echo")
            .arg("hello from subprocess")
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();

        let stdout = child.stdout.take().unwrap();
        let mut lines = BufReader::new(stdout).lines();
        let line = lines.next_line().await.unwrap().unwrap();
        assert_eq!(line, "hello from subprocess");
        child.wait().await.unwrap();
    }

    // learning test: verifies writing to subprocess stdin and reading back via stdout (cat)
    #[tokio::test]
    async fn tokio_process_stdin_write() {
        let mut child = Command::new("cat")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();

        let mut stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();

        stdin.write_all(b"round-trip-test\n").await.unwrap();
        drop(stdin);

        let mut lines = BufReader::new(stdout).lines();
        let line = lines.next_line().await.unwrap().unwrap();
        assert_eq!(line, "round-trip-test");
        child.wait().await.unwrap();
    }

    #[test]
    fn run_id_is_unique() {
        let a = RunId::new();
        let b = RunId::new();
        assert_ne!(a, b);
    }

    #[test]
    fn run_id_display_is_uuid_format() {
        let id = RunId::new();
        let s = id.to_string();
        assert_eq!(s.len(), 36);
        assert_eq!(s.chars().filter(|&c| c == '-').count(), 4);
    }

    fn echo_manager() -> RalphProcessManager {
        // sh -c 'read line; printf ...; printf ...' worker
        // The trailing "worker" arg becomes $0 in the shell -c script — harmless.
        RalphProcessManager::with_binary_and_args(
            PathBuf::from("sh"),
            vec![
                OsString::from("-c"),
                OsString::from(
                    r#"read line; printf '{"type":"status","phase":"init","progress":0.1,"message":"started"}\n'; printf '{"type":"done","success":true}\n'"#,
                ),
            ],
        )
    }

    fn sleep_manager() -> RalphProcessManager {
        // sh -c 'read line; sleep 60' worker
        RalphProcessManager::with_binary_and_args(
            PathBuf::from("sh"),
            vec![OsString::from("-c"), OsString::from("read line; sleep 60")],
        )
    }

    #[tokio::test]
    async fn spawn_emits_worker_events() {
        let mut manager = echo_manager();
        let mut rx = manager.subscribe();

        let run_id = manager
            .spawn("job_discovery", json!({"test": true}))
            .await
            .unwrap();

        let (id1, event1) = rx.recv().await.unwrap();
        let (id2, event2) = rx.recv().await.unwrap();

        assert_eq!(id1, run_id);
        assert_eq!(id2, run_id);

        assert!(
            matches!(event1, WorkerEvent::Status { ref phase, .. } if phase == "init"),
            "expected Status, got {:?}",
            event1
        );
        assert_eq!(event2, WorkerEvent::Done { success: true });
    }

    #[tokio::test]
    async fn cancel_terminates_running_process() {
        let mut manager = sleep_manager();
        let run_id = manager.spawn("job_discovery", json!(null)).await.unwrap();

        assert!(manager.active_runs().contains(&run_id));

        manager.cancel(&run_id).await.unwrap();

        assert!(!manager.active_runs().contains(&run_id));
    }

    #[tokio::test]
    async fn cancel_unknown_run_returns_not_found() {
        let mut manager = echo_manager();
        let fake_id = RunId::new();
        let result = manager.cancel(&fake_id).await;
        assert!(matches!(result, Err(RalphError::NotFound(_))));
    }
}
