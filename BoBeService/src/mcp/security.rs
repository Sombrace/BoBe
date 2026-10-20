use std::collections::{HashMap, VecDeque};
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::error::AppError;

const MAX_COMMAND_ARGUMENTS: usize = 256;
const MAX_SHEBANG_BYTES: u64 = 1024;
const NESTED_ENV: &str = "env cannot start another env in an MCP command";

const DEFAULT_BLOCKED_COMMANDS: &[&str] = &[
    "rm",
    "rmdir",
    "dd",
    "mkfs",
    "fdisk",
    "format",
    "del",
    "sh",
    "bash",
    "zsh",
    "fish",
    "cmd",
    "powershell",
    "pwsh",
    "sudo",
    "su",
    "doas",
    "chmod",
    "chown",
    "chgrp",
    "kill",
    "killall",
    "pkill",
    "reboot",
    "shutdown",
    "halt",
    "init",
    "systemctl",
    "tee",
    "find",
];

const DEFAULT_DANGEROUS_ENV_KEYS: &[&str] = &[
    "LD_PRELOAD",
    "LD_LIBRARY_PATH",
    "DYLD_INSERT_LIBRARIES",
    "DYLD_LIBRARY_PATH",
];

pub(crate) fn validate_mcp_command_with_args(
    command: &str,
    args: &[String],
    blocked: &[String],
) -> Result<(), AppError> {
    validate_subprocess_command_spec(command, args, blocked).map_err(AppError::Validation)
}

pub(crate) fn validate_mcp_env(
    env: &HashMap<String, String>,
    dangerous_keys: &[String],
) -> Result<(), AppError> {
    validate_subprocess_env(env, dangerous_keys).map_err(AppError::Validation)
}

fn validate_subprocess_command_spec(
    command: &str,
    args: &[String],
    blocked: &[String],
) -> Result<(), String> {
    if args.len() > MAX_COMMAND_ARGUMENTS {
        return Err(format!(
            "MCP commands accept at most {MAX_COMMAND_ARGUMENTS} arguments"
        ));
    }
    let resolved = validate_executable(command, blocked)?;
    if !uses_env_wrapper(command, resolved.as_deref()) {
        return Ok(());
    }
    let Some(wrapped_command) = env_wrapped_command(args, EnvArguments::Exec)? else {
        return Ok(());
    };
    let wrapped = validate_executable(&wrapped_command, blocked)?;
    if uses_env_wrapper(&wrapped_command, wrapped.as_deref()) {
        return Err(NESTED_ENV.to_owned());
    }
    Ok(())
}

fn validate_executable(command: &str, blocked: &[String]) -> Result<Option<PathBuf>, String> {
    validate_command_target(command, blocked)?;
    let resolved = resolve_command_path(command);
    if let Some(resolved) = &resolved {
        validate_command_target(&resolved.to_string_lossy(), blocked)?;
        if let Some(shebang) = read_shebang(resolved) {
            validate_shebang(&shebang, blocked)?;
        }
    }
    Ok(resolved)
}

fn validate_shebang(tokens: &[String], blocked: &[String]) -> Result<(), String> {
    let Some((interpreter, args)) = tokens.split_first() else {
        return Ok(());
    };
    validate_command_target(interpreter, blocked)?;
    if command_basename(interpreter) != Some("env") {
        return Ok(());
    }
    let Some(command) = env_wrapped_command(args, EnvArguments::Shebang)? else {
        return Ok(());
    };
    validate_command_target(&command, blocked)?;
    if command_basename(&command) == Some("env") {
        return Err(NESTED_ENV.to_owned());
    }
    Ok(())
}

fn validate_subprocess_env(
    env: &HashMap<String, String>,
    dangerous_keys: &[String],
) -> Result<(), String> {
    let forbidden: Vec<&str> = env
        .keys()
        .filter(|k| is_dangerous_env_key(k, dangerous_keys))
        .map(String::as_str)
        .collect();

    if !forbidden.is_empty() {
        return Err(format!(
            "Dangerous environment variables not allowed: {}",
            forbidden.join(", ")
        ));
    }

    Ok(())
}

fn is_blocked_command(command: &str, blocked: &[String]) -> bool {
    DEFAULT_BLOCKED_COMMANDS.contains(&command)
        || blocked.iter().any(|candidate| candidate == command)
}

fn is_dangerous_env_key(key: &str, dangerous_keys: &[String]) -> bool {
    DEFAULT_DANGEROUS_ENV_KEYS.contains(&key)
        || dangerous_keys.iter().any(|candidate| candidate == key)
}

fn validate_command_target(command: &str, blocked: &[String]) -> Result<(), String> {
    let basename = Path::new(command)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(command);

    if is_blocked_command(basename, blocked) {
        return Err(format!(
            "Command '{basename}' is blocked for security reasons"
        ));
    }

    Ok(())
}

fn resolve_command_path(command: &str) -> Option<PathBuf> {
    let path = Path::new(command);
    if path.components().count() > 1 || path.is_absolute() {
        return std::fs::canonicalize(path)
            .ok()
            .or_else(|| Some(path.to_path_buf()));
    }

    which::which(command).ok()
}

fn read_shebang(path: &Path) -> Option<Vec<String>> {
    if !std::fs::metadata(path).ok()?.is_file() {
        return None;
    }
    let mut head = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(MAX_SHEBANG_BYTES)
        .read_to_end(&mut head)
        .ok()?;
    let first_line = head.split(|byte| *byte == b'\n').next()?;
    let shebang = std::str::from_utf8(first_line.strip_prefix(b"#!")?).ok()?;
    Some(shebang.split_whitespace().map(str::to_owned).collect())
}

fn uses_env_wrapper(command: &str, resolved: Option<&Path>) -> bool {
    command_basename(command) == Some("env")
        || resolved
            .and_then(|path| path.file_name())
            .and_then(|name| name.to_str())
            == Some("env")
}

fn command_basename(command: &str) -> Option<&str> {
    Path::new(command)
        .file_name()
        .and_then(|name| name.to_str())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum EnvArguments {
    Exec,
    Shebang,
}

fn env_wrapped_command(args: &[String], form: EnvArguments) -> Result<Option<String>, String> {
    let mut tokens: VecDeque<String> = args.iter().cloned().collect();
    while let Some(token) = tokens.pop_front() {
        if token == "--" || token == "-" {
            break;
        }
        let Some(option) = token.strip_prefix('-') else {
            tokens.push_front(token);
            break;
        };
        if let Some(long) = option.strip_prefix('-') {
            let (name, value) = long
                .split_once('=')
                .map_or((long, None), |(name, value)| (name, Some(value)));
            match (name, value) {
                ("ignore-environment" | "null" | "debug", None) | ("unset" | "chdir", Some(_)) => {}
                ("unset" | "chdir", None) => take_env_option_value(&mut tokens, &token)?,
                ("split-string", value) if form == EnvArguments::Shebang => {
                    ensure_literal_split_string(args)?;
                    if let Some(value) = value {
                        tokens.push_front(value.to_owned());
                    }
                }
                _ => return Err(disallowed_env_option(&token)),
            }
            continue;
        }
        for (index, flag) in option.char_indices() {
            let attached = &option[index + flag.len_utf8()..];
            match flag {
                'i' | '0' | 'v' => continue,
                'u' | 'C' if attached.is_empty() => take_env_option_value(&mut tokens, &token)?,
                'u' | 'C' => {}
                'S' if form == EnvArguments::Shebang => {
                    ensure_literal_split_string(args)?;
                    if !attached.is_empty() {
                        tokens.push_front(attached.to_owned());
                    }
                }
                _ => return Err(disallowed_env_option(&token)),
            }
            break;
        }
    }
    while tokens.front().is_some_and(|token| token.contains('=')) {
        tokens.pop_front();
    }
    let Some(command) = tokens.pop_front() else {
        return Ok(None);
    };
    if command.starts_with('-') {
        return Err(format!(
            "env would run '{command}', which looks like an option; MCP commands must name an executable"
        ));
    }
    Ok(Some(command))
}

fn take_env_option_value(tokens: &mut VecDeque<String>, option: &str) -> Result<(), String> {
    tokens
        .pop_front()
        .map(drop)
        .ok_or_else(|| format!("env option '{option}' is missing its value"))
}

fn ensure_literal_split_string(tokens: &[String]) -> Result<(), String> {
    if tokens
        .iter()
        .any(|token| token.contains(['"', '\'', '\\', '$']))
    {
        return Err(
            "env -S with quotes, escapes or variables is not allowed in an MCP script interpreter"
                .to_owned(),
        );
    }
    Ok(())
}

fn disallowed_env_option(option: &str) -> String {
    format!(
        "env option '{option}' is not allowed in MCP commands because it can hide the program that runs"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use uuid::Uuid;

    #[test]
    fn test_validate_command_allows_safe() {
        assert!(validate_mcp_command_with_args("npx", &[], &[]).is_ok());
        assert!(validate_mcp_command_with_args("uvx", &[], &[]).is_ok());
        assert!(validate_mcp_command_with_args("docker", &[], &[]).is_ok());
        assert!(validate_mcp_command_with_args("node", &[], &[]).is_ok());
        assert!(
            validate_mcp_command_with_args(
                "env",
                &[
                    "-i".into(),
                    "FOO=bar".into(),
                    "node".into(),
                    "server.js".into()
                ],
                &[]
            )
            .is_ok()
        );
    }

    #[test]
    fn test_validate_command_blocks_dangerous() {
        assert!(validate_mcp_command_with_args("rm", &[], &[]).is_err());
        assert!(validate_mcp_command_with_args("/bin/rm", &[], &[]).is_err());
        assert!(validate_mcp_command_with_args("sudo", &[], &[]).is_err());
        assert!(validate_mcp_command_with_args("bash", &[], &[]).is_err());
    }

    #[test]
    fn test_validate_command_blocks_tee_and_find() {
        assert!(validate_mcp_command_with_args("tee", &[], &[]).is_err());
        assert!(validate_mcp_command_with_args("/usr/bin/tee", &[], &[]).is_err());
        assert!(validate_mcp_command_with_args("find", &[], &[]).is_err());
        assert!(validate_mcp_command_with_args("/usr/bin/find", &[], &[]).is_err());
    }

    #[test]
    fn test_validate_command_blocks_env_wrapped_tee_and_find() {
        assert!(
            validate_mcp_command_with_args(
                "env",
                &["-i".into(), "tee".into(), "/tmp/x".into()],
                &[]
            )
            .is_err()
        );
        assert!(
            validate_mcp_command_with_args(
                "env",
                &["-i".into(), "find".into(), "/".into(), "-delete".into()],
                &[]
            )
            .is_err()
        );
    }

    #[test]
    fn test_validate_command_with_custom_blocklist() {
        let custom = vec!["npm".into(), "yarn".into()];
        assert!(validate_mcp_command_with_args("npm", &[], &custom).is_err());
        assert!(validate_mcp_command_with_args("bash", &[], &custom).is_err());
    }

    #[test]
    fn test_validate_command_blocks_env_wrapped_shell() {
        let result = validate_mcp_command_with_args(
            "env",
            &[
                "-i".into(),
                "FOO=bar".into(),
                "bash".into(),
                "-lc".into(),
                "echo ok".into(),
            ],
            &[],
        );

        assert!(result.is_err());
    }

    #[cfg(unix)]
    #[test]
    fn test_validate_command_blocks_symlinked_shell() {
        let tmp = std::env::temp_dir().join(format!("bobe-mcp-security-{}", Uuid::new_v4()));
        fs::create_dir_all(&tmp).unwrap();
        let link_path = tmp.join("safe-shell");
        std::os::unix::fs::symlink("/bin/bash", &link_path).unwrap();

        let result = validate_mcp_command_with_args(link_path.to_str().unwrap(), &[], &[]);

        fs::remove_file(&link_path).unwrap();
        fs::remove_dir(&tmp).unwrap();

        assert!(result.is_err());
    }

    #[cfg(unix)]
    #[test]
    fn test_validate_command_blocks_script_with_shell_shebang() {
        let tmp = std::env::temp_dir().join(format!("bobe-mcp-security-{}", Uuid::new_v4()));
        fs::create_dir_all(&tmp).unwrap();
        let script_path = tmp.join("safe-wrapper");
        fs::write(&script_path, "#!/bin/bash\necho ok\n").unwrap();

        let result = validate_mcp_command_with_args(script_path.to_str().unwrap(), &[], &[]);

        fs::remove_file(&script_path).unwrap();
        fs::remove_dir(&tmp).unwrap();

        assert!(result.is_err());
    }

    fn env_args(args: &[&str]) -> Vec<String> {
        args.iter().map(|arg| (*arg).to_owned()).collect()
    }

    #[test]
    fn env_split_string_is_rejected_in_every_form() {
        for args in [
            &["-S", "bash -c id"][..],
            &["-Sbash -c id"],
            &["-S", "rm -rf /srv/data"],
            &["--split-string=sh -c id"],
            &["--split-string", "bash -c id"],
            &["--split=bash -c id"],
            &["-iS", "bash -c id"],
            &["-i", "-vSbash -c id"],
            &["-S", "node server.js"],
        ] {
            let error = validate_mcp_command_with_args("env", &env_args(args), &[])
                .expect_err("env split strings hide the program that runs");
            assert!(
                error.to_string().contains("not allowed"),
                "{args:?}: {error}"
            );
        }
    }

    #[test]
    fn env_options_with_values_do_not_hide_a_blocked_command() {
        for args in [
            &["-u", "HOME", "bash"][..],
            &["-uHOME", "bash"],
            &["--unset", "HOME", "bash"],
            &["--unset=HOME", "rm", "-rf", "."],
            &["-C", "/srv", "sh"],
            &["-iC/srv", "bash"],
            &["--chdir=/srv", "bash"],
            &["--", "FOO=1", "bash"],
            &["-", "FOO=1", "bash"],
            &["env", "-S", "bash -c id"],
            &["env", "-i", "bash"],
            &["-P", "/bin", "sh"],
            &["-a", "node", "bash"],
            &["--argv0=node", "bash"],
            &["FOO=1", "-i", "bash"],
        ] {
            assert!(
                validate_mcp_command_with_args("env", &env_args(args), &[]).is_err(),
                "{args:?}"
            );
        }
        for args in [
            &["-u", "HOME", "node", "server.js"][..],
            &["--chdir=/srv", "node", "server.js"],
            &["--", "FOO=1", "node", "server.js"],
            &["-0iv", "FOO=1", "node"],
        ] {
            assert!(
                validate_mcp_command_with_args("env", &env_args(args), &[]).is_ok(),
                "{args:?}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn env_shebangs_are_parsed_like_env_arguments() {
        let tmp = std::env::temp_dir().join(format!("bobe-mcp-security-{}", Uuid::new_v4()));
        fs::create_dir_all(&tmp).unwrap();
        let validate = |name: &str, shebang: &str| {
            let script = tmp.join(name);
            fs::write(&script, format!("{shebang}\nconsole.log('ok')\n")).unwrap();
            validate_mcp_command_with_args(script.to_str().unwrap(), &[], &[])
        };

        let blocked = [
            validate("split", "#!/usr/bin/env -S bash -e"),
            validate("attached", "#!/usr/bin/env -Ssh"),
            validate("unset", "#!/usr/bin/env -u HOME bash"),
            validate("assignment", "#!/usr/bin/env FOO=1 sh"),
            validate("variable", "#!/usr/bin/env -S ${SHELL}"),
            validate("nested", "#!/usr/bin/env env -S bash"),
        ];
        let allowed = [
            validate("node", "#!/usr/bin/env node"),
            validate("node-flags", "#!/usr/bin/env -S node --no-warnings"),
        ];
        fs::remove_dir_all(&tmp).unwrap();

        for result in blocked {
            assert!(result.is_err());
        }
        for result in allowed {
            assert!(result.is_ok(), "{result:?}");
        }
    }

    #[test]
    fn long_and_nested_env_chains_are_refused_without_recursion() {
        let chain = vec!["env".to_owned(); 20_000];
        let error = validate_mcp_command_with_args("env", &chain, &[])
            .expect_err("an oversized argument list is refused");
        assert!(
            error.to_string().contains("at most 256 arguments"),
            "{error}"
        );

        let chain = vec!["env".to_owned(); MAX_COMMAND_ARGUMENTS];
        let error =
            validate_mcp_command_with_args("env", &chain, &[]).expect_err("env cannot start env");
        assert!(error.to_string().contains("another env"), "{error}");
        for args in [
            &["-i", "env", "node"][..],
            &["FOO=1", "/usr/bin/env", "node"],
            &["--", "env"],
        ] {
            assert!(
                validate_mcp_command_with_args("env", &env_args(args), &[]).is_err(),
                "{args:?}"
            );
        }

        let mut assignments = vec!["FOO=1".to_owned(); MAX_COMMAND_ARGUMENTS - 1];
        assignments.push("node".to_owned());
        assert!(validate_mcp_command_with_args("env", &assignments, &[]).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn only_regular_files_are_read_for_a_script_interpreter() {
        assert!(validate_mcp_command_with_args("/dev/zero", &[], &[]).is_ok());
        assert!(read_shebang(Path::new("/dev/zero")).is_none());
    }

    #[test]
    fn test_validate_env_allows_safe() {
        let env: HashMap<String, String> = [("PATH".into(), "/usr/bin".into())].into();
        assert!(validate_mcp_env(&env, &[]).is_ok());
    }

    #[test]
    fn test_validate_env_blocks_dangerous() {
        let env: HashMap<String, String> = [("LD_PRELOAD".into(), "evil.so".into())].into();
        assert!(validate_mcp_env(&env, &[]).is_err());
    }

    #[test]
    fn test_validate_env_custom_keys_are_additive() {
        let env: HashMap<String, String> = [("PYTHONPATH".into(), "evil".into())].into();
        assert!(validate_mcp_env(&env, &["PYTHONPATH".into()]).is_err());

        let env: HashMap<String, String> = [("LD_PRELOAD".into(), "evil.so".into())].into();
        assert!(validate_mcp_env(&env, &["PYTHONPATH".into()]).is_err());
    }
}
