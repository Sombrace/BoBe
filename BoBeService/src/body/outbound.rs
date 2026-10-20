use std::time::Duration;

use axum::extract::ws::Message;
use tokio::sync::{mpsc, oneshot};

const SEND_TIMEOUT: Duration = Duration::from_secs(2);
const DELIVERY_TIMEOUT: Duration = Duration::from_secs(5);

pub(crate) type OutboundSender = mpsc::Sender<OutboundMessage>;
pub(crate) type OutboundReceiver = mpsc::Receiver<OutboundMessage>;

pub(crate) struct OutboundMessage {
    pub(crate) message: Message,
    pub(crate) completion: Option<oneshot::Sender<bool>>,
}

pub(crate) async fn send_json<T: serde::Serialize>(sender: &OutboundSender, value: &T) -> bool {
    let Ok(text) = serde_json::to_string(value) else {
        return false;
    };
    send_message(sender, Message::Text(text.into())).await
}

pub(crate) async fn send_json_confirmed<T: serde::Serialize>(
    sender: &OutboundSender,
    value: &T,
) -> bool {
    let Ok(text) = serde_json::to_string(value) else {
        return false;
    };
    send_message_confirmed(sender, Message::Text(text.into())).await
}

pub(crate) async fn send_message(sender: &OutboundSender, message: Message) -> bool {
    let outbound = OutboundMessage {
        message,
        completion: None,
    };
    matches!(
        tokio::time::timeout(SEND_TIMEOUT, sender.send(outbound)).await,
        Ok(Ok(()))
    )
}

pub(crate) fn try_send_message(sender: &OutboundSender, message: Message) -> bool {
    sender
        .try_send(OutboundMessage {
            message,
            completion: None,
        })
        .is_ok()
}

pub(crate) async fn send_message_confirmed(sender: &OutboundSender, message: Message) -> bool {
    let (completion, delivered) = oneshot::channel();
    let outbound = OutboundMessage {
        message,
        completion: Some(completion),
    };
    if !matches!(
        tokio::time::timeout(SEND_TIMEOUT, sender.send(outbound)).await,
        Ok(Ok(()))
    ) {
        return false;
    }
    matches!(
        tokio::time::timeout(DELIVERY_TIMEOUT, delivered).await,
        Ok(Ok(true))
    )
}

#[cfg(test)]
mod tests {

    use super::*;

    #[tokio::test]
    async fn confirmed_send_waits_for_writer_delivery() {
        let (sender, mut receiver) = mpsc::channel(1);
        let send = tokio::spawn(async move {
            send_message_confirmed(&sender, Message::Text("terminal".into())).await
        });
        let outbound = receiver.recv().await.expect("outbound message");

        assert!(!send.is_finished());
        outbound
            .completion
            .expect("delivery completion")
            .send(true)
            .expect("sender still waiting");
        assert!(send.await.expect("send task"));
    }
}
