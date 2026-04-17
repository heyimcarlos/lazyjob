use std::io::{self, BufRead, Write};
use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::json;
use tokio::sync::mpsc;

use lazyjob_core::config::Config;
use lazyjob_core::cover_letter::{CoverLetterOptions, CoverLetterService, CoverLetterTemplate};
use lazyjob_core::db::Database;
use lazyjob_core::discovery::Completer;
use lazyjob_core::domain::JobId;
use lazyjob_core::error::CoreError;
use lazyjob_core::repositories::JobRepository;
use lazyjob_core::resume::repository::ResumeVersionRepository;
use lazyjob_core::resume::{ResumeTailor, TailoringOptions};
use lazyjob_llm::{ChatMessage, CompletionOptions, LlmBuilder, LlmProvider};
use lazyjob_ralph::protocol::{WorkerCommand, WorkerEvent};

use lazyjob_core::credentials::CredentialManager;

struct LlmProviderCompleter {
    provider: Box<dyn LlmProvider>,
}

#[async_trait]
impl Completer for LlmProviderCompleter {
    async fn complete(&self, system: &str, user: &str) -> lazyjob_core::error::Result<String> {
        let messages = vec![
            ChatMessage::System(system.into()),
            ChatMessage::User(user.into()),
        ];
        let opts = CompletionOptions::default();
        self.provider
            .complete(messages, opts)
            .await
            .map(|r| r.content)
            .map_err(|e| CoreError::Parse(e.to_string()))
    }
}

fn emit_event(event: &WorkerEvent) {
    let mut line = serde_json::to_string(event).expect("WorkerEvent serialization never fails");
    line.push('\n');
    let stdout = io::stdout();
    let mut handle = stdout.lock();
    let _ = handle.write_all(line.as_bytes());
    let _ = handle.flush();
}

fn emit_status(phase: &str, progress: f32, message: &str) {
    emit_event(&WorkerEvent::Status {
        phase: phase.to_string(),
        progress,
        message: message.to_string(),
    });
}

pub async fn run_worker(db_url: &str) -> Result<()> {
    let line = {
        let stdin = io::stdin();
        let mut buf = String::new();
        stdin.lock().read_line(&mut buf)?;
        buf
    };

    let cmd: WorkerCommand =
        serde_json::from_str(line.trim()).context("failed to parse WorkerCommand from stdin")?;

    match cmd {
        WorkerCommand::Start { loop_type, params } => match loop_type.as_str() {
            "resume-tailor" => run_resume_tailor(params, db_url).await,
            "cover-letter" => run_cover_letter(params, db_url).await,
            other => {
                emit_event(&WorkerEvent::Error {
                    code: "unknown_loop_type".into(),
                    message: format!("unknown loop type: {other}"),
                });
                emit_event(&WorkerEvent::Done { success: false });
                Ok(())
            }
        },
        WorkerCommand::Cancel => Ok(()),
    }
}

async fn run_resume_tailor(params: serde_json::Value, db_url: &str) -> Result<()> {
    let job_id_str = params["job_id"]
        .as_str()
        .context("missing job_id in params")?;
    let job_uuid: uuid::Uuid = job_id_str.parse().context("invalid job UUID")?;
    let job_id = JobId::from_uuid(job_uuid);

    emit_status("connecting", 5.0, "Connecting to database...");

    let db = Database::connect(db_url)
        .await
        .context("failed to connect to database")?;

    let repo = JobRepository::new(db.pool().clone());
    let job = repo
        .find_by_id(&job_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Job not found: {job_uuid}"))?;

    emit_status("loading_profile", 8.0, "Loading life sheet...");

    let life_sheet = lazyjob_core::life_sheet::load_from_db(db.pool())
        .await
        .context("failed to load life sheet")?;

    let config = Config::load().unwrap_or_default();
    let creds = CredentialManager::new();
    let provider = LlmBuilder::from_config(&config, &creds).map_err(|e| anyhow::anyhow!("{e}"))?;

    let completer = Arc::new(LlmProviderCompleter { provider });
    let tailor = ResumeTailor::new(completer);

    let (progress_tx, mut progress_rx) = mpsc::channel(16);

    let options = TailoringOptions::default();
    let tailor_fut = tailor.tailor(&job, &life_sheet, options.clone(), Some(progress_tx));

    let forward_fut = async {
        while let Some(event) = progress_rx.recv().await {
            let (phase, pct) = match &event {
                lazyjob_core::resume::ProgressEvent::ParsingJd { pct } => {
                    ("parsing_jd", *pct as f32)
                }
                lazyjob_core::resume::ProgressEvent::GapAnalysis { pct } => {
                    ("gap_analysis", *pct as f32)
                }
                lazyjob_core::resume::ProgressEvent::FabricationPreCheck { pct } => {
                    ("fabrication_pre_check", *pct as f32)
                }
                lazyjob_core::resume::ProgressEvent::RewritingBullets { pct } => {
                    ("rewriting_bullets", *pct as f32)
                }
                lazyjob_core::resume::ProgressEvent::GeneratingSummary { pct } => {
                    ("generating_summary", *pct as f32)
                }
                lazyjob_core::resume::ProgressEvent::Assembling { pct } => {
                    ("assembling", *pct as f32)
                }
                lazyjob_core::resume::ProgressEvent::Done { match_score } => {
                    emit_status("done", 100.0, &format!("Match score: {match_score:.0}%"));
                    return;
                }
                lazyjob_core::resume::ProgressEvent::Error { message } => {
                    emit_event(&WorkerEvent::Error {
                        code: "pipeline_error".into(),
                        message: message.clone(),
                    });
                    return;
                }
            };
            emit_status(phase, pct, phase);
        }
    };

    let (result, _) = tokio::join!(tailor_fut, forward_fut);

    match result {
        Ok((content, gap_report, fabrication_report)) => {
            let version = ResumeTailor::build_resume_version(
                &job,
                content,
                gap_report.clone(),
                fabrication_report,
                options,
                "v1".into(),
            );

            let version_repo = ResumeVersionRepository::new(db.pool().clone());
            let count: i64 = version_repo.count_for_job(&job_uuid).await.unwrap_or(0);
            let version = ResumeTailor::build_resume_version(
                &job,
                version.content,
                version.gap_report,
                version.fabrication_report,
                version.tailoring_options,
                format!("v{}", count + 1),
            );

            if let Err(e) = version_repo.save(&version).await {
                emit_event(&WorkerEvent::Error {
                    code: "save_error".into(),
                    message: format!("failed to save resume version: {e}"),
                });
                emit_event(&WorkerEvent::Done { success: false });
            } else {
                emit_event(&WorkerEvent::Results {
                    data: json!({
                        "version_id": version.id.0.to_string(),
                        "label": version.label,
                        "match_score": gap_report.match_score,
                    }),
                });
                emit_event(&WorkerEvent::Done { success: true });
            }
        }
        Err(e) => {
            emit_event(&WorkerEvent::Error {
                code: "tailor_error".into(),
                message: e.to_string(),
            });
            emit_event(&WorkerEvent::Done { success: false });
        }
    }

    db.close().await;
    Ok(())
}

async fn run_cover_letter(params: serde_json::Value, db_url: &str) -> Result<()> {
    let job_id_str = params["job_id"]
        .as_str()
        .context("missing job_id in params")?;
    let job_uuid: uuid::Uuid = job_id_str.parse().context("invalid job UUID")?;
    let job_id = JobId::from_uuid(job_uuid);

    let template_str = params["template"].as_str().unwrap_or("standard");

    emit_status("connecting", 5.0, "Connecting to database...");

    let db = Database::connect(db_url)
        .await
        .context("failed to connect to database")?;

    let repo = JobRepository::new(db.pool().clone());
    let job = repo
        .find_by_id(&job_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Job not found: {job_uuid}"))?;

    emit_status("loading_profile", 8.0, "Loading life sheet...");

    let life_sheet = lazyjob_core::life_sheet::load_from_db(db.pool())
        .await
        .context("failed to load life sheet")?;

    let config = Config::load().unwrap_or_default();
    let creds = CredentialManager::new();
    let provider = LlmBuilder::from_config(&config, &creds).map_err(|e| anyhow::anyhow!("{e}"))?;

    let completer = Arc::new(LlmProviderCompleter { provider });
    let svc = CoverLetterService::new(completer, db.pool().clone());

    let options = CoverLetterOptions {
        template: CoverLetterTemplate::from_str_loose(template_str),
        ..Default::default()
    };

    let (progress_tx, mut progress_rx) = mpsc::channel(16);

    let generate_fut = svc.generate(&job, &life_sheet, options, Some(progress_tx));

    let forward_fut = async {
        while let Some(event) = progress_rx.recv().await {
            let (phase, pct) = match &event {
                lazyjob_core::cover_letter::ProgressEvent::Generating { pct } => {
                    ("generating", *pct as f32)
                }
                lazyjob_core::cover_letter::ProgressEvent::CheckingFabrication { pct } => {
                    ("checking_fabrication", *pct as f32)
                }
                lazyjob_core::cover_letter::ProgressEvent::Persisting { pct } => {
                    ("persisting", *pct as f32)
                }
                lazyjob_core::cover_letter::ProgressEvent::Done { version } => {
                    emit_status("done", 100.0, &format!("Version {version} saved"));
                    return;
                }
                lazyjob_core::cover_letter::ProgressEvent::Error { message } => {
                    emit_event(&WorkerEvent::Error {
                        code: "pipeline_error".into(),
                        message: message.clone(),
                    });
                    return;
                }
            };
            emit_status(phase, pct, phase);
        }
    };

    let (result, _) = tokio::join!(generate_fut, forward_fut);

    match result {
        Ok(version) => {
            emit_event(&WorkerEvent::Results {
                data: json!({
                    "version_id": version.id.0.to_string(),
                    "version": version.version,
                    "template": version.template.as_str(),
                    "words": version.plain_text.split_whitespace().count(),
                }),
            });
            emit_event(&WorkerEvent::Done { success: true });
        }
        Err(e) => {
            emit_event(&WorkerEvent::Error {
                code: "cover_letter_error".into(),
                message: e.to_string(),
            });
            emit_event(&WorkerEvent::Done { success: false });
        }
    }

    db.close().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use lazyjob_ralph::protocol::WorkerEvent;

    #[test]
    fn emit_event_produces_valid_json() {
        let event = WorkerEvent::Status {
            phase: "test".into(),
            progress: 0.5,
            message: "testing".into(),
        };
        let json = serde_json::to_string(&event).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["type"], "status");
        assert_eq!(parsed["phase"], "test");
    }

    #[test]
    fn worker_command_start_deserializes() {
        let json = r#"{"type":"start","loop_type":"resume-tailor","params":{"job_id":"abc"}}"#;
        let cmd: WorkerCommand = serde_json::from_str(json).unwrap();
        match cmd {
            WorkerCommand::Start { loop_type, params } => {
                assert_eq!(loop_type, "resume-tailor");
                assert_eq!(params["job_id"], "abc");
            }
            _ => panic!("expected Start"),
        }
    }

    #[test]
    fn worker_command_cancel_deserializes() {
        let json = r#"{"type":"cancel"}"#;
        let cmd: WorkerCommand = serde_json::from_str(json).unwrap();
        assert!(matches!(cmd, WorkerCommand::Cancel));
    }

    #[test]
    fn results_event_serializes_with_data() {
        let event = WorkerEvent::Results {
            data: json!({"version_id": "abc-123", "match_score": 75.0}),
        };
        let json = serde_json::to_string(&event).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["type"], "results");
        assert_eq!(parsed["data"]["version_id"], "abc-123");
    }
}
