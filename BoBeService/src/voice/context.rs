use std::sync::Arc;

use tokio::sync::mpsc;

use crate::runtime::response_streamer::StreamDelivery;
use crate::runtime::session::RuntimeSession;
use crate::voice::engines::VoiceEngines;
use crate::voice::output::VoiceOutput;
use crate::voice::session::VoiceDefaults;
use crate::voice::sinks::VoiceSink;

#[derive(Clone)]
pub(crate) struct VoiceContext {
    pub(crate) conversation_channel: crate::models::ids::ConversationChannelId,
    pub(crate) networked_runtime: bool,
    pub(crate) output: VoiceOutput,
    pub(crate) runtime_session: Arc<RuntimeSession>,
    pub(crate) engines: VoiceEngines,
    pub(crate) voice_defaults: VoiceDefaults,
    pub(crate) voice_sink: Arc<VoiceSink>,
    pub(crate) delivery: StreamDelivery,
    pub(crate) turn_completion_tx: mpsc::Sender<String>,
}
