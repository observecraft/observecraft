-- Assist (AI chat): conversations, each with the model's provider-neutral messages (assist::llm::Msg)
-- and the tool calls waiting for the user's approval; per-user settings; and one usage row per
-- message for Assist → Reports.
CREATE TABLE IF NOT EXISTS assist_conversations (
    org_id     TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    id         UUID NOT NULL DEFAULT gen_random_uuid(),
    user_id    UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    title      TEXT NOT NULL,
    -- Where it was started: chat (Assist Chat) or panel (Ask Assist on any page).
    surface    TEXT NOT NULL DEFAULT 'chat',
    messages   JSONB NOT NULL DEFAULT '[]',
    pending    JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (org_id, id)
);
CREATE INDEX IF NOT EXISTS assist_conversations_user ON assist_conversations (org_id, user_id, updated_at DESC);

CREATE TABLE IF NOT EXISTS assist_settings (
    org_id          TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    user_id         UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    -- Interests, preferences or instructions the model keeps in mind.
    context         TEXT NOT NULL DEFAULT '',
    disabled_skills TEXT[] NOT NULL DEFAULT '{}',
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (org_id, user_id)
);

CREATE TABLE IF NOT EXISTS assist_usage (
    org_id          TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    user_id         UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    conversation_id UUID NOT NULL,
    at              TIMESTAMPTZ NOT NULL DEFAULT now(),
    surface         TEXT NOT NULL,
    -- 1 for the message that started the conversation.
    new_conversation BOOLEAN NOT NULL DEFAULT false,
    input_tokens    BIGINT NOT NULL DEFAULT 0,
    output_tokens   BIGINT NOT NULL DEFAULT 0,
    tool_calls      INT NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS assist_usage_at ON assist_usage (org_id, at);
