
CREATE TABLE IF NOT EXISTS runtime_authority (
    singleton INTEGER PRIMARY KEY NOT NULL CHECK (singleton = 1),
    authority_id BLOB NOT NULL,
    authority_epoch INTEGER NOT NULL DEFAULT 1 CHECK (authority_epoch >= 1),
    data_root TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    last_incarnation_id BLOB,
    last_started_at TEXT,
    last_daemon_version TEXT
);

CREATE TABLE IF NOT EXISTS runtime_initialization (
    singleton INTEGER PRIMARY KEY NOT NULL CHECK (singleton = 1),
    state TEXT NOT NULL CHECK (state IN (
        'provider_required',
        'personalization_required',
        'ready'
    )),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE TABLE IF NOT EXISTS runtime_markers (
    key TEXT PRIMARY KEY NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE TABLE IF NOT EXISTS bobe_schema (
    singleton INTEGER PRIMARY KEY NOT NULL CHECK (singleton = 1),
    version INTEGER NOT NULL CHECK (version >= 1),
    applied_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    daemon_version TEXT NOT NULL
);


CREATE TABLE IF NOT EXISTS conversations (
    id BLOB PRIMARY KEY NOT NULL,
    channel_id BLOB NOT NULL DEFAULT X'00000000000000000000000000000000',
    state TEXT NOT NULL DEFAULT 'pending',
    closed_at TEXT,
    summary TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
CREATE INDEX IF NOT EXISTS ix_conversations_state ON conversations(state);
CREATE INDEX IF NOT EXISTS ix_conversations_state_closed_at ON conversations(state, closed_at);
CREATE INDEX IF NOT EXISTS ix_conversations_state_updated_at ON conversations(state, updated_at);

CREATE TABLE IF NOT EXISTS conversation_turns (
    id BLOB PRIMARY KEY NOT NULL,
    role TEXT NOT NULL,
    content TEXT NOT NULL,
    is_complete INTEGER NOT NULL DEFAULT 1 CHECK (is_complete IN (0, 1)),
    conversation_id BLOB NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
CREATE INDEX IF NOT EXISTS ix_conversation_turns_conversation_id ON conversation_turns(conversation_id);
CREATE INDEX IF NOT EXISTS ix_conversation_turns_role_created_at ON conversation_turns(role, created_at DESC);
CREATE INDEX IF NOT EXISTS ix_conversation_turns_conversation_role_created_at
    ON conversation_turns(conversation_id, role, created_at DESC);

CREATE TABLE IF NOT EXISTS message_requests (
    request_id BLOB PRIMARY KEY NOT NULL,
    delivery_kind TEXT NOT NULL DEFAULT 'surface',
    owner_input_id BLOB,
    channel_id BLOB NOT NULL DEFAULT X'00000000000000000000000000000000',
    content_sha256 BLOB NOT NULL,
    content TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'queued'
        CHECK (status IN ('queued', 'running', 'completed', 'failed')),
    failure_reason TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
CREATE INDEX IF NOT EXISTS ix_message_requests_created_at ON message_requests(created_at);
CREATE INDEX IF NOT EXISTS ix_message_requests_status_created_at
    ON message_requests(status, created_at);

CREATE TABLE IF NOT EXISTS agent_jobs (
    id BLOB PRIMARY KEY NOT NULL,
    channel_id BLOB DEFAULT X'00000000000000000000000000000000',
    goal_id BLOB,
    schedule_id BLOB,
    delivery_after TEXT,
    source_key TEXT NOT NULL UNIQUE,
    title TEXT NOT NULL,
    task TEXT NOT NULL,
    context TEXT NOT NULL,
    timeout_seconds INTEGER NOT NULL DEFAULT 1800 CHECK (timeout_seconds > 0),
    max_turns INTEGER NOT NULL DEFAULT 64 CHECK (max_turns > 0),
    budget_json TEXT NOT NULL DEFAULT '{}',
    resume_context TEXT NOT NULL DEFAULT '',
    profile TEXT NOT NULL,
    model TEXT,
    reasoning_effort TEXT,
    status TEXT NOT NULL DEFAULT 'queued' CHECK (status IN (
        'queued', 'running', 'cancel_requested', 'completed', 'failed', 'cancelled', 'interrupted', 'awaiting_input'
    )),
    attempt INTEGER NOT NULL DEFAULT 1 CHECK (attempt >= 1),
    session_id TEXT,
    output TEXT,
    error TEXT,
    delivery_id BLOB NOT NULL UNIQUE,
    delivery_status TEXT NOT NULL DEFAULT 'pending' CHECK (delivery_status IN (
        'pending', 'delivering', 'committed', 'presented', 'interrupted', 'suppressed'
    )),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
CREATE INDEX IF NOT EXISTS ix_agent_jobs_status_created_at ON agent_jobs(status, created_at);
CREATE INDEX IF NOT EXISTS ix_agent_jobs_delivery ON agent_jobs(delivery_status, status);

CREATE TABLE IF NOT EXISTS agent_schedules (
    id BLOB PRIMARY KEY NOT NULL,
    source_key TEXT NOT NULL UNIQUE,
    title TEXT NOT NULL,
    task TEXT NOT NULL,
    context TEXT NOT NULL,
    channel_id BLOB NOT NULL,
    goal_id BLOB,
    next_run_at TEXT NOT NULL,
    every_seconds INTEGER CHECK (every_seconds > 0),
    enabled INTEGER NOT NULL DEFAULT 1,
    generation INTEGER NOT NULL DEFAULT 1,
    last_job_id BLOB,
    last_error TEXT,
    budget_json TEXT NOT NULL DEFAULT '{}',
    definition_json TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE INDEX IF NOT EXISTS ix_agent_schedules_due ON agent_schedules(enabled,next_run_at);

CREATE TABLE IF NOT EXISTS agent_work_tombstones (
    source_key TEXT PRIMARY KEY NOT NULL,
    retired_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

CREATE TABLE IF NOT EXISTS runtime_cadences (
    key TEXT PRIMARY KEY NOT NULL,
    next_due_at TEXT NOT NULL,
    interval_seconds INTEGER NOT NULL CHECK (interval_seconds > 0)
);

CREATE TABLE IF NOT EXISTS owner_inputs (
    id BLOB PRIMARY KEY NOT NULL,
    source_key TEXT NOT NULL UNIQUE,
    origin_run_id BLOB NOT NULL,
    origin_settled INTEGER NOT NULL DEFAULT 0,
    job_id BLOB,
    resumed_job_id BLOB,
    channel_id BLOB NOT NULL,
    goal_id BLOB,
    kind TEXT NOT NULL CHECK (kind IN ('approval','information')),
    question TEXT NOT NULL,
    action TEXT NOT NULL,
    prompt TEXT NOT NULL,
    choices_json TEXT NOT NULL,
    definition_json TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending','answered','declined','cancelled','expired')),
    expires_at TEXT NOT NULL,
    response TEXT,
    response_id BLOB,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE INDEX IF NOT EXISTS ix_owner_inputs_pending ON owner_inputs(status,expires_at);

CREATE TABLE IF NOT EXISTS owner_input_presentations (
    input_id BLOB NOT NULL REFERENCES owner_inputs(id) ON DELETE CASCADE,
    channel_id BLOB NOT NULL,
    turn_id BLOB,
    presented_at TEXT,
    PRIMARY KEY (input_id,channel_id)
);

CREATE TABLE IF NOT EXISTS conversation_presentations (
    turn_id BLOB NOT NULL REFERENCES conversation_turns(id) ON DELETE CASCADE,
    channel_id BLOB NOT NULL,
    presented_at TEXT NOT NULL,
    PRIMARY KEY (turn_id,channel_id)
);

CREATE TABLE IF NOT EXISTS message_request_tombstones (
    request_id BLOB PRIMARY KEY NOT NULL,
    retired_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE TABLE IF NOT EXISTS souls (
    id BLOB PRIMARY KEY NOT NULL,
    name TEXT NOT NULL UNIQUE,
    content TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    is_default INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
CREATE INDEX IF NOT EXISTS ix_souls_name ON souls(name);

CREATE TABLE IF NOT EXISTS user_profiles (
    id BLOB PRIMARY KEY NOT NULL,
    name TEXT NOT NULL UNIQUE,
    content TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    is_default INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
CREATE INDEX IF NOT EXISTS ix_user_profiles_name ON user_profiles(name);

CREATE TABLE IF NOT EXISTS endpoints (
    id TEXT PRIMARY KEY NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('physical_body', 'mac_app', 'other')),
    label TEXT NOT NULL,
    room TEXT,
    certificate_sha256 BLOB UNIQUE,
    protocol_min INTEGER NOT NULL DEFAULT 1 CHECK (protocol_min >= 1),
    protocol_max INTEGER NOT NULL DEFAULT 1 CHECK (protocol_max >= protocol_min),
    capabilities_json TEXT NOT NULL DEFAULT '{}',
    observed_capabilities_json TEXT NOT NULL DEFAULT '{}',
    revoked_at TEXT,
    revocation_reason TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
CREATE INDEX IF NOT EXISTS ix_endpoints_kind_revoked_at ON endpoints(kind, revoked_at);
CREATE UNIQUE INDEX IF NOT EXISTS ix_endpoints_certificate_active
    ON endpoints(certificate_sha256) WHERE revoked_at IS NULL;

CREATE TABLE IF NOT EXISTS endpoint_credentials (
    id TEXT PRIMARY KEY NOT NULL,
    endpoint_id TEXT NOT NULL REFERENCES endpoints(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('bearer', 'body_certificate', 'public_key')),
    lookup_id TEXT UNIQUE,
    verifier BLOB,
    certificate_sha256 BLOB UNIQUE,
    public_key BLOB,
    issued_authority_epoch INTEGER CHECK (issued_authority_epoch >= 1),
    generation INTEGER NOT NULL DEFAULT 1 CHECK (generation >= 1),
    not_before TEXT,
    expires_at TEXT,
    last_used_at TEXT,
    revoked_at TEXT,
    revocation_reason TEXT,
    replaced_by TEXT REFERENCES endpoint_credentials(id),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    CHECK (
        (kind = 'bearer' AND lookup_id IS NOT NULL AND verifier IS NOT NULL)
        OR (kind = 'body_certificate' AND certificate_sha256 IS NOT NULL)
        OR (kind = 'public_key' AND public_key IS NOT NULL)
    )
);
CREATE INDEX IF NOT EXISTS ix_endpoint_credentials_endpoint_active
    ON endpoint_credentials(endpoint_id, revoked_at);
CREATE UNIQUE INDEX IF NOT EXISTS ix_endpoint_credentials_generation
    ON endpoint_credentials(endpoint_id, kind, generation);

CREATE TABLE IF NOT EXISTS endpoint_scope_grants (
    endpoint_id TEXT NOT NULL REFERENCES endpoints(id) ON DELETE CASCADE,
    scope TEXT NOT NULL,
    granted_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    granted_by TEXT NOT NULL,
    revoked_at TEXT,
    PRIMARY KEY (endpoint_id, scope, granted_at)
);
CREATE INDEX IF NOT EXISTS ix_endpoint_scope_grants_active
    ON endpoint_scope_grants(endpoint_id, scope) WHERE revoked_at IS NULL;

CREATE TABLE IF NOT EXISTS security_audit_events (
    id BLOB PRIMARY KEY NOT NULL,
    event_kind TEXT NOT NULL,
    endpoint_id TEXT REFERENCES endpoints(id) ON DELETE SET NULL,
    credential_id TEXT REFERENCES endpoint_credentials(id) ON DELETE SET NULL,
    source_summary TEXT,
    details_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
CREATE INDEX IF NOT EXISTS ix_security_audit_events_created_at
    ON security_audit_events(created_at DESC);
CREATE INDEX IF NOT EXISTS ix_security_audit_events_endpoint_created_at
    ON security_audit_events(endpoint_id, created_at DESC);

CREATE TABLE IF NOT EXISTS cooldown_state (
    id BLOB PRIMARY KEY NOT NULL,
    last_engagement TEXT,
    last_user_response TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
