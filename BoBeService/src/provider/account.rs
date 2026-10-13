#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct ProviderAuthStatus {
    pub(crate) is_authenticated: bool,
    pub(crate) auth_type: Option<String>,
    pub(crate) host: Option<String>,
    pub(crate) login: Option<String>,
    pub(crate) status_message: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct ProviderModelDescriptor {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) vision: bool,
    pub(crate) context_window: Option<i64>,
    pub(crate) default_reasoning_effort: Option<String>,
    pub(crate) supported_reasoning_efforts: Vec<String>,
}
