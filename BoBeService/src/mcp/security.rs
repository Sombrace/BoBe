use std::collections::HashMap;
use std::io::BufRead;
use std::path::{Path, PathBuf};

use crate::error::AppError;

pub(crate) const DEFAULT_BLOCKED_COMMANDS: &[&str] = &[
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

pub(crate) const DEFAULT_DANGEROUS_ENV_KEYS: &[&str] = &[
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
    validate_subprocess_command_spec(command, args, blocked, true).map_err(AppError::Mcp)
}

pub(crate) fn validate_mcp_env(
    env: &HashMap<String, String>,
    dangerous_keys: &[String],
) -> Result<(), AppError> {
    validate_subprocess_env(env, dangerous_keys).map_err(AppError::Mcp)
}

pub(crate) fn validate_subprocess_command_spec(
    command: &str,
    args: &[String],
    blocked: &[String],
    include_default_blocked_commands: bool,
) -> Result<(), String> {
    validate_command_target(command, blocked, include_default_blocked_commands)?;
    let resolved = resolve_command_path(command);
    if let Some(ref resolved) = resolved {
        validate_command_target(
            &resolved.to_string_lossy(),
            blocked,
            include_default_blocked_commands,
        )?;
        if let Some(interpreter) = read_script_interpreter(resolved) {
            validate_command_target(&interpreter, blocked, include_default_blocked_commands)?;
        }
    }

    if uses_env_wrapper(command, resolved.as_deref())
        && let Some(wrapped_command) = extract_env_wrapped_command(args)
    {
        validate_subprocess_command_spec(
            wrapped_command,
            &[],
            blocked,
            include_default_blocked_commands,
        )?;
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

fn is_blocked_command(
    command: &str,
    blocked: &[String],
    include_default_blocked_commands: bool,
) -> bool {
    (include_default_blocked_commands && DEFAULT_BLOCKED_COMMANDS.contains(&command))
        || blocked.iter().any(|candidate| candidate == command)
}

fn is_dangerous_env_key(key: &str, dangerous_keys: &[String]) -> bool {
    DEFAULT_DANGEROUS_ENV_KEYS.contains(&key)
        || dangerous_keys.iter().any(|candidate| candidate == key)
}

fn validate_command_target(
    command: &str,
    blocked: &[String],
    include_default_blocked_commands: bool,
) -> Result<(), String> {
    let basename = Path::new(command)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(command);

    if is_blocked_command(basename, blocked, include_default_blocked_commands) {
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

fn read_script_interpreter(path: &Path) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    let mut reader = std::io::BufReader::new(file);
    let mut first_line = String::new();
    if reader.read_line(&mut first_line).ok()? == 0 {
        return None;
    }

    let shebang = first_line.strip_prefix("#!")?.trim();
    let mut parts = shebang.split_whitespace();
    let interpreter = parts.next()?;

    if Path::new(interpreter)
        .file_name()
        .and_then(|name| name.to_str())
        == Some("env")
    {
        return parts.find(|part| !part.starts_with('-')).map(str::to_owned);
    }

    Some(interpreter.to_owned())
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

fn extract_env_wrapped_command(args: &[String]) -> Option<&str> {
    let mut index = 0usize;
    let mut treat_remaining_as_command = false;

    while index < args.len() {
        let arg = args[index].as_str();
        if treat_remaining_as_command {
            return Some(arg);
        }

        if arg == "--" {
            treat_remaining_as_command = true;
            index += 1;
            continue;
        }

        if arg.contains('=') {
            index += 1;
            continue;
        }

        if let Some(consumed) = env_option_value_arity(arg) {
            index += 1 + consumed;
            continue;
        }

        if arg.starts_with('-') {
            index += 1;
            continue;
        }

        return Some(arg);
    }

    None
}

fn env_option_value_arity(arg: &str) -> Option<usize> {
    match arg {
        "-u" | "--unset" | "-C" | "--chdir" | "-S" | "--split-string" => Some(1),
        _ if arg.starts_with("--unset=")
            || arg.starts_with("--chdir=")
            || arg.starts_with("--split-string=")
            || arg.starts_with("-u")
            || arg.starts_with("-C")
            || arg.starts_with("-S") =>
        {
            Some(0)
        }
        _ if arg.starts_with('-') => Some(0),
        _ => None,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
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
