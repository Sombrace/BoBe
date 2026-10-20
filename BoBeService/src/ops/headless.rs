#![allow(
    clippy::needless_return,
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "command match arms return stable exit codes and interactive chat writes to the terminal"
)]

use clap::{Args, Subcommand, ValueEnum};

use crate::{
    config, emit_report, error, http_exit, ops, provider, render_provider_mutation, response_json,
};

#[derive(Args)]
pub(crate) struct ChatArgs {
    prompt: Vec<String>,
    #[arg(long, default_value = "15m", value_parser = crate::parse_duration)]
    timeout: std::time::Duration,
    #[arg(long)]
    json: bool,
    #[arg(long)]
    url: Option<String>,
    #[arg(long)]
    token: Option<String>,
}

pub(crate) async fn execute_chat(args: ChatArgs) -> anyhow::Result<ops::exit::ExitCode> {
    let ChatArgs {
        prompt,
        timeout,
        json,
        url,
        token,
    } = args;
    run_cli_chat(prompt, timeout, json, url.as_deref(), token.as_deref()).await
}

#[derive(Subcommand)]
pub(crate) enum AgentCommands {
    Tools {
        #[command(subcommand)]
        command: ToolCommands,
        #[arg(long, global = true)]
        url: Option<String>,
        #[arg(long, global = true)]
        token: Option<String>,
    },
    Status {
        #[arg(long)]
        json: bool,
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        token: Option<String>,
    },
    Compact {
        #[arg(long, value_parser = clap::value_parser!(u32).range(1..=64_000))]
        keep_recent_tokens: Option<u32>,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        token: Option<String>,
    },
}

#[derive(Subcommand)]
pub(crate) enum ToolCommands {
    List {
        #[arg(long)]
        json: bool,
    },
    Enable {
        #[arg(value_enum)]
        tools: Vec<config::AgentTool>,
        #[arg(long)]
        json: bool,
    },
    Disable {
        #[arg(value_enum)]
        tools: Vec<config::AgentTool>,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Args)]
pub(crate) struct JobsArgs {
    #[command(subcommand)]
    command: JobCommand,
    #[arg(long, global = true)]
    json: bool,
    #[arg(long, global = true)]
    url: Option<String>,
    #[arg(long, global = true)]
    token: Option<String>,
}

#[derive(Args)]
pub(crate) struct RunsArgs {
    #[command(subcommand)]
    command: RunCommand,
    #[arg(long, global = true)]
    json: bool,
    #[arg(long, global = true)]
    url: Option<String>,
    #[arg(long, global = true)]
    token: Option<String>,
}

#[derive(Subcommand)]
enum RunCommand {
    List,
    Stop {
        id: uuid::Uuid,
    },
    Guide {
        id: uuid::Uuid,
        #[arg(required = true, num_args = 1..)]
        text: Vec<String>,
        #[arg(long)]
        follow_up: bool,
        #[arg(long)]
        request_id: Option<uuid::Uuid>,
    },
}

pub(crate) async fn execute_runs(args: RunsArgs) -> anyhow::Result<ops::exit::ExitCode> {
    let loaded = config::Config::load_operational()?;
    let target =
        ops::status::OpsTarget::resolve(&loaded.config, args.url.as_deref(), args.token.as_deref())
            .map_err(error::AppError::Config)?;
    let response = match args.command {
        RunCommand::List => target.get("/runs").await?,
        RunCommand::Stop { id } => target.post(&format!("/runs/{id}/cancel")).await?,
        RunCommand::Guide {
            id,
            text,
            follow_up,
            request_id,
        } => {
            target
                .put_json(
                    &format!(
                        "/runs/{id}/inputs/{}",
                        request_id.unwrap_or_else(uuid::Uuid::new_v4)
                    ),
                    &serde_json::json!({"text":text.join(" "),"follow_up":follow_up}),
                )
                .await?
        }
    };
    let (status, report) = response_json(response).await?;
    emit_report(args.json, &report, serde_json::to_string_pretty(&report)?);
    Ok(http_exit(status))
}

#[derive(Args)]
pub(crate) struct SchedulesArgs {
    #[command(subcommand)]
    command: ScheduleCommand,
    #[arg(long, global = true)]
    json: bool,
    #[arg(long, global = true)]
    url: Option<String>,
    #[arg(long, global = true)]
    token: Option<String>,
}

#[derive(Args)]
pub(crate) struct InputArgs {
    #[command(subcommand)]
    command: InputCommand,
    #[arg(long, global = true)]
    json: bool,
    #[arg(long, global = true)]
    url: Option<String>,
    #[arg(long, global = true)]
    token: Option<String>,
}

#[derive(Subcommand)]
enum InputCommand {
    List,
    Show {
        id: uuid::Uuid,
    },
    Answer {
        id: uuid::Uuid,
        #[arg(required=true,num_args=1..)]
        response: Vec<String>,
        #[arg(long)]
        request_id: Option<uuid::Uuid>,
    },
    Cancel {
        id: uuid::Uuid,
    },
}

pub(crate) async fn execute_input(args: InputArgs) -> anyhow::Result<ops::exit::ExitCode> {
    let loaded = config::Config::load_operational()?;
    let target =
        ops::status::OpsTarget::resolve(&loaded.config, args.url.as_deref(), args.token.as_deref())
            .map_err(error::AppError::Config)?;
    let shown = if let InputCommand::Show { id } = args.command {
        Some(id)
    } else {
        None
    };
    let response=match args.command {
        InputCommand::List=>target.get("/owner-input").await?,
        InputCommand::Show {id}=>target.get(&format!("/owner-input/{id}")).await?,
        InputCommand::Answer {id,response,request_id}=>target.post_json(&format!("/owner-input/{id}/answer"),
            &serde_json::json!({"response_id":request_id.unwrap_or_else(uuid::Uuid::new_v4),"response":response.join(" ")})).await?,
        InputCommand::Cancel {id}=>target.post(&format!("/owner-input/{id}/cancel")).await?,
    };
    let (status, report) = response_json(response).await?;
    emit_report(args.json, &report, serde_json::to_string_pretty(&report)?);
    if status.is_success()
        && report["status"] == "pending"
        && let Some(id) = shown
    {
        let presented = target.post(&format!("/owner-input/{id}/presented")).await?;
        if !presented.status().is_success() {
            return Err(anyhow::anyhow!(
                "the question was shown but its presentation receipt failed"
            ));
        }
    }
    Ok(http_exit(status))
}

#[derive(Subcommand)]
enum ScheduleCommand {
    List,
    Show {
        id: uuid::Uuid,
    },
    Create {
        title: String,
        task: String,
        #[arg(long)]
        at: chrono::DateTime<chrono::Utc>,
        #[arg(long,value_parser=clap::value_parser!(u32).range(1..=31536000))]
        every_seconds: Option<u32>,
        #[arg(long, default_value = "")]
        context: String,
        #[arg(long)]
        goal_id: Option<uuid::Uuid>,
        #[arg(long)]
        request_id: Option<uuid::Uuid>,
        #[arg(long,value_parser=clap::value_parser!(u32).range(1..=86400))]
        timeout_seconds: Option<u32>,
        #[arg(long,value_parser=clap::value_parser!(u32).range(1..=512))]
        max_turns: Option<u32>,
    },
    Pause {
        id: uuid::Uuid,
    },
    Resume {
        id: uuid::Uuid,
    },
    Cancel {
        id: uuid::Uuid,
    },
    Reschedule {
        id: uuid::Uuid,
        #[arg(long)]
        at: chrono::DateTime<chrono::Utc>,
    },
}

pub(crate) async fn execute_schedules(args: SchedulesArgs) -> anyhow::Result<ops::exit::ExitCode> {
    let loaded = config::Config::load_operational()?;
    let target =
        ops::status::OpsTarget::resolve(&loaded.config, args.url.as_deref(), args.token.as_deref())
            .map_err(error::AppError::Config)?;
    let response=match args.command {
        ScheduleCommand::List => target.get("/agent/schedules").await?,
        ScheduleCommand::Show { id } => target.get(&format!("/agent/schedules/{id}")).await?,
        ScheduleCommand::Create { title,task,at,every_seconds,context,goal_id,request_id,timeout_seconds,max_turns } =>
            target.post_json("/agent/schedules",&serde_json::json!({
                "request_id":request_id.unwrap_or_else(uuid::Uuid::new_v4),
                "schedule":{"title":title,"task":task,"at":at,"every_seconds":every_seconds,"context":context,"goal_id":goal_id,
                    "budget":{"timeout_seconds":timeout_seconds,"max_turns":max_turns}},
            })).await?,
        ScheduleCommand::Pause { id } => target.put_json(&format!("/agent/schedules/{id}/enabled"),&serde_json::json!({"enabled":false})).await?,
        ScheduleCommand::Resume { id } => target.put_json(&format!("/agent/schedules/{id}/enabled"),&serde_json::json!({"enabled":true})).await?,
        ScheduleCommand::Cancel { id } => target.delete(&format!("/agent/schedules/{id}")).await?,
        ScheduleCommand::Reschedule { id,at } => {
            let (status,report)=response_json(target.get(&format!("/agent/schedules/{id}")).await?).await?;
            if !status.is_success() {
                emit_report(args.json,&report,serde_json::to_string_pretty(&report)?);
                return Ok(http_exit(status));
            }
            let generation=report.get("generation").and_then(serde_json::Value::as_i64)
                .ok_or_else(||anyhow::anyhow!("schedule response has no generation"))?;
            target.put_json(&format!("/agent/schedules/{id}/time"),&serde_json::json!({"generation":generation,"at":at})).await?
        }
    };
    let (status, report) = response_json(response).await?;
    emit_report(args.json, &report, serde_json::to_string_pretty(&report)?);
    Ok(http_exit(status))
}

#[derive(Subcommand)]
enum JobCommand {
    List,
    Show {
        id: uuid::Uuid,
    },
    Start {
        title: String,
        task: String,
        #[arg(long, default_value = "")]
        context: String,
        #[arg(long)]
        request_id: Option<uuid::Uuid>,
        #[arg(long, value_parser = clap::value_parser!(u32).range(1..=86400))]
        timeout_seconds: Option<u32>,
        #[arg(long, value_parser = clap::value_parser!(u32).range(1..=512))]
        max_turns: Option<u32>,
    },
    Cancel {
        id: uuid::Uuid,
    },
    Resume {
        id: uuid::Uuid,
    },
}

pub(crate) async fn execute_jobs(args: JobsArgs) -> anyhow::Result<ops::exit::ExitCode> {
    let loaded = config::Config::load_operational()?;
    let target =
        ops::status::OpsTarget::resolve(&loaded.config, args.url.as_deref(), args.token.as_deref())
            .map_err(error::AppError::Config)?;
    let response = match args.command {
        JobCommand::List => target.get("/agent/jobs").await?,
        JobCommand::Show { id } => target.get(&format!("/agent/jobs/{id}")).await?,
        JobCommand::Start { title,task,context,request_id,timeout_seconds,max_turns } => target.post_json("/agent/jobs",
            &serde_json::json!({"request_id":request_id.unwrap_or_else(uuid::Uuid::new_v4),"title":title,"task":task,"context":context,"budget":{"timeout_seconds":timeout_seconds,"max_turns":max_turns}}),
        ).await?,
        JobCommand::Cancel { id } => target.post(&format!("/agent/jobs/{id}/cancel")).await?,
        JobCommand::Resume { id } => target.post(&format!("/agent/jobs/{id}/resume")).await?,
    };
    let (status, report) = response_json(response).await?;
    emit_report(args.json, &report, serde_json::to_string_pretty(&report)?);
    Ok(http_exit(status))
}

pub(crate) async fn execute_agent(command: AgentCommands) -> anyhow::Result<ops::exit::ExitCode> {
    if let AgentCommands::Tools {
        command,
        url,
        token,
    } = command
    {
        let loaded = config::Config::load_operational()?;
        let target =
            ops::status::OpsTarget::resolve(&loaded.config, url.as_deref(), token.as_deref())
                .map_err(error::AppError::Config)?;
        let response = target.get("/agent/tools").await?;
        let (status, mut report) = response_json(response).await?;
        let (change, json) = match command {
            ToolCommands::List { json } => (None, json),
            ToolCommands::Enable { tools, json } => (Some((true, tools)), json),
            ToolCommands::Disable { tools, json } => (Some((false, tools)), json),
        };
        if !status.is_success() {
            emit_report(json, &report, serde_json::to_string_pretty(&report)?);
            return Ok(http_exit(status));
        }
        if let Some((enable, tools)) = change {
            if tools.is_empty() {
                return Err(error::AppError::Validation("name at least one tool".into()).into());
            }
            let mut enabled = std::collections::BTreeSet::<String>::new();
            for tool in report["tools"]
                .as_array()
                .ok_or_else(|| error::AppError::Internal("invalid tool catalog".into()))?
            {
                if tool["enabled"] == true {
                    enabled.insert(
                        tool["name"]
                            .as_str()
                            .ok_or_else(|| error::AppError::Internal("invalid tool name".into()))?
                            .to_owned(),
                    );
                }
            }
            for tool in tools {
                if enable {
                    enabled.insert(tool.name().into());
                } else {
                    enabled.remove(tool.name());
                }
            }
            let response = target
                .put_json(
                    "/agent/tools",
                    &serde_json::json!({"enabled_tools":enabled}),
                )
                .await?;
            let (status, value) = response_json(response).await?;
            report = value;
            emit_report(json, &report, serde_json::to_string_pretty(&report)?);
            return Ok(http_exit(status));
        }
        emit_report(json, &report, serde_json::to_string_pretty(&report)?);
        return Ok(ops::exit::ExitCode::Success);
    }
    let (compact, keep_recent_tokens, json, url, token) = match command {
        AgentCommands::Status { json, url, token } => (false, None, json, url, token),
        AgentCommands::Compact {
            keep_recent_tokens,
            json,
            url,
            token,
        } => (true, keep_recent_tokens, json, url, token),
        AgentCommands::Tools { .. } => unreachable!(),
    };
    let loaded = config::Config::load_operational()?;
    let target = ops::status::OpsTarget::resolve(&loaded.config, url.as_deref(), token.as_deref())
        .map_err(error::AppError::Config)?;
    let response = if compact {
        target
            .post_json(
                "/agent/context/compact",
                &serde_json::json!({
                    "keep_recent_tokens":keep_recent_tokens,
                }),
            )
            .await?
    } else {
        target.get("/agent/context").await?
    };
    let (status, report) = response_json(response).await?;
    emit_report(json, &report, serde_json::to_string_pretty(&report)?);
    Ok(http_exit(status))
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum WorkloadArg {
    Chat,
    Goals,
    Consolidate,
    Small,
    All,
}

impl WorkloadArg {
    const fn keys(self) -> &'static [&'static str] {
        match self {
            Self::Chat => &["chat"],
            Self::Goals => &["goals"],
            Self::Consolidate => &["consolidate"],
            Self::Small => &["small"],
            Self::All => &["chat", "goals", "consolidate", "small"],
        }
    }
}

#[derive(Subcommand)]
pub(crate) enum ModelCommands {
    List {
        #[arg(long)]
        profile: Option<String>,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        token: Option<String>,
    },
    Use {
        profile: String,
        model: String,
        #[arg(long, value_enum, default_value_t = WorkloadArg::Chat)]
        workload: WorkloadArg,
        #[arg(long)]
        reasoning_effort: Option<String>,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        token: Option<String>,
    },
}

#[derive(Subcommand)]
pub(crate) enum LocalRuntimeCommands {
    Install {
        #[arg(long, default_value = "local")]
        profile: String,
        #[arg(long)]
        chat_model: String,
        #[arg(long)]
        batch_model: String,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        token: Option<String>,
    },
    Status {
        #[arg(long)]
        json: bool,
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        token: Option<String>,
    },
    Cancel {
        #[arg(long)]
        json: bool,
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        token: Option<String>,
    },
}

#[derive(Subcommand)]
pub(crate) enum MemoryCommands {
    Show {
        #[arg(long)]
        json: bool,
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        token: Option<String>,
    },
    Replace {
        #[arg(long, value_name = "PATH", conflicts_with = "stdin")]
        file: Option<std::path::PathBuf>,
        #[arg(long, conflicts_with = "file")]
        stdin: bool,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        token: Option<String>,
    },
}

#[derive(Subcommand)]
pub(crate) enum ProfileCommands {
    List {
        #[arg(long)]
        json: bool,
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        token: Option<String>,
    },
    Create {
        name: String,
        #[arg(long, value_name = "PATH", conflicts_with = "stdin")]
        file: Option<std::path::PathBuf>,
        #[arg(long, conflicts_with = "file")]
        stdin: bool,
        #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
        enabled: bool,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        token: Option<String>,
    },
    Enable {
        id: uuid::Uuid,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        token: Option<String>,
    },
    Disable {
        id: uuid::Uuid,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        token: Option<String>,
    },
}

#[derive(Subcommand)]
pub(crate) enum SkillCommands {
    List {
        #[arg(long)]
        json: bool,
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        token: Option<String>,
    },
    Show {
        id: String,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        token: Option<String>,
    },
    Reload {
        #[arg(long)]
        json: bool,
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        token: Option<String>,
    },
}

#[derive(Subcommand)]
pub(crate) enum WorkspaceCommands {
    Path {
        #[arg(long)]
        json: bool,
    },
    List {
        #[arg(default_value = "")]
        path: String,
        #[arg(long, default_value_t = 100)]
        max_entries: usize,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        token: Option<String>,
    },
    Read {
        path: String,
        #[arg(long, default_value_t = 256 * 1024)]
        max_bytes: usize,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        token: Option<String>,
    },
    Write {
        path: String,
        #[arg(long, value_name = "PATH", conflicts_with = "stdin")]
        file: Option<std::path::PathBuf>,
        #[arg(long, conflicts_with = "file")]
        stdin: bool,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        token: Option<String>,
    },
}

pub(crate) async fn execute_model(command: ModelCommands) -> anyhow::Result<ops::exit::ExitCode> {
    let loaded = config::Config::load_operational()?;
    match command {
        ModelCommands::List {
            profile,
            json,
            url,
            token,
        } => {
            let target =
                ops::status::OpsTarget::resolve(&loaded.config, url.as_deref(), token.as_deref())
                    .map_err(error::AppError::Config)?;
            let path = profile.map_or_else(
                || "/models".to_owned(),
                |profile| query_path("/models", &[("profile", profile.as_str())]),
            );
            let response = target.get(&path).await?;
            let (status, report) = response_json(response).await?;
            emit_report(json, &report, serde_json::to_string_pretty(&report)?);
            return Ok(http_exit(status));
        }
        ModelCommands::Use {
            profile,
            model,
            workload,
            reasoning_effort,
            json,
            url,
            token,
        } => {
            provider::ProfileId::parse(&profile)?;
            if model.trim().is_empty() {
                return Err(error::AppError::Validation("model must not be empty".into()).into());
            }
            let target =
                ops::status::OpsTarget::resolve(&loaded.config, url.as_deref(), token.as_deref())
                    .map_err(error::AppError::Config)?;
            let mut patch = serde_json::Map::new();
            for key in workload.keys() {
                patch.insert(
                    (*key).into(),
                    serde_json::json!({
                        "profile": profile.clone(),
                        "model": model.clone(),
                        "reasoning_effort": reasoning_effort.clone(),
                    }),
                );
            }
            let response = target
                .patch_json("/provider-routes", &serde_json::Value::Object(patch))
                .await?;
            let (status, report) = response_json(response).await?;
            emit_report(
                json,
                &report,
                render_provider_mutation("Model route updated", &report),
            );
            return Ok(http_exit(status));
        }
    }
}

pub(crate) async fn execute_local_runtime(
    command: LocalRuntimeCommands,
) -> anyhow::Result<ops::exit::ExitCode> {
    let loaded = config::Config::load_operational()?;
    match command {
        LocalRuntimeCommands::Install {
            profile,
            chat_model,
            batch_model,
            json,
            url,
            token,
        } => {
            let target =
                ops::status::OpsTarget::resolve(&loaded.config, url.as_deref(), token.as_deref())
                    .map_err(error::AppError::Config)?;
            let response = target
                .post_json(
                    "/local-runtime/install",
                    &serde_json::json!({
                        "profile": profile,
                        "chat_model": chat_model,
                        "batch_model": batch_model,
                    }),
                )
                .await?;
            if !response.status().is_success() {
                let (status, report) = response_json(response).await?;
                emit_report(json, &report, serde_json::to_string_pretty(&report)?);
                return Ok(http_exit(status));
            }
            let report = local_runtime_snapshot(&target, true).await?;
            let complete = report["status"] == "complete";
            emit_report(json, &report, serde_json::to_string_pretty(&report)?);
            return Ok(if complete {
                ops::exit::ExitCode::Success
            } else {
                ops::exit::ExitCode::Failed
            });
        }
        LocalRuntimeCommands::Status { json, url, token } => {
            let target =
                ops::status::OpsTarget::resolve(&loaded.config, url.as_deref(), token.as_deref())
                    .map_err(error::AppError::Config)?;
            let report = local_runtime_snapshot(&target, false).await?;
            emit_report(json, &report, serde_json::to_string_pretty(&report)?);
        }
        LocalRuntimeCommands::Cancel { json, url, token } => {
            let target =
                ops::status::OpsTarget::resolve(&loaded.config, url.as_deref(), token.as_deref())
                    .map_err(error::AppError::Config)?;
            let response = target.post("/local-runtime/cancel").await?;
            let (status, report) = response_json(response).await?;
            emit_report(json, &report, serde_json::to_string_pretty(&report)?);
            return Ok(http_exit(status));
        }
    }
    Ok(ops::exit::ExitCode::Success)
}

pub(crate) async fn execute_memory(command: MemoryCommands) -> anyhow::Result<ops::exit::ExitCode> {
    let loaded = config::Config::load_operational()?;
    match command {
        MemoryCommands::Show { json, url, token } => {
            let target =
                ops::status::OpsTarget::resolve(&loaded.config, url.as_deref(), token.as_deref())
                    .map_err(error::AppError::Config)?;
            let response = target.get("/memory").await?;
            let (status, report) = response_json(response).await?;
            let human = report["content"].as_str().map_or_else(
                || serde_json::to_string_pretty(&report).unwrap_or_default(),
                str::to_owned,
            );
            emit_report(json, &report, human);
            return Ok(http_exit(status));
        }
        MemoryCommands::Replace {
            file,
            stdin,
            json,
            url,
            token,
        } => {
            let content = read_text_input(file.as_deref(), stdin, "memory content")?;
            let target =
                ops::status::OpsTarget::resolve(&loaded.config, url.as_deref(), token.as_deref())
                    .map_err(error::AppError::Config)?;
            let response = target
                .put_json("/memory", &serde_json::json!({"content": content}))
                .await?;
            let (status, report) = response_json(response).await?;
            emit_report(
                json,
                &report,
                format!("Memory replaced ({} bytes).", report["bytes"]),
            );
            return Ok(http_exit(status));
        }
    }
}

pub(crate) async fn execute_profile(
    command: ProfileCommands,
) -> anyhow::Result<ops::exit::ExitCode> {
    let loaded = config::Config::load_operational()?;
    match command {
        ProfileCommands::List { json, url, token } => {
            let target =
                ops::status::OpsTarget::resolve(&loaded.config, url.as_deref(), token.as_deref())
                    .map_err(error::AppError::Config)?;
            let response = target.get("/user-profiles").await?;
            let (status, report) = response_json(response).await?;
            emit_report(json, &report, serde_json::to_string_pretty(&report)?);
            return Ok(http_exit(status));
        }
        ProfileCommands::Create {
            name,
            file,
            stdin,
            enabled,
            json,
            url,
            token,
        } => {
            let content = read_text_input(file.as_deref(), stdin, "profile content")?;
            let target =
                ops::status::OpsTarget::resolve(&loaded.config, url.as_deref(), token.as_deref())
                    .map_err(error::AppError::Config)?;
            let response = target
                .post_json(
                    "/user-profiles",
                    &serde_json::json!({
                        "name": name,
                        "content": content,
                        "enabled": enabled,
                    }),
                )
                .await?;
            let (status, report) = response_json(response).await?;
            emit_report(json, &report, serde_json::to_string_pretty(&report)?);
            return Ok(http_exit(status));
        }
        ProfileCommands::Enable {
            id,
            json,
            url,
            token,
        } => {
            let target =
                ops::status::OpsTarget::resolve(&loaded.config, url.as_deref(), token.as_deref())
                    .map_err(error::AppError::Config)?;
            let response = target.post(&format!("/user-profiles/{id}/enable")).await?;
            let (status, report) = response_json(response).await?;
            emit_report(json, &report, serde_json::to_string_pretty(&report)?);
            return Ok(http_exit(status));
        }
        ProfileCommands::Disable {
            id,
            json,
            url,
            token,
        } => {
            let target =
                ops::status::OpsTarget::resolve(&loaded.config, url.as_deref(), token.as_deref())
                    .map_err(error::AppError::Config)?;
            let response = target.post(&format!("/user-profiles/{id}/disable")).await?;
            let (status, report) = response_json(response).await?;
            emit_report(json, &report, serde_json::to_string_pretty(&report)?);
            return Ok(http_exit(status));
        }
    }
}

pub(crate) async fn execute_skill(command: SkillCommands) -> anyhow::Result<ops::exit::ExitCode> {
    let loaded = config::Config::load_operational()?;
    match command {
        SkillCommands::List { json, url, token } => {
            let target =
                ops::status::OpsTarget::resolve(&loaded.config, url.as_deref(), token.as_deref())
                    .map_err(error::AppError::Config)?;
            let response = target.get("/skills").await?;
            let (status, report) = response_json(response).await?;
            emit_report(json, &report, serde_json::to_string_pretty(&report)?);
            return Ok(http_exit(status));
        }
        SkillCommands::Show {
            id,
            json,
            url,
            token,
        } => {
            let target =
                ops::status::OpsTarget::resolve(&loaded.config, url.as_deref(), token.as_deref())
                    .map_err(error::AppError::Config)?;
            let response = target.get(&format!("/skills/{id}")).await?;
            let (status, report) = response_json(response).await?;
            let human = report["body"].as_str().map_or_else(
                || serde_json::to_string_pretty(&report).unwrap_or_default(),
                str::to_owned,
            );
            emit_report(json, &report, human);
            return Ok(http_exit(status));
        }
        SkillCommands::Reload { json, url, token } => {
            let target =
                ops::status::OpsTarget::resolve(&loaded.config, url.as_deref(), token.as_deref())
                    .map_err(error::AppError::Config)?;
            let response = target.post("/skills/reload").await?;
            let (status, report) = response_json(response).await?;
            emit_report(json, &report, serde_json::to_string_pretty(&report)?);
            return Ok(http_exit(status));
        }
    }
}

pub(crate) async fn execute_workspace(
    command: WorkspaceCommands,
) -> anyhow::Result<ops::exit::ExitCode> {
    let loaded = config::Config::load_operational()?;
    match command {
        WorkspaceCommands::Path { json } => {
            let report = serde_json::json!({
                "path": loaded.paths.agent_workspace_dir(),
            });
            emit_report(
                json,
                &report,
                loaded.paths.agent_workspace_dir().display().to_string(),
            );
        }
        WorkspaceCommands::List {
            path,
            max_entries,
            json,
            url,
            token,
        } => {
            let target =
                ops::status::OpsTarget::resolve(&loaded.config, url.as_deref(), token.as_deref())
                    .map_err(error::AppError::Config)?;
            let maximum = max_entries.to_string();
            let endpoint = query_path(
                "/workspace",
                &[("path", path.as_str()), ("max_entries", maximum.as_str())],
            );
            let response = target.get(&endpoint).await?;
            let (status, report) = response_json(response).await?;
            emit_report(json, &report, serde_json::to_string_pretty(&report)?);
            return Ok(http_exit(status));
        }
        WorkspaceCommands::Read {
            path,
            max_bytes,
            json,
            url,
            token,
        } => {
            let target =
                ops::status::OpsTarget::resolve(&loaded.config, url.as_deref(), token.as_deref())
                    .map_err(error::AppError::Config)?;
            let maximum = max_bytes.to_string();
            let endpoint = query_path(
                "/workspace/file",
                &[("path", path.as_str()), ("max_bytes", maximum.as_str())],
            );
            let response = target.get(&endpoint).await?;
            let (status, report) = response_json(response).await?;
            let human = report["content"].as_str().map_or_else(
                || serde_json::to_string_pretty(&report).unwrap_or_default(),
                str::to_owned,
            );
            emit_report(json, &report, human);
            return Ok(http_exit(status));
        }
        WorkspaceCommands::Write {
            path,
            file,
            stdin,
            json,
            url,
            token,
        } => {
            let content = read_text_input(file.as_deref(), stdin, "workspace content")?;
            let target =
                ops::status::OpsTarget::resolve(&loaded.config, url.as_deref(), token.as_deref())
                    .map_err(error::AppError::Config)?;
            let response = target
                .put_json(
                    "/workspace/file",
                    &serde_json::json!({"path": path, "content": content}),
                )
                .await?;
            let (status, report) = response_json(response).await?;
            emit_report(json, &report, serde_json::to_string_pretty(&report)?);
            return Ok(http_exit(status));
        }
    }
    Ok(ops::exit::ExitCode::Success)
}

async fn run_cli_chat(
    prompt: Vec<String>,
    timeout: std::time::Duration,
    json: bool,
    url: Option<&str>,
    token: Option<&str>,
) -> anyhow::Result<ops::exit::ExitCode> {
    use std::io::{IsTerminal, Read, Write};
    use tokio::io::AsyncBufReadExt;

    let loaded = config::Config::load_operational()?;
    let target = ops::status::OpsTarget::resolve(&loaded.config, url, token)
        .map_err(error::AppError::Config)?;
    let (readiness_exit, readiness) = ops::status::collect(&target).await;
    if readiness_exit != ops::exit::ExitCode::Success {
        let reason = readiness.error.clone().unwrap_or_else(|| {
            readiness
                .readiness
                .as_ref()
                .map(|view| view.blocking_reasons.join(", "))
                .filter(|reason| !reason.is_empty())
                .unwrap_or_else(|| "daemon is not ready".into())
        });
        emit_report(json, &readiness, format!("BoBe is not ready: {reason}"));
        return Ok(readiness_exit);
    }
    let prompt = prompt.join(" ");
    if !prompt.trim().is_empty() {
        let report = ops::chat::turn(&target, &prompt, timeout, json).await?;
        if json {
            emit_report(true, &report, String::new());
        }
        ops::chat::acknowledge(&target, &report).await?;
        return Ok(if report.cancelled {
            ops::exit::ExitCode::Failed
        } else {
            ops::exit::ExitCode::Success
        });
    }
    if !std::io::stdin().is_terminal() {
        let mut input = String::new();
        std::io::stdin().read_to_string(&mut input)?;
        let report = ops::chat::turn(&target, &input, timeout, json).await?;
        if json {
            emit_report(true, &report, String::new());
        }
        ops::chat::acknowledge(&target, &report).await?;
        return Ok(if report.cancelled {
            ops::exit::ExitCode::Failed
        } else {
            ops::exit::ExitCode::Success
        });
    }
    if json {
        return Err(error::AppError::Validation(
            "--json requires a prompt argument or piped standard input".into(),
        )
        .into());
    }

    eprintln!("BoBe CLI chat. Use /exit to leave; Ctrl-C cancels the active turn.");
    let mut lines = tokio::io::BufReader::new(tokio::io::stdin()).lines();
    loop {
        print!("you> ");
        std::io::stdout().flush()?;
        let Some(line) = lines.next_line().await? else {
            break;
        };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match line {
            "/exit" | "/quit" => break,
            "/help" => {
                eprintln!(
                    "/exit leaves chat. Model and provider changes use `bobe model` and `bobe provider`."
                );
                continue;
            }
            _ => {}
        }
        print!("bobe> ");
        std::io::stdout().flush()?;
        let report = ops::chat::turn(&target, line, timeout, false).await?;
        ops::chat::acknowledge(&target, &report).await?;
        if report.cancelled {
            eprintln!("\n[cancelled]");
        }
    }
    Ok(ops::exit::ExitCode::Success)
}

fn query_path(path: &str, pairs: &[(&str, &str)]) -> String {
    let query = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(pairs.iter().copied())
        .finish();
    format!("{path}?{query}")
}

fn read_text_input(
    path: Option<&std::path::Path>,
    stdin_requested: bool,
    label: &str,
) -> Result<String, error::AppError> {
    use std::io::{IsTerminal, Read};

    let content = if let Some(path) = path {
        std::fs::read_to_string(path)?
    } else if stdin_requested || !std::io::stdin().is_terminal() {
        let mut content = String::new();
        std::io::stdin()
            .take(1024 * 1024 + 1)
            .read_to_string(&mut content)?;
        content
    } else {
        return Err(error::AppError::Validation(format!(
            "{label} requires --file PATH or piped standard input"
        )));
    };
    if content.is_empty() || content.len() > 1024 * 1024 {
        return Err(error::AppError::Validation(format!(
            "{label} must be 1-1048576 bytes"
        )));
    }
    Ok(content)
}

async fn local_runtime_snapshot(
    target: &ops::status::OpsTarget,
    wait_for_terminal: bool,
) -> anyhow::Result<serde_json::Value> {
    use futures::StreamExt;

    let response = target.stream_get("/local-runtime/status").await?;
    if !response.status().is_success() {
        return Err(error::AppError::Internal(format!(
            "local runtime status failed with HTTP {}",
            response.status()
        ))
        .into());
    }
    let mut stream = response.bytes_stream();
    let mut buffer = Vec::new();
    while let Some(chunk) = stream.next().await {
        buffer.extend_from_slice(&chunk?);
        while let Some(end) = buffer.windows(2).position(|window| window == b"\n\n") {
            let frame = buffer.drain(..end).collect::<Vec<_>>();
            buffer.drain(..2);
            let frame = String::from_utf8(frame)?;
            let data = frame
                .lines()
                .filter_map(|line| line.strip_prefix("data:"))
                .map(str::trim_start)
                .collect::<Vec<_>>()
                .join("\n");
            if data.is_empty() {
                continue;
            }
            let snapshot: serde_json::Value = serde_json::from_str(&data)?;
            if !wait_for_terminal
                || matches!(
                    snapshot["status"].as_str(),
                    Some("complete" | "failed" | "canceled")
                )
            {
                return Ok(snapshot);
            }
        }
    }
    Err(
        error::AppError::Internal("local runtime status stream ended without a snapshot".into())
            .into(),
    )
}
