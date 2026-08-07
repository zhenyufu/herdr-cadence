use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use herdr_cadence::app::{App, context_project_path, context_workspace_id};

#[derive(Parser)]
#[command(name = "herdr-cadence", version, about)]
struct Cli {
    #[arg(long, global = true, env = "CADENCE_STATE_DIR")]
    state_dir: Option<PathBuf>,
    #[arg(long, global = true, env = "CADENCE_PROJECT_ROOT")]
    project_root: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Action {
        #[arg(value_enum)]
        action: Action,
    },
    Startup,
    Event,
    Run {
        #[command(subcommand)]
        command: RunCommand,
    },
    Worker {
        #[command(subcommand)]
        command: WorkerCommand,
    },
}

#[derive(Clone, ValueEnum)]
enum Action {
    EnableProject,
    DisableProject,
    Start,
    Status,
}

#[derive(Subcommand)]
enum RunCommand {
    Status,
    Finish,
}

#[derive(Subcommand)]
enum WorkerCommand {
    Spawn {
        #[arg(long)]
        request_file: PathBuf,
    },
    List,
    Status {
        worker_id: String,
    },
    Report {
        worker_id: String,
    },
    Complete {
        worker_id: String,
        #[arg(long)]
        report_file: PathBuf,
    },
    Integrate {
        worker_id: String,
    },
    Prompt {
        worker_id: String,
        #[arg(long)]
        prompt_file: PathBuf,
    },
    Cancel {
        worker_id: String,
    },
}

fn main() {
    if let Err(error) = run() {
        let causes = error
            .chain()
            .skip(1)
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        eprintln!(
            "{}",
            serde_json::json!({"error": error.to_string(), "causes": causes})
        );
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let root = cli
        .project_root
        .map(Ok)
        .unwrap_or_else(context_project_path)?;
    let state_dir = cli
        .state_dir
        .or_else(|| std::env::var_os("HERDR_PLUGIN_STATE_DIR").map(PathBuf::from))
        .context("CADENCE_STATE_DIR or HERDR_PLUGIN_STATE_DIR is required")?;
    let runtime_only = matches!(&cli.command, Command::Startup | Command::Event);
    let app = if runtime_only {
        App::new_runtime(root, state_dir)?
    } else {
        App::new(root, state_dir)?
    };
    let value = match cli.command {
        Command::Action { action } => match action {
            Action::EnableProject => app.enable_project()?,
            Action::DisableProject => app.disable_project()?,
            Action::Start => app.start(&context_workspace_id()?)?,
            Action::Status => app.status()?,
        },
        Command::Startup => app.startup()?,
        Command::Event => {
            let name =
                std::env::var("HERDR_PLUGIN_EVENT").context("HERDR_PLUGIN_EVENT is required")?;
            let event = std::env::var("HERDR_PLUGIN_EVENT_JSON")
                .context("HERDR_PLUGIN_EVENT_JSON is required")?;
            app.handle_event(&name, &event)?
        }
        Command::Run { command } => match command {
            RunCommand::Status => app.status()?,
            RunCommand::Finish => app.finish_run()?,
        },
        Command::Worker { command } => match command {
            WorkerCommand::Spawn { request_file } => app.spawn_worker(&request_file)?,
            WorkerCommand::List => app.list_workers()?,
            WorkerCommand::Status { worker_id } => app.worker_status(&worker_id)?,
            WorkerCommand::Report { worker_id } => app.worker_report(&worker_id)?,
            WorkerCommand::Complete {
                worker_id,
                report_file,
            } => app.complete_worker(&worker_id, &report_file)?,
            WorkerCommand::Integrate { worker_id } => app.integrate_worker(&worker_id)?,
            WorkerCommand::Prompt {
                worker_id,
                prompt_file,
            } => app.prompt_worker(&worker_id, &prompt_file)?,
            WorkerCommand::Cancel { worker_id } => app.cancel_worker(&worker_id)?,
        },
    };
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}
