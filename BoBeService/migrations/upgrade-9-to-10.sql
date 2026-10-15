DROP TABLE IF EXISTS security_audit_events;

ALTER TABLE runtime_authority DROP COLUMN last_incarnation_id;
ALTER TABLE runtime_authority DROP COLUMN last_started_at;
ALTER TABLE runtime_authority DROP COLUMN last_daemon_version;
ALTER TABLE runtime_initialization DROP COLUMN updated_at;
ALTER TABLE runtime_markers DROP COLUMN created_at;
ALTER TABLE bobe_schema DROP COLUMN applied_at;
ALTER TABLE bobe_schema DROP COLUMN daemon_version;
ALTER TABLE conversations DROP COLUMN summary;
ALTER TABLE message_requests DROP COLUMN failure_reason;
ALTER TABLE agent_work_tombstones DROP COLUMN retired_at;
ALTER TABLE message_request_tombstones DROP COLUMN retired_at;
ALTER TABLE endpoints DROP COLUMN revocation_reason;
ALTER TABLE endpoint_scope_grants DROP COLUMN granted_by;

DROP INDEX IF EXISTS ix_conversations_state;
DROP INDEX IF EXISTS ix_conversation_turns_conversation_id;
DROP INDEX IF EXISTS ix_souls_name;
DROP INDEX IF EXISTS ix_user_profiles_name;
DROP INDEX IF EXISTS ix_endpoints_certificate_active;

CREATE TABLE endpoint_credentials_v10 (
    id TEXT PRIMARY KEY NOT NULL,
    endpoint_id TEXT NOT NULL REFERENCES endpoints(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('bearer', 'body_certificate')),
    lookup_id TEXT UNIQUE,
    verifier BLOB,
    certificate_sha256 BLOB UNIQUE,
    issued_authority_epoch INTEGER CHECK (issued_authority_epoch >= 1),
    generation INTEGER NOT NULL DEFAULT 1 CHECK (generation >= 1),
    last_used_at TEXT,
    revoked_at TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    CHECK (
        (kind = 'bearer' AND lookup_id IS NOT NULL AND verifier IS NOT NULL)
        OR (kind = 'body_certificate' AND certificate_sha256 IS NOT NULL)
    )
);
INSERT INTO endpoint_credentials_v10 (
    rowid, id, endpoint_id, kind, lookup_id, verifier, certificate_sha256,
    issued_authority_epoch, generation, last_used_at, revoked_at, created_at
)
SELECT
    rowid, id, endpoint_id, kind, lookup_id, verifier, certificate_sha256,
    issued_authority_epoch, generation, last_used_at, revoked_at, created_at
FROM endpoint_credentials
WHERE kind IN ('bearer', 'body_certificate');
DROP TABLE endpoint_credentials;
ALTER TABLE endpoint_credentials_v10 RENAME TO endpoint_credentials;
CREATE INDEX ix_endpoint_credentials_endpoint_active
    ON endpoint_credentials(endpoint_id, revoked_at);
CREATE UNIQUE INDEX ix_endpoint_credentials_generation
    ON endpoint_credentials(endpoint_id, kind, generation);

DELETE FROM user_profiles
WHERE is_default = 1 AND content = '# User Profile

This document describes you, the user of BoBe. BoBe uses this information to provide personalized assistance.

## About You

*Edit this section to describe yourself - your role, work context, preferences, etc.*

- **Name**: [Your name]
- **Role**: [Your job title or role]
- **Work Context**: [What you''re working on, your team, your projects]

## Preferences

*What are your communication preferences? How do you like to receive help?*

- **Communication Style**: [Direct, detailed, casual, formal]
- **Proactive Help**: [When should BoBe reach out? What triggers would be helpful?]
- **Topics of Interest**: [Areas where you''d like more context or assistance]

## Current Focus

*What are you currently working on? What goals are top of mind?*

- [Current project or focus area]
- [Key deadlines or milestones]
- [Challenges you''re facing]

## Notes

*Any other context that would help BoBe assist you better.*
';
UPDATE user_profiles SET is_default = 0 WHERE is_default = 1;
