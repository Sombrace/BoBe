use std::sync::Arc;
use tokio::sync::mpsc;

use crate::speech::protocol::ServerMessage;

#[derive(Debug, Clone)]
pub(crate) enum VoiceOutputFrame {
    Control(ServerMessage),
    Audio(Vec<u8>),
    Ping,
}

#[derive(Clone)]
pub(crate) struct VoiceOutput {
    sender: mpsc::Sender<VoiceOutputFrame>,
    audio_sequence: Arc<tokio::sync::Mutex<()>>,
}

impl VoiceOutput {
    pub(crate) fn channel(capacity: usize) -> (Self, mpsc::Receiver<VoiceOutputFrame>) {
        let (sender, receiver) = mpsc::channel(capacity);
        (
            Self {
                sender,
                audio_sequence: Arc::new(tokio::sync::Mutex::new(())),
            },
            receiver,
        )
    }

    pub(crate) async fn audio_sequence(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.audio_sequence.lock().await
    }

    pub(crate) fn try_audio_sequence(&self) -> Option<tokio::sync::MutexGuard<'_, ()>> {
        self.audio_sequence.try_lock().ok()
    }

    pub(crate) async fn send(&self, frame: VoiceOutputFrame) -> bool {
        self.sender.send(frame).await.is_ok()
    }

    pub(crate) async fn control(&self, message: ServerMessage) -> bool {
        self.send(VoiceOutputFrame::Control(message)).await
    }

    pub(crate) async fn audio(&self, bytes: Vec<u8>) -> bool {
        self.send(VoiceOutputFrame::Audio(bytes)).await
    }

    pub(crate) async fn ping(&self) -> bool {
        self.send(VoiceOutputFrame::Ping).await
    }
}
