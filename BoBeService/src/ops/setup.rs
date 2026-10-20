use std::io::{BufRead, Write};
use std::time::Duration;

use serde_json::{Value, json};

use crate::config;
use crate::db::runtime_initialization_repo::InitializationState;
use crate::error::AppError;
use crate::ops;
use crate::ops::status::OpsTarget;

const READY_WAIT: Duration = Duration::from_mins(2);
const SHOWN_MODELS: usize = 9;

struct Service {
    name: &'static str,
    kind: &'static str,
    profile: &'static str,
    asks_address: bool,
    needs_key: bool,
}

const ONLINE_SERVICES: [Service; 5] = [
    Service {
        name: "Rail Relay",
        kind: "rail_relay",
        profile: "rail-relay",
        asks_address: true,
        needs_key: true,
    },
    Service {
        name: "OpenAI",
        kind: "open_ai",
        profile: "openai",
        asks_address: false,
        needs_key: true,
    },
    Service {
        name: "Anthropic",
        kind: "anthropic",
        profile: "anthropic",
        asks_address: false,
        needs_key: true,
    },
    Service {
        name: "DeepSeek",
        kind: "deepseek",
        profile: "deepseek",
        asks_address: false,
        needs_key: true,
    },
    Service {
        name: "OpenRouter",
        kind: "open_router",
        profile: "openrouter",
        asks_address: false,
        needs_key: true,
    },
];

const OWN_SERVER: Service = Service {
    name: "Your own AI server",
    kind: "open_ai_compatible",
    profile: "custom",
    asks_address: true,
    needs_key: false,
};

pub(crate) struct SetupContext {
    pub(crate) executable: std::path::PathBuf,
    pub(crate) agent: Option<crate::agent::RuntimeFiles>,
    pub(crate) service_paths: ops::service::ServicePaths,
}

#[expect(clippy::print_stdout, reason = "setup talks to the person running it")]
pub(crate) async fn execute(context: SetupContext) -> Result<ops::exit::ExitCode, AppError> {
    let prompt = Prompt;
    println!("BoBe setup\n");
    ensure_running(&context).await?;
    let target = owner_target()?;
    if initialization(&target).await? == InitializationState::ProviderRequired {
        choose_ai(&prompt, &target, &context).await?;
        if initialization(&target).await? == InitializationState::ProviderRequired {
            advance(&target, "personalization_required").await?;
        }
    }
    if initialization(&target).await? == InitializationState::PersonalizationRequired {
        personalize(&prompt, &target).await?;
    }
    let address = reach(&prompt, &target, &context).await?;
    pair(&prompt, address.as_deref()).await?;
    println!("\nBoBe is ready. Talk with her: bobe chat");
    Ok(ops::exit::ExitCode::Success)
}

fn owner_target() -> Result<OpsTarget, AppError> {
    let loaded = config::Config::load_operational()?;
    OpsTarget::resolve(&loaded.config, None, None).map_err(AppError::Config)
}

async fn reachable() -> bool {
    let Ok(target) = owner_target() else {
        return false;
    };
    let (_, report) = ops::status::collect(&target).await;
    report.reachable && report.readiness.is_some()
}

async fn wait_reachable() -> Result<(), AppError> {
    let deadline = tokio::time::Instant::now() + READY_WAIT;
    while tokio::time::Instant::now() < deadline {
        if reachable().await {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    Err(AppError::Internal(
        "BoBe did not start. See: bobe service logs".into(),
    ))
}

#[expect(clippy::print_stdout, reason = "setup talks to the person running it")]
async fn ensure_running(context: &SetupContext) -> Result<(), AppError> {
    if reachable().await {
        return Ok(());
    }
    println!("Starting BoBe…");
    let status = ops::service::status(&context.service_paths).await?;
    if status.installed && status.manages_data_root {
        ops::service::restart(&context.service_paths).await?;
    } else {
        ops::service::install(
            &context.executable,
            context.agent.as_ref(),
            &context.service_paths,
        )
        .await
        .map_err(|error| match error {
            AppError::Conflict(_) => AppError::Conflict(
                "another BoBe already runs as this computer's service. Remove it with \
                 bobe service uninstall, using that BoBe's data, then run bobe setup again"
                    .into(),
            ),
            error => error,
        })?;
    }
    wait_reachable().await?;
    println!("BoBe is running and starts with this computer.\n");
    Ok(())
}

async fn restart(context: &SetupContext) -> Result<(), AppError> {
    ops::service::restart(&context.service_paths).await?;
    tokio::time::sleep(Duration::from_secs(1)).await;
    wait_reachable().await
}

async fn initialization(target: &OpsTarget) -> Result<InitializationState, AppError> {
    let (_, report) = ops::status::collect(target).await;
    report
        .readiness
        .map(|view| view.initialization_state)
        .ok_or_else(|| {
            AppError::Internal(
                report
                    .error
                    .unwrap_or_else(|| "BoBe is not answering".into()),
            )
        })
}

async fn advance(target: &OpsTarget, state: &str) -> Result<(), AppError> {
    let response = target
        .post_json("/initialization", &json!({ "initialization_state": state }))
        .await?;
    expect_success(response).await.map(|_| ())
}

async fn expect_success(response: reqwest::Response) -> Result<Value, AppError> {
    let status = response.status();
    let body = response.json::<Value>().await.unwrap_or(Value::Null);
    if status.is_success() {
        return Ok(body);
    }
    let message = body
        .get("error")
        .and_then(|error| error.get("message").or(Some(error)))
        .and_then(Value::as_str)
        .map_or_else(|| format!("BoBe answered {status}"), str::to_owned);
    Err(AppError::Validation(message))
}

#[expect(clippy::print_stdout, reason = "setup talks to the person running it")]
async fn choose_ai(
    prompt: &Prompt,
    target: &OpsTarget,
    context: &SetupContext,
) -> Result<(), AppError> {
    let engine = prompt.choose(
        "How should BoBe think?",
        &["An online AI (recommended)", "Your own AI server"],
    )?;
    let service = if engine == 0 {
        let names: Vec<&str> = ONLINE_SERVICES.iter().map(|service| service.name).collect();
        &ONLINE_SERVICES[prompt.choose("Which service?", &names)?]
    } else {
        &OWN_SERVER
    };
    loop {
        let base_url = if service.asks_address {
            Some(prompt.line("Address: ")?)
        } else {
            None
        };
        let key = if service.needs_key {
            prompt.secret("Key: ")?
        } else {
            prompt.secret("Key (leave empty if none): ")?
        };
        let offline = !service.needs_key && key.is_empty();
        println!("Checking…");
        let access = Access {
            base_url: base_url.as_deref(),
            key: &key,
            offline,
        };
        match connect(prompt, target, context, service, &access).await {
            Ok(restart_required) => {
                if restart_required {
                    restart(context).await?;
                }
                println!("BoBe can think.\n");
                return Ok(());
            }
            Err(error) => println!("That didn't work: {error}\nTry again.\n"),
        }
    }
}

struct Access<'a> {
    base_url: Option<&'a str>,
    key: &'a str,
    offline: bool,
}

async fn connect(
    prompt: &Prompt,
    target: &OpsTarget,
    context: &SetupContext,
    service: &Service,
    access: &Access<'_>,
) -> Result<bool, AppError> {
    let profile = service.profile;
    let base_url = access.base_url;
    let key = access.key;
    let offline = access.offline;
    expect_success(
        target
            .put_json(
                &format!("/providers/{profile}"),
                &json!({
                    "kind": service.kind,
                    "auth_method": "api_key",
                    "base_url": base_url,
                    "organization": null,
                    "offline": offline,
                }),
            )
            .await?,
    )
    .await?;
    if !key.is_empty() {
        expect_success(
            target
                .put_json(
                    &format!("/providers/{profile}/credential"),
                    &json!({ "api_key": key }),
                )
                .await?,
        )
        .await?;
    }
    let saved = expect_success(target.get("/providers").await?).await?;
    let needs_restart = saved
        .get("profiles")
        .and_then(Value::as_array)
        .and_then(|profiles| {
            profiles
                .iter()
                .find(|entry| entry.get("id").and_then(Value::as_str) == Some(profile))
        })
        .and_then(|entry| entry.get("restart_required"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if needs_restart {
        restart(context).await?;
    }
    let catalog = expect_success(target.get(&format!("/models?profile={profile}")).await?).await?;
    let models: Vec<String> = catalog
        .get("models")
        .and_then(Value::as_array)
        .map(|models| {
            models
                .iter()
                .filter_map(|model| model.get("id").and_then(Value::as_str))
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    if models.is_empty() {
        return Err(AppError::Validation("the service offered no models".into()));
    }
    let model = prompt.choose_model(&models)?;
    let activation = expect_success(
        target
            .put_json(
                &format!("/providers/{profile}/activation"),
                &json!({
                    "kind": service.kind,
                    "auth_method": "api_key",
                    "base_url": base_url,
                    "organization": null,
                    "offline": offline,
                    "local_runtime": null,
                    "api_key": null,
                    "chat_model": model,
                    "goals_model": model,
                    "consolidate_model": model,
                    "small_model": model,
                }),
            )
            .await?,
    )
    .await?;
    Ok(activation
        .get("restart_required")
        .and_then(Value::as_bool)
        .unwrap_or(false))
}

async fn personalize(prompt: &Prompt, target: &OpsTarget) -> Result<(), AppError> {
    let name = prompt.line("What should BoBe call you? ")?;
    if !name.is_empty() {
        let existing = expect_success(target.get("/user-profiles").await?).await?;
        let known = existing
            .get("profiles")
            .and_then(Value::as_array)
            .is_some_and(|profiles| {
                profiles.iter().any(|profile| {
                    profile
                        .get("name")
                        .and_then(Value::as_str)
                        .is_some_and(|known| known.eq_ignore_ascii_case(&name))
                })
            });
        if !known {
            expect_success(
                target
                    .post_json(
                        "/user-profiles",
                        &json!({
                            "name": name,
                            "content": format!("# About me\n\n## Preferred name\n\n{name}"),
                            "enabled": true,
                        }),
                    )
                    .await?,
            )
            .await?;
        }
    }
    advance(target, "ready").await
}

#[expect(clippy::print_stdout, reason = "setup talks to the person running it")]
async fn reach(
    prompt: &Prompt,
    target: &OpsTarget,
    context: &SetupContext,
) -> Result<Option<String>, AppError> {
    let loaded = config::Config::load_operational()?;
    let port = loaded.config.server.port;
    if loaded.config.server.is_networked() {
        return Ok(first_address(&loaded.config.server.allowed_hosts, port));
    }
    println!();
    if !prompt.confirm("Let your Mac and iPhone reach BoBe?")? {
        return Ok(None);
    }
    let hosts = crate::util::network::reachable_hosts();
    if hosts.is_empty() {
        println!("This computer has no network address yet. Run bobe setup again once it does.");
        return Ok(None);
    }
    expect_success(
        target
            .patch_json(
                "/config",
                &json!({ "changes": {
                    "server.host": "0.0.0.0",
                    "server.allowed_hosts": hosts,
                }}),
            )
            .await?,
    )
    .await?;
    if ops::service::status(&context.service_paths)
        .await?
        .installed
    {
        restart(context).await?;
    } else {
        println!("Restart BoBe so other devices can reach her.");
    }
    let address = first_address(&hosts, port);
    if let Some(address) = &address {
        println!("Devices can reach BoBe at {address}");
    }
    if firewall_may_block() {
        println!("If your firewall blocks them: sudo ufw allow {port}/tcp");
    }
    Ok(address)
}

fn first_address(hosts: &[String], port: u16) -> Option<String> {
    hosts
        .iter()
        .find(|host| host.parse::<std::net::IpAddr>().is_ok())
        .or_else(|| hosts.first())
        .map(|host| format!("{host}:{port}"))
}

fn firewall_may_block() -> bool {
    std::fs::read_to_string("/etc/ufw/ufw.conf")
        .is_ok_and(|text| text.lines().any(|line| line.trim() == "ENABLED=yes"))
}

#[expect(clippy::print_stdout, reason = "setup talks to the person running it")]
async fn pair(prompt: &Prompt, address: Option<&str>) -> Result<(), AppError> {
    let Some(address) = address else {
        return Ok(());
    };
    println!();
    let choice = prompt.choose("Pair a device now?", &["iPhone", "Mac", "Not now"])?;
    let kind = match choice {
        0 => "phone",
        1 => "mac_app",
        _ => return Ok(()),
    };
    let loaded = config::Config::load_operational()?;
    let label = crate::api::scope::ScopeProfile::from_kind(kind)
        .map_or("Device", |profile| profile.default_label());
    let report = ops::admin_socket::request(
        &loaded.paths.admin_socket(),
        json!({ "command": "pair", "kind": kind, "label": label }),
    )
    .await?;
    println!();
    if kind == "phone" {
        println!("{}", ops::device_pairing::render(&report));
    } else {
        let code = report
            .get("code")
            .and_then(Value::as_str)
            .unwrap_or_default();
        println!("On your Mac, open BoBe and choose \"On another computer\".");
        println!("Address  {address}");
        println!("Code     {}", ops::device_pairing::spaced(code));
        println!("The code expires in 5 minutes.");
    }
    Ok(())
}

struct Prompt;

#[expect(clippy::print_stdout, reason = "setup talks to the person running it")]
impl Prompt {
    fn line(&self, question: &str) -> Result<String, AppError> {
        print!("{question}");
        std::io::stdout().flush()?;
        let mut answer = String::new();
        if std::io::stdin().lock().read_line(&mut answer)? == 0 {
            return Err(AppError::Validation(
                "setup needs answers; run bobe setup in a terminal".into(),
            ));
        }
        Ok(answer.trim().to_owned())
    }

    fn secret(&self, question: &str) -> Result<String, AppError> {
        let hidden = crate::util::terminal::HiddenInput::start().ok().flatten();
        let answer = self.line(question);
        if hidden.is_some() {
            println!();
        }
        drop(hidden);
        answer
    }

    fn confirm(&self, question: &str) -> Result<bool, AppError> {
        let answer = self.line(&format!("{question} [Y/n] "))?;
        Ok(!matches!(answer.to_lowercase().as_str(), "n" | "no"))
    }

    fn choose(&self, title: &str, options: &[&str]) -> Result<usize, AppError> {
        println!("{title}");
        for (index, option) in options.iter().enumerate() {
            println!("  {}  {option}", index + 1);
        }
        loop {
            let answer = self.line("Choose [1]: ")?;
            if answer.is_empty() {
                return Ok(0);
            }
            if let Ok(number) = answer.parse::<usize>()
                && (1..=options.len()).contains(&number)
            {
                return Ok(number - 1);
            }
        }
    }

    fn choose_model(&self, models: &[String]) -> Result<String, AppError> {
        println!("Which model?");
        for (index, model) in models.iter().take(SHOWN_MODELS).enumerate() {
            println!("  {}  {model}", index + 1);
        }
        if models.len() > SHOWN_MODELS {
            println!("  or type the name of another model");
        }
        loop {
            let answer = self.line("Choose [1]: ")?;
            if answer.is_empty() {
                return Ok(models[0].clone());
            }
            if let Ok(number) = answer.parse::<usize>()
                && (1..=models.len().min(SHOWN_MODELS)).contains(&number)
            {
                return Ok(models[number - 1].clone());
            }
            if let Some(model) = models.iter().find(|model| **model == answer) {
                return Ok(model.clone());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_address_shown_prefers_an_ip_address() {
        let hosts = vec![
            "bobe-box.local".to_owned(),
            "192.168.1.20".to_owned(),
            "100.64.0.7".to_owned(),
        ];
        assert_eq!(
            first_address(&hosts, 8766).as_deref(),
            Some("192.168.1.20:8766")
        );
        assert_eq!(
            first_address(&["bobe.example.net".to_owned()], 8766).as_deref(),
            Some("bobe.example.net:8766")
        );
        assert_eq!(first_address(&[], 8766), None);
    }

    #[test]
    fn every_guided_service_names_a_wire_kind_and_profile() {
        for service in ONLINE_SERVICES.iter().chain(std::iter::once(&OWN_SERVER)) {
            assert!(crate::provider::ProfileId::parse(service.profile).is_ok());
            assert_ne!(service.kind, "");
        }
    }
}
