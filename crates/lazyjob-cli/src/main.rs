use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use secrecy::SecretString;

use lazyjob_core::config::Config;
use lazyjob_core::credentials::CredentialManager;
use lazyjob_core::db::{DEFAULT_DATABASE_URL, Database};
use lazyjob_core::domain::Job;
use lazyjob_core::repositories::{JobRepository, Pagination};

#[derive(Parser)]
#[command(name = "lazyjob", version, about = "AI-powered job search TUI")]
struct Cli {
    #[arg(long, env = "DATABASE_URL")]
    database_url: Option<String>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Jobs(JobsArgs),
    Profile(ProfileArgs),
    Config(ConfigArgs),
    Tui,
}

#[derive(Parser)]
struct JobsArgs {
    #[command(subcommand)]
    command: JobsCommand,
}

#[derive(Subcommand)]
enum JobsCommand {
    List,
    Add {
        #[arg(long)]
        title: String,
        #[arg(long)]
        company: Option<String>,
        #[arg(long)]
        url: Option<String>,
    },
}

#[derive(Parser)]
struct ProfileArgs {
    #[command(subcommand)]
    command: ProfileCommand,
}

#[derive(Subcommand)]
enum ProfileCommand {
    Import {
        #[arg(long)]
        file: PathBuf,
    },
    Export,
}

#[derive(Parser)]
struct ConfigArgs {
    #[command(subcommand)]
    command: ConfigCommand,
}

#[derive(Subcommand)]
#[allow(clippy::enum_variant_names)]
enum ConfigCommand {
    SetKey {
        #[arg(long)]
        provider: String,
        #[arg(long)]
        key: String,
    },
    GetKey {
        #[arg(long)]
        provider: String,
    },
    DeleteKey {
        #[arg(long)]
        provider: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();
    run(cli).await
}

async fn run(cli: Cli) -> Result<()> {
    let db_url = cli.database_url.as_deref().unwrap_or(DEFAULT_DATABASE_URL);

    match cli.command {
        Commands::Tui => {
            let config = Config::load().unwrap_or_default();
            lazyjob_tui::run(std::sync::Arc::new(config)).await
        }
        Commands::Config(args) => handle_config(args),
        Commands::Jobs(args) => {
            let db = connect_db(db_url).await?;
            let result = match args.command {
                JobsCommand::List => handle_jobs_list(&db).await,
                JobsCommand::Add {
                    title,
                    company,
                    url,
                } => handle_jobs_add(&db, title, company, url).await,
            };
            db.close().await;
            result
        }
        Commands::Profile(args) => {
            let db = connect_db(db_url).await?;
            let result = match args.command {
                ProfileCommand::Import { file } => handle_profile_import(&db, file).await,
                ProfileCommand::Export => handle_profile_export(&db).await,
            };
            db.close().await;
            result
        }
    }
}

async fn connect_db(url: &str) -> Result<Database> {
    Database::connect(url)
        .await
        .context("failed to connect to database")
}

async fn handle_jobs_list(db: &Database) -> Result<()> {
    let repo = JobRepository::new(db.pool().clone());
    let jobs = repo.list(&Pagination::default()).await?;

    if jobs.is_empty() {
        println!("No jobs found.");
        return Ok(());
    }

    println!("{:<40} {:<20} {:<30}", "TITLE", "COMPANY", "URL");
    println!("{}", "-".repeat(90));

    for job in &jobs {
        println!(
            "{:<40} {:<20} {:<30}",
            truncate(&job.title, 38),
            truncate(job.company_name.as_deref().unwrap_or("-"), 18),
            truncate(job.url.as_deref().unwrap_or("-"), 28),
        );
    }

    println!("\n{} job(s) found.", jobs.len());
    Ok(())
}

async fn handle_jobs_add(
    db: &Database,
    title: String,
    company: Option<String>,
    url: Option<String>,
) -> Result<()> {
    let mut job = Job::new(&title);
    job.company_name = company;
    job.url = url;

    let repo = JobRepository::new(db.pool().clone());
    repo.insert(&job).await?;

    println!("Job added: {} (id: {})", job.title, job.id);
    Ok(())
}

async fn handle_profile_import(db: &Database, file: PathBuf) -> Result<()> {
    let sheet = lazyjob_core::life_sheet::import_from_yaml(&file, db.pool())
        .await
        .context("failed to import life sheet")?;

    println!("Imported life sheet for: {}", sheet.basics.name);
    println!(
        "  {} work experience(s), {} education(s), {} skill categories",
        sheet.work_experience.len(),
        sheet.education.len(),
        sheet.skills.len(),
    );
    Ok(())
}

async fn handle_profile_export(db: &Database) -> Result<()> {
    let sheet = lazyjob_core::life_sheet::load_from_db(db.pool())
        .await
        .context("failed to load life sheet from database")?;

    let json_resume = sheet.to_json_resume();
    let json = serde_json::to_string_pretty(&json_resume)?;
    println!("{json}");
    Ok(())
}

fn handle_config(args: ConfigArgs) -> Result<()> {
    let cred = CredentialManager::new();
    match args.command {
        ConfigCommand::SetKey { provider, key } => {
            let secret = SecretString::new(key);
            cred.set_api_key(&provider, &secret)
                .context("failed to store API key")?;
            println!("API key for '{provider}' stored in system keychain.");
            Ok(())
        }
        ConfigCommand::GetKey { provider } => {
            match cred
                .get_api_key(&provider)
                .context("failed to retrieve API key")?
            {
                Some(_) => println!("API key for '{provider}': ******* (set)"),
                None => println!("No API key found for '{provider}'."),
            }
            Ok(())
        }
        ConfigCommand::DeleteKey { provider } => {
            cred.delete_api_key(&provider)
                .context("failed to delete API key")?;
            println!("API key for '{provider}' deleted from system keychain.");
            Ok(())
        }
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max - 1])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn clap_derive_nested_subcommands() {
        let cli = Cli::try_parse_from(["lazyjob", "jobs", "list"]).unwrap();
        assert!(matches!(cli.command, Commands::Jobs(_)));
    }

    #[test]
    fn parse_jobs_list() {
        let cli = Cli::try_parse_from(["lazyjob", "jobs", "list"]).unwrap();
        match cli.command {
            Commands::Jobs(args) => assert!(matches!(args.command, JobsCommand::List)),
            _ => panic!("expected Jobs command"),
        }
    }

    #[test]
    fn parse_jobs_add() {
        let cli = Cli::try_parse_from([
            "lazyjob",
            "jobs",
            "add",
            "--title",
            "Rust Developer",
            "--company",
            "Acme",
            "--url",
            "https://example.com",
        ])
        .unwrap();

        match cli.command {
            Commands::Jobs(args) => match args.command {
                JobsCommand::Add {
                    title,
                    company,
                    url,
                } => {
                    assert_eq!(title, "Rust Developer");
                    assert_eq!(company.as_deref(), Some("Acme"));
                    assert_eq!(url.as_deref(), Some("https://example.com"));
                }
                _ => panic!("expected Add"),
            },
            _ => panic!("expected Jobs"),
        }
    }

    #[test]
    fn parse_jobs_add_minimal() {
        let cli = Cli::try_parse_from(["lazyjob", "jobs", "add", "--title", "Engineer"]).unwrap();

        match cli.command {
            Commands::Jobs(args) => match args.command {
                JobsCommand::Add {
                    title,
                    company,
                    url,
                } => {
                    assert_eq!(title, "Engineer");
                    assert!(company.is_none());
                    assert!(url.is_none());
                }
                _ => panic!("expected Add"),
            },
            _ => panic!("expected Jobs"),
        }
    }

    #[test]
    fn parse_profile_import() {
        let cli = Cli::try_parse_from([
            "lazyjob",
            "profile",
            "import",
            "--file",
            "/path/to/sheet.yaml",
        ])
        .unwrap();

        match cli.command {
            Commands::Profile(args) => match args.command {
                ProfileCommand::Import { file } => {
                    assert_eq!(file, PathBuf::from("/path/to/sheet.yaml"));
                }
                _ => panic!("expected Import"),
            },
            _ => panic!("expected Profile"),
        }
    }

    #[test]
    fn parse_profile_export() {
        let cli = Cli::try_parse_from(["lazyjob", "profile", "export"]).unwrap();
        match cli.command {
            Commands::Profile(args) => assert!(matches!(args.command, ProfileCommand::Export)),
            _ => panic!("expected Profile"),
        }
    }

    #[test]
    fn parse_tui() {
        let cli = Cli::try_parse_from(["lazyjob", "tui"]).unwrap();
        assert!(matches!(cli.command, Commands::Tui));
    }

    #[test]
    fn parse_database_url_flag() {
        let cli =
            Cli::try_parse_from(["lazyjob", "--database-url", "postgresql://custom/db", "tui"])
                .unwrap();
        assert_eq!(cli.database_url.as_deref(), Some("postgresql://custom/db"));
    }

    #[test]
    fn database_url_defaults_to_none() {
        let cli = Cli::try_parse_from(["lazyjob", "tui"]).unwrap();
        assert!(cli.database_url.is_none());
    }

    #[test]
    fn cross_crate_version_accessible() {
        let core_ver = lazyjob_core::version();
        let tui_ver = lazyjob_tui::version();
        assert_eq!(core_ver, tui_ver);
    }

    #[test]
    fn truncate_short_string() {
        assert_eq!(truncate("hello", 10), "hello");
    }

    #[test]
    fn truncate_long_string() {
        let result = truncate("a very long string here", 10);
        assert_eq!(result.chars().count(), 10);
        assert!(result.ends_with('…'));
    }

    #[test]
    fn parse_config_set_key() {
        let cli = Cli::try_parse_from([
            "lazyjob",
            "config",
            "set-key",
            "--provider",
            "anthropic",
            "--key",
            "sk-ant-test",
        ])
        .unwrap();
        match cli.command {
            Commands::Config(args) => match args.command {
                ConfigCommand::SetKey { provider, key } => {
                    assert_eq!(provider, "anthropic");
                    assert_eq!(key, "sk-ant-test");
                }
                _ => panic!("expected SetKey"),
            },
            _ => panic!("expected Config"),
        }
    }

    #[test]
    fn parse_config_get_key() {
        let cli =
            Cli::try_parse_from(["lazyjob", "config", "get-key", "--provider", "openai"]).unwrap();
        match cli.command {
            Commands::Config(args) => match args.command {
                ConfigCommand::GetKey { provider } => {
                    assert_eq!(provider, "openai");
                }
                _ => panic!("expected GetKey"),
            },
            _ => panic!("expected Config"),
        }
    }

    #[test]
    fn parse_config_delete_key() {
        let cli = Cli::try_parse_from(["lazyjob", "config", "delete-key", "--provider", "ollama"])
            .unwrap();
        match cli.command {
            Commands::Config(args) => match args.command {
                ConfigCommand::DeleteKey { provider } => {
                    assert_eq!(provider, "ollama");
                }
                _ => panic!("expected DeleteKey"),
            },
            _ => panic!("expected Config"),
        }
    }
}
