use std::collections::HashMap;
use std::time::Duration;

use tokio::sync::Mutex;
use tokio::time::Instant;

const WINDOW: Duration = Duration::from_millis(150);

#[derive(Clone, Debug)]
pub(crate) struct WakeCandidate {
    pub(crate) endpoint_id: String,
    pub(crate) request_id: String,
    pub(crate) confidence: f32,
    pub(crate) snr_db: f32,
    pub(crate) energy_dbfs: f32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum WakeDecision {
    Winner,
    Lost,
    ExplicitCapturePreempted,
}

struct Round {
    id: u64,
    deadline: Instant,
    candidates: HashMap<String, WakeCandidate>,
    winner: Option<String>,
    preempted: bool,
}

#[derive(Default)]
pub(crate) struct WakeArbitrator {
    round: Mutex<Option<Round>>,
    next_round_id: std::sync::atomic::AtomicU64,
}

impl WakeArbitrator {
    pub(crate) async fn submit(&self, candidate: WakeCandidate) -> WakeDecision {
        let key = candidate_key(&candidate);
        let (round_id, deadline) = {
            let mut state = self.round.lock().await;
            let now = Instant::now();
            if state
                .as_ref()
                .is_none_or(|round| round.deadline <= now || round.preempted)
            {
                let id = self
                    .next_round_id
                    .fetch_add(1, std::sync::atomic::Ordering::AcqRel)
                    .wrapping_add(1);
                *state = Some(Round {
                    id,
                    deadline: now + WINDOW,
                    candidates: HashMap::new(),
                    winner: None,
                    preempted: false,
                });
            }
            let Some(round) = state.as_mut() else {
                return WakeDecision::Lost;
            };
            round.candidates.insert(key.clone(), candidate);
            (round.id, round.deadline)
        };
        tokio::time::sleep_until(deadline).await;
        let mut state = self.round.lock().await;
        let Some(round) = state.as_mut() else {
            return WakeDecision::Lost;
        };
        if round.id != round_id {
            return WakeDecision::Lost;
        }
        if round.preempted {
            return WakeDecision::ExplicitCapturePreempted;
        }
        let winner = round.winner.get_or_insert_with(|| {
            round
                .candidates
                .values()
                .max_by(|left, right| compare_candidates(left, right))
                .map(candidate_key)
                .unwrap_or_default()
        });
        if winner == &key {
            WakeDecision::Winner
        } else {
            WakeDecision::Lost
        }
    }

    pub(crate) async fn preempt_pending(&self) {
        if let Some(round) = self.round.lock().await.as_mut()
            && round.deadline > Instant::now()
        {
            round.preempted = true;
        }
    }
}

fn candidate_key(candidate: &WakeCandidate) -> String {
    format!("{}:{}", candidate.endpoint_id, candidate.request_id)
}

fn compare_candidates(left: &WakeCandidate, right: &WakeCandidate) -> std::cmp::Ordering {
    left.confidence
        .total_cmp(&right.confidence)
        .then_with(|| left.snr_db.total_cmp(&right.snr_db))
        .then_with(|| left.energy_dbfs.total_cmp(&right.energy_dbfs))
        .then_with(|| right.endpoint_id.cmp(&left.endpoint_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(endpoint: &str, confidence: f32) -> WakeCandidate {
        WakeCandidate {
            endpoint_id: endpoint.into(),
            request_id: endpoint.into(),
            confidence,
            snr_db: 10.0,
            energy_dbfs: -20.0,
        }
    }

    #[tokio::test]
    async fn highest_confidence_wins_one_window() {
        let arbiter = std::sync::Arc::new(WakeArbitrator::default());
        let first = {
            let arbiter = std::sync::Arc::clone(&arbiter);
            tokio::spawn(async move { arbiter.submit(candidate("a", 0.7)).await })
        };
        tokio::time::sleep(Duration::from_millis(10)).await;
        let second = {
            let arbiter = std::sync::Arc::clone(&arbiter);
            tokio::spawn(async move { arbiter.submit(candidate("b", 0.9)).await })
        };
        assert_eq!(first.await.unwrap(), WakeDecision::Lost);
        assert_eq!(second.await.unwrap(), WakeDecision::Winner);
    }

    #[tokio::test]
    async fn explicit_capture_preempts_pending_window() {
        let arbiter = std::sync::Arc::new(WakeArbitrator::default());
        let pending = {
            let arbiter = std::sync::Arc::clone(&arbiter);
            tokio::spawn(async move { arbiter.submit(candidate("a", 1.0)).await })
        };
        tokio::time::sleep(Duration::from_millis(10)).await;
        arbiter.preempt_pending().await;
        assert_eq!(
            pending.await.unwrap(),
            WakeDecision::ExplicitCapturePreempted
        );
    }
}
