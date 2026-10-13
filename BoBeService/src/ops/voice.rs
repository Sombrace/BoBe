use std::path::PathBuf;
use std::time::Duration;

use clap::Subcommand;
use futures::{SinkExt, StreamExt};
use opusic_c::{Channels, Decoder, SampleRate};
use serde::Serialize;
use serde_json::json;
use tokio::process::Child;
use tokio::time::Instant;
use tokio_tungstenite::tungstenite::Message;

use crate::error::AppError;
use crate::ops::{exit::ExitCode, status::OpsTarget};

mod wav;

#[derive(Subcommand)]
pub(crate) enum VoiceCommands {
    #[command(
        about = "Speak text through the daemon's real voice pipeline; no model-provider account is needed"
    )]
    Preview {
        text: String,
        #[arg(long)]
        voice: Option<String>,
        #[arg(long)]
        speed: Option<f32>,
        #[arg(
            long,
            help = "Save decoded speech as a 24 kHz mono WAV; the file must not already exist"
        )]
        output: PathBuf,
        #[arg(
            long,
            help = "Play the generated WAV using the system audio player, then acknowledge playback"
        )]
        play: bool,
        #[arg(long, value_parser = crate::parse_duration, help = "Cancel this long after the first received audio frame")]
        cancel_after: Option<Duration>,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        token: Option<String>,
    },
}

#[derive(Serialize)]
struct PreviewReport {
    output: PathBuf,
    voice: String,
    speed: f32,
    sample_rate: u32,
    audio_ms: u64,
    frames: u64,
    sentences: Vec<String>,
    first_audio_ms: Option<u64>,
    synthesis_ms: Option<u64>,
    elapsed_ms: u64,
    cancelled: bool,
    playback_completed: bool,
    peak: f32,
}

pub(crate) async fn run(command: VoiceCommands) -> Result<ExitCode, AppError> {
    let VoiceCommands::Preview {
        text,
        voice,
        speed,
        output,
        play,
        cancel_after,
        json,
        url,
        token,
    } = command;
    if text.trim().is_empty() || text.len() > crate::voice::preview::MAX_TEXT_BYTES {
        return Err(AppError::Validation(format!(
            "Preview requires 1–{} bytes of text",
            crate::voice::preview::MAX_TEXT_BYTES
        )));
    }
    if speed.is_some_and(|value| !value.is_finite() || !(0.5..=2.0).contains(&value)) {
        return Err(AppError::Validation(
            "Speech speed must be between 0.5 and 2.0".into(),
        ));
    }
    if cancel_after.is_some_and(|delay| delay > Duration::from_secs(600)) {
        return Err(AppError::Validation(
            "Cancellation delay must not exceed ten minutes".into(),
        ));
    }
    let player = if play { Some(audio_player()?) } else { None };
    let loaded = crate::config::Config::load_operational()?;
    let target = OpsTarget::resolve(&loaded.config, url.as_deref(), token.as_deref())
        .map_err(AppError::Config)?;
    let mut socket = tokio::time::timeout(Duration::from_secs(10), target.voice_socket())
        .await
        .map_err(|_| AppError::Internal("Voice connection timed out".into()))??;
    let mut file = wav::WavOutput::create(&output)?;
    let mut decoder = Decoder::new(Channels::Mono, SampleRate::Hz24000).map_err(ws_error)?;
    socket
        .send(Message::Text(
            json!({"text":text,"voice":voice,"speed":speed,"wait_for_playback":play})
                .to_string()
                .into(),
        ))
        .await
        .map_err(ws_error)?;
    let started = Instant::now();
    let deadline = started + Duration::from_secs(600);
    let mut cancel_at = None;
    let mut playback: Option<Child> = None;
    let mut turn_id = None;
    let mut ended = false;
    let mut report = PreviewReport {
        output,
        voice: String::new(),
        speed: 1.0,
        sample_rate: 24_000,
        audio_ms: 0,
        frames: 0,
        sentences: Vec::new(),
        first_audio_ms: None,
        synthesis_ms: None,
        elapsed_ms: 0,
        cancelled: false,
        playback_completed: false,
        peak: 0.0,
    };
    let result: Result<(), AppError> = async {
        loop {
            tokio::select! {
                biased;
                signal = tokio::signal::ctrl_c(), if !report.cancelled => {
                    signal?;
                    report.cancelled = true;
                    if let Some(child) = playback.as_mut() { child.kill().await?; }
                    playback = None;
                    socket.send(Message::Text(json!({"type":"control","action":"abort"}).to_string().into())).await.map_err(ws_error)?;
                }
                () = tokio::time::sleep_until(cancel_at.unwrap_or(deadline)), if cancel_at.is_some() && !report.cancelled => {
                    report.cancelled = true;
                    if let Some(child) = playback.as_mut() { child.kill().await?; }
                    playback = None;
                    socket.send(Message::Text(json!({"type":"control","action":"abort"}).to_string().into())).await.map_err(ws_error)?;
                }
                status = wait_player(&mut playback), if playback.is_some() => {
                    if !status?.success() { return Err(AppError::Internal("System audio player failed".into())); }
                    playback = None;
                    report.playback_completed = true;
                    socket.send(Message::Text(json!({"type":"tts_playback_complete","turn_id":turn_id}).to_string().into())).await.map_err(ws_error)?;
                }
                () = tokio::time::sleep_until(deadline) => return Err(AppError::Internal("Voice preview timed out".into())),
                frame = socket.next() => match frame {
                    Some(Ok(Message::Binary(bytes))) => {
                        if report.cancelled { continue; }
                        if turn_id.is_none() || ended || bytes.len() <= 9 {
                            return Err(AppError::Internal("Unexpected voice audio frame".into()));
                        }
                        let id = u64::from_be_bytes(bytes[..8].try_into().map_err(ws_error)?);
                        let expected_flags = if report.frames == 0 { crate::speech::protocol::FLAG_FIRST_OF_TURN } else { 0 };
                        if id != report.frames || bytes[8] != expected_flags {
                            return Err(AppError::Internal("Voice frame sequence or flags are invalid".into()));
                        }
                        let mut pcm = [0.0; 480];
                        let samples = decoder.decode_float_to_slice(&bytes[9..], &mut pcm, false).map_err(ws_error)?;
                        if samples != 480 { return Err(AppError::Internal("Expected a 20 ms voice packet".into())); }
                        file.append(&pcm)?;
                        report.peak = pcm.iter().map(|s| s.abs()).fold(report.peak, f32::max);
                        report.frames += 1;
                        report.audio_ms += 20;
                        if report.first_audio_ms.is_none() {
                            report.first_audio_ms = Some(started.elapsed().as_millis() as u64);
                            cancel_at = cancel_after.map(|delay| Instant::now() + delay);
                        }
                    }
                    Some(Ok(Message::Text(text))) => {
                        let event: serde_json::Value = serde_json::from_str(&text)?;
                        match event["type"].as_str() {
                            Some("preview_ready") => {
                                if turn_id.is_some() || event["sample_rate"] != 24_000 { return Err(AppError::Internal("Invalid preview negotiation".into())); }
                                turn_id = event["turn_id"].as_str().map(str::to_owned);
                                event["voice_id"].as_str().unwrap_or_default().clone_into(&mut report.voice);
                                report.speed = event["speed"].as_f64().unwrap_or(1.0) as f32;
                            }
                            Some("tts_text") => {
                                if let Some(text) = event["text"].as_str() { report.sentences.push(text.to_owned()); }
                            }
                            Some("tts_end") => {
                                ended = true;
                                report.synthesis_ms = Some(started.elapsed().as_millis() as u64);
                                file.finish()?;
                                if let Some(player) = &player && !report.cancelled {
                                    playback = Some(tokio::process::Command::new(player).arg(&report.output).kill_on_drop(true).spawn()?);
                                }
                            }
                            Some("truncate") => report.cancelled = true,
                            Some("state") if event["phase"] == "listening" => {
                                if !ended && !report.cancelled { return Err(AppError::Internal("Preview ended before speech completed".into())); }
                                return Ok(());
                            }
                            Some("error") => {
                                let message = event["message"].as_str().unwrap_or("Voice preview failed").to_owned();
                                return Err(match event["code"].as_str() {
                                    Some("invalid_preview" | "invalid_preview_control") => AppError::Validation(message),
                                    Some("voice_busy" | "conflict") => AppError::Conflict(message),
                                    Some("engines_unavailable" | "voice_disabled") => AppError::ServiceUnavailable(message),
                                    _ => AppError::Internal(message),
                                });
                            }
                            _ => {},
                        }
                    }
                    Some(Ok(Message::Ping(_) | Message::Pong(_))) => {},
                    Some(Err(error)) => return Err(ws_error(error)),
                    _ => return Err(AppError::Internal("Voice connection closed before completion".into())),
                }
            }
        }
    }.await;
    if let Some(child) = playback.as_mut() {
        drop(child.kill().await);
    }
    drop(socket.close(None).await);
    result?;
    file.finish()?;
    report.elapsed_ms = started.elapsed().as_millis() as u64;
    let rendered = format!(
        "{} {} ms of speech to {} ({} frames, voice {}).",
        if report.cancelled {
            "Cancelled; saved"
        } else {
            "Saved"
        },
        report.audio_ms,
        report.output.display(),
        report.frames,
        report.voice
    );
    crate::emit_report(json, &report, rendered);
    Ok(ExitCode::Success)
}

fn ws_error(error: impl std::fmt::Debug) -> AppError {
    AppError::Internal(format!("Voice preview: {error:?}"))
}

async fn wait_player(player: &mut Option<Child>) -> std::io::Result<std::process::ExitStatus> {
    match player {
        Some(child) => child.wait().await,
        None => std::future::pending().await,
    }
}

fn audio_player() -> Result<PathBuf, AppError> {
    let candidates: &[&str] = if cfg!(target_os = "macos") {
        &["afplay"]
    } else {
        &["pw-play", "aplay"]
    };
    candidates
        .iter()
        .find_map(|name| which::which(name).ok())
        .ok_or_else(|| {
            AppError::Config("No system audio player found; omit --play to export a WAV".into())
        })
}
