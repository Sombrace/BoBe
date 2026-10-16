use std::time::Instant;

use tracing::{debug, info};

use crate::speech::protocol::{ServerMessage, VoicePhase};
use crate::voice::context::VoiceContext;
use crate::voice::protocol_helpers::{send_json, send_state};
use crate::voice::session::{AbortedTurn, PendingBargeIn, TurnInFlight, VoiceSession};

pub(crate) const MIN_WORDS_FOR_BARGE_IN: usize = 2;
const MIN_UNSPACED_CHARS_FOR_BARGE_IN: usize = 4;
const MAX_ACKNOWLEDGEMENT_WORDS: usize = 4;

const ACKNOWLEDGEMENTS: &[&[&str]] = &[
    &["mm", "hmm"],
    &["uh", "huh"],
    &["i", "see"],
    &["got", "it"],
    &["all", "right"],
    &["thank", "you"],
    &["makes", "sense"],
    &["of", "course"],
    &["yeah"],
    &["yes"],
    &["yep"],
    &["yup"],
    &["ok"],
    &["okay"],
    &["right"],
    &["sure"],
    &["mhm"],
    &["mhmm"],
    &["mmhmm"],
    &["uhhuh"],
    &["mm"],
    &["hmm"],
    &["hm"],
    &["oh"],
    &["ah"],
    &["cool"],
    &["nice"],
    &["great"],
    &["true"],
    &["exactly"],
    &["alright"],
    &["thanks"],
];

const INTERRUPTION_OPENERS: &[&str] = &[
    "wait", "no", "nope", "but", "actually", "hold", "hang", "sorry", "excuse", "hey", "listen",
    "bobe",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BargeIn {
    Interrupt,
    Acknowledgement,
    Unclear,
}

fn spoken_words(partial: &str) -> Vec<String> {
    partial
        .split(|c: char| c.is_whitespace() || matches!(c, '-' | '\u{2013}' | '\u{2014}'))
        .map(|word| {
            word.chars()
                .filter(|c| c.is_alphanumeric())
                .flat_map(char::to_lowercase)
                .collect::<String>()
        })
        .filter(|word| !word.is_empty())
        .collect()
}

fn is_acknowledgement(words: &[String]) -> bool {
    if words.is_empty() || words.len() > MAX_ACKNOWLEDGEMENT_WORDS {
        return false;
    }
    let mut rest = words;
    while !rest.is_empty() {
        let Some(phrase) = ACKNOWLEDGEMENTS.iter().find(|phrase| {
            rest.len() >= phrase.len() && rest.iter().zip(phrase.iter()).all(|(a, b)| a == b)
        }) else {
            return false;
        };
        rest = &rest[phrase.len()..];
    }
    true
}

fn is_unspaced_script(c: char) -> bool {
    matches!(
        c as u32,
        0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF
    )
}

pub(crate) fn judge_barge_in(partial: &str) -> BargeIn {
    let words = spoken_words(partial);
    if is_acknowledgement(&words) {
        return BargeIn::Acknowledgement;
    }
    if words
        .first()
        .is_some_and(|word| INTERRUPTION_OPENERS.contains(&word.as_str()))
    {
        return BargeIn::Interrupt;
    }
    if words.len() >= MIN_WORDS_FOR_BARGE_IN {
        return BargeIn::Interrupt;
    }
    let unspaced = partial.chars().filter(|c| is_unspaced_script(*c)).count();
    if unspaced >= MIN_UNSPACED_CHARS_FOR_BARGE_IN {
        return BargeIn::Interrupt;
    }
    BargeIn::Unclear
}

pub(crate) async fn abort_active_turn(
    s: &mut VoiceSession,
    ctx: &VoiceContext,
    keep_ms: u64,
    reason: &'static str,
) -> bool {
    let Some(AbortedTurn { turn, overlap }) = s.abort_turn() else {
        debug!(reason, "voice.abort_no_active_turn");
        return false;
    };
    info!(
        turn_id = %turn.turn_id,
        keep_ms,
        reason,
        "voice.abort_active_turn"
    );
    let TurnInFlight {
        turn_id,
        join,
        script,
        ..
    } = turn;
    join.abort();
    drop(join.await);
    if let Ok(turn) = crate::models::ids::conversation_turn_id_from_wire_id(&turn_id) {
        ctx.runtime_session.settle_interrupted_turn(turn).await;
    }
    if let Some(reply) = overlap
        .map(|overlap| overlap.playhead_chunk)
        .and_then(|chunk| script.heard_until(chunk))
    {
        info!(
            turn_id = %turn_id,
            heard_chars = reply.heard.chars().count(),
            cut_off = reply.cut_off.is_some(),
            "voice.interrupted_reply_noted"
        );
        ctx.runtime_session
            .note_interrupted_reply(ctx.conversation_channel, reply);
    }
    send_json(
        &ctx.output,
        &ServerMessage::Truncate {
            turn_id: turn_id.clone(),
            keep_ms,
        },
    )
    .await;
    send_state(&ctx.output, VoicePhase::Listening, &turn_id).await;
    true
}

pub(crate) fn pending_interruption(s: &VoiceSession, text: &str) -> Option<u64> {
    let pending = s.pending_barge_in?;
    if s.current_turn.is_none() || !pending.is_live(Instant::now()) {
        return None;
    }
    (judge_barge_in(text) == BargeIn::Interrupt)
        .then(|| pending.played_ms.max(s.last_acked_played_ms))
}

pub(crate) async fn handle_barge_in(
    ctx: &VoiceContext,
    session: &mut Option<VoiceSession>,
    played_ms: u64,
    playhead_chunk: u64,
) {
    let Some(s) = session.as_mut() else { return };
    if s.current_turn.is_none() {
        debug!("voice.barge_in_no_turn");
        return;
    }
    s.pending_barge_in = Some(PendingBargeIn {
        heard_at: Instant::now(),
        played_ms,
        playhead_chunk,
    });
    let verdict = judge_barge_in(&s.last_partial_text);
    if verdict != BargeIn::Interrupt {
        debug!(
            ?verdict,
            words = s.last_partial_text.split_whitespace().count(),
            "voice.barge_in_held"
        );
        return;
    }
    let keep_ms = played_ms.max(s.last_acked_played_ms);
    abort_active_turn(s, ctx, keep_ms, "barge_in").await;
}

#[cfg(test)]
mod tests {
    use super::{BargeIn, judge_barge_in};

    #[test]
    fn short_agreement_does_not_take_the_floor() {
        for partial in [
            "yeah",
            "Mm-hmm.",
            "uh huh",
            "okay",
            "oh, I see",
            "got it",
            "Right, right.",
        ] {
            assert_eq!(
                judge_barge_in(partial),
                BargeIn::Acknowledgement,
                "{partial}"
            );
        }
    }

    #[test]
    fn real_speech_takes_the_floor() {
        for partial in [
            "please stop now",
            "what about",
            "tell me more",
            "okay but why",
        ] {
            assert_eq!(judge_barge_in(partial), BargeIn::Interrupt, "{partial}");
        }
    }

    #[test]
    fn disagreement_and_redirection_need_only_one_word() {
        for partial in ["wait", "No.", "actually", "hold on", "Sorry"] {
            assert_eq!(judge_barge_in(partial), BargeIn::Interrupt, "{partial}");
        }
    }

    #[test]
    fn noise_and_single_words_stay_unclear() {
        for partial in ["", "   ", "the", "weather"] {
            assert_eq!(judge_barge_in(partial), BargeIn::Unclear, "{partial:?}");
        }
    }

    #[test]
    fn unspaced_scripts_count_characters() {
        assert_eq!(judge_barge_in("ちょっと待って"), BargeIn::Interrupt);
        assert_eq!(judge_barge_in("等一下好吗"), BargeIn::Interrupt);
        assert_eq!(judge_barge_in("はい"), BargeIn::Unclear);
    }
}
