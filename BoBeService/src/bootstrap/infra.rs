use std::sync::Arc;

use arc_swap::ArcSwap;

use crate::config::Config;
use crate::error::AppError;
use crate::util::network::MdnsAnnouncer;
use crate::util::sse::connection_manager::SseConnectionManager;
use crate::util::sse::event_queue::EventQueue;

pub(crate) struct Infrastructure {
    pub(crate) config_arc: Arc<ArcSwap<Config>>,
    pub(crate) config_write_lock: Arc<std::sync::Mutex<()>>,
    pub(crate) event_queue: Arc<EventQueue>,
    pub(crate) connection_manager: Arc<SseConnectionManager>,
    pub(crate) mdns_announcer: Arc<MdnsAnnouncer>,
}

impl Infrastructure {
    pub(crate) fn build(
        config: &Config,
        runtime_incarnation_id: uuid::Uuid,
    ) -> Result<Self, AppError> {
        let config_arc = Arc::new(ArcSwap::from_pointee(config.clone()));

        let event_queue = Arc::new(EventQueue::with_incarnation(256, runtime_incarnation_id));
        let connection_manager = Arc::new(SseConnectionManager::new());

        let mdns_announcer = Arc::new(MdnsAnnouncer::new(
            config.server.port,
            config.server.mdns_enabled && config.server.host == "0.0.0.0",
        ));

        Ok(Self {
            config_arc,
            config_write_lock: Arc::new(std::sync::Mutex::new(())),
            event_queue,
            connection_manager,
            mdns_announcer,
        })
    }
}
