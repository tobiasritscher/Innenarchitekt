-- Projects: current snapshot (plan_core::Project as JSONB) + monotonically increasing version.
CREATE TABLE projects (
    id          uuid PRIMARY KEY,
    name        text NOT NULL,
    snapshot    jsonb NOT NULL,
    version     bigint NOT NULL DEFAULT 0,
    created_at  timestamptz NOT NULL DEFAULT now(),
    updated_at  timestamptz NOT NULL DEFAULT now()
);

-- Op log: one row per applied batch.
CREATE TABLE project_ops (
    id          bigserial PRIMARY KEY,
    project_id  uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    version     bigint NOT NULL,
    ops         jsonb NOT NULL,
    created_at  timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX project_ops_project_version ON project_ops (project_id, version);

-- Background jobs (queue via FOR UPDATE SKIP LOCKED).
CREATE TABLE jobs (
    id          uuid PRIMARY KEY,
    kind        text NOT NULL,
    status      text NOT NULL DEFAULT 'queued',
    progress    real NOT NULL DEFAULT 0,
    message     text NOT NULL DEFAULT '',
    payload     jsonb NOT NULL DEFAULT '{}'::jsonb,
    result      jsonb,
    error       text,
    created_at  timestamptz NOT NULL DEFAULT now(),
    updated_at  timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX jobs_queued ON jobs (created_at) WHERE status = 'queued';

-- Furniture library (data = plan_core::assets::Asset).
CREATE TABLE assets (
    id            uuid PRIMARY KEY,
    kind          text NOT NULL,
    name          text NOT NULL,
    data          jsonb NOT NULL,
    ikea_article  text UNIQUE,
    created_at    timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX assets_created ON assets (created_at DESC);
