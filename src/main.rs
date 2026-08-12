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
    Agent {
        #[command(subcommand)]
        command: AgentCommand,
    },
}

#[derive(Clone, ValueEnum)]
enum Action {
    Init,
    DisableProject,
    Start,
    Status,
    ValidateConfig,
}

#[derive(Subcommand)]
enum RunCommand {
    Status,
    Finish {
        #[arg(long)]
        force: bool,
    },
}

#[derive(Subcommand)]
enum AgentCommand {
    Spawn {
        #[arg(long)]
        request_file: PathBuf,
    },
    List,
    Status {
        agent_id: String,
    },
    Report {
        agent_id: String,
    },
    Complete {
        agent_id: String,
        #[arg(long)]
        report_file: PathBuf,
    },
    Integrate {
        agent_id: String,
    },
    Prompt {
        agent_id: String,
        #[arg(long)]
        prompt_file: PathBuf,
    },
    Cancel {
        agent_id: String,
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
            Action::Init => app.init_project()?,
            Action::DisableProject => app.disable_project()?,
            Action::Start => app.start(&context_workspace_id()?)?,
            Action::Status => app.status()?,
            Action::ValidateConfig => app.validate_config()?,
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
            RunCommand::Finish { force } => app.finish_run(force)?,
        },
        Command::Agent { command } => match command {
            AgentCommand::Spawn { request_file } => app.spawn_agent(&request_file)?,
            AgentCommand::List => app.list_agents()?,
            AgentCommand::Status { agent_id } => app.agent_status(&agent_id)?,
            AgentCommand::Report { agent_id } => app.agent_report(&agent_id)?,
            AgentCommand::Complete {
                agent_id,
                report_file,
            } => app.complete_agent(&agent_id, &report_file)?,
            AgentCommand::Integrate { agent_id } => app.integrate_agent(&agent_id)?,
            AgentCommand::Prompt {
                agent_id,
                prompt_file,
            } => app.prompt_agent(&agent_id, &prompt_file)?,
            AgentCommand::Cancel { agent_id } => app.cancel_agent(&agent_id)?,
        },
    };
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}
