-- 本地题库初始结构（v1）
-- 约定：题干指纹 stem_normalized、软删除 deleted_at、同步状态 sync_state 与中心系统一致。
-- 回滚：drop table 顺序见文件末尾注释。

create table if not exists meta (
    key   text primary key,
    value text not null
);

create table if not exists subject (
    id          text primary key,
    code        text,
    name        text not null,
    sort        integer not null default 0,
    external_id text
);

create table if not exists textbook (
    id          text primary key,
    subject_id  text not null references subject (id) on delete cascade,
    name        text not null,
    grade       text,
    sort        integer not null default 0,
    external_id text
);

create table if not exists chapter (
    id          text primary key,
    textbook_id text not null references textbook (id) on delete cascade,
    parent_id   text references chapter (id) on delete cascade,
    name        text not null,
    sort        integer not null default 0,
    external_id text
);

create table if not exists knowledge_point (
    id          text primary key,
    subject_id  text not null references subject (id) on delete cascade,
    chapter_id  text references chapter (id) on delete set null,
    code        text,
    name        text not null,
    summary     text,
    sort        integer not null default 0,
    external_id text,
    created_at  text not null default (datetime('now'))
);

create table if not exists exam_point (
    id          text primary key,
    subject_id  text not null references subject (id) on delete cascade,
    name        text not null,
    external_id text,
    unique (subject_id, name)
);

create table if not exists question (
    id              text primary key,
    subject_id      text references subject (id) on delete set null,
    type            text not null,
    stem            text not null,
    options_json    text not null default '[]',
    answer          text not null default '',
    analysis        text not null default '',
    difficulty      integer not null default 3,
    stem_normalized text not null default '',
    source          text,
    year            integer,
    status          text not null default 'DRAFT',
    origin          text not null default 'MANUAL',
    needs_review    integer not null default 0,
    ai_answer       integer not null default 0,
    ai_analysis     integer not null default 0,
    confidence      real,
    review_note     text,
    created_at      text not null default (datetime('now')),
    updated_at      text not null default (datetime('now')),
    deleted_at      text,
    sync_state      text not null default 'LOCAL',
    server_id       text
);

create index if not exists ix_question_updated on question (updated_at desc);
create index if not exists ix_question_subject on question (subject_id, deleted_at);
create index if not exists ix_question_stem on question (stem_normalized);
create index if not exists ix_question_state on question (sync_state);

create table if not exists question_tag (
    question_id text not null references question (id) on delete cascade,
    tag         text not null,
    kind        text not null default 'FREE',
    primary key (question_id, tag, kind)
);

create index if not exists ix_question_tag_tag on question_tag (tag, kind);

create table if not exists question_knowledge (
    question_id       text not null references question (id) on delete cascade,
    knowledge_point_id text not null references knowledge_point (id) on delete cascade,
    weight            real not null default 1.0,
    primary key (question_id, knowledge_point_id)
);

create table if not exists question_asset (
    id            text primary key,
    question_id   text not null references question (id) on delete cascade,
    kind          text not null default 'FIGURE',
    relative_path text not null,
    content_type  text not null,
    sha256        text not null,
    width         integer,
    height        integer,
    page_no       integer,
    bbox_json     text,
    sort          integer not null default 0,
    created_at    text not null default (datetime('now'))
);

create index if not exists ix_asset_question on question_asset (question_id, sort);

create table if not exists import_batch (
    id            text primary key,
    filename      text not null,
    source_format text not null,
    total         integer not null default 0,
    imported      integer not null default 0,
    skipped       integer not null default 0,
    failed        integer not null default 0,
    status        text not null default 'PREVIEW',
    created_at    text not null default (datetime('now'))
);

create table if not exists import_error (
    id       text primary key,
    batch_id text not null references import_batch (id) on delete cascade,
    row_no   integer not null,
    code     text not null,
    message  text not null,
    raw_json text
);

-- 批次 ↔ 题目：撤销时按批次软删除，不靠时间或文件名猜。
create table if not exists import_batch_question (
    batch_id    text not null references import_batch (id) on delete cascade,
    question_id text not null references question (id) on delete cascade,
    primary key (batch_id, question_id)
);

create table if not exists generation_job (
    id          text primary key,
    kind        text not null,
    status      text not null default 'RUNNING',
    input_json  text not null default '{}',
    result_json text,
    error       text,
    progress    integer not null default 0,
    created_at  text not null default (datetime('now')),
    updated_at  text not null default (datetime('now'))
);

create table if not exists llm_model (
    id              text primary key,
    name            text not null,
    base_url        text not null,
    model           text not null,
    supports_vision integer not null default 0,
    enabled         integer not null default 1,
    key_ref         text not null,
    note            text,
    created_at      text not null default (datetime('now'))
);

create table if not exists llm_route (
    purpose  text not null,
    priority integer not null default 0,
    model_id text not null references llm_model (id) on delete cascade,
    primary key (purpose, priority)
);

create table if not exists sync_account (
    id            integer primary key check (id = 1),
    server_url    text not null default '',
    username      text not null default '',
    device_id     text not null default '',
    device_name   text not null default '',
    token_ref     text not null default 'sync-token',
    enabled       integer not null default 0,
    expires_at    text,
    last_login_at text
);

create table if not exists sync_outbox (
    id           text primary key,
    entity_type  text not null,
    entity_id    text not null,
    op           text not null,
    payload_json text not null default '{}',
    attempts     integer not null default 0,
    last_error   text,
    created_at   text not null default (datetime('now')),
    pushed_at    text
);

create index if not exists ix_outbox_pending on sync_outbox (pushed_at, created_at);

create table if not exists sync_link (
    local_id          text primary key,
    server_id         text not null,
    server_updated_at text
);

create table if not exists app_setting (
    key        text primary key,
    value_json text not null
);

-- 回滚（开发期手工执行）：
-- drop table if exists sync_link;
-- drop table if exists sync_outbox;
-- drop table if exists sync_account;
-- drop table if exists llm_route;
-- drop table if exists llm_model;
-- drop table if exists generation_job;
-- drop table if exists import_error;
-- drop table if exists import_batch;
-- drop table if exists question_asset;
-- drop table if exists question_knowledge;
-- drop table if exists question_tag;
-- drop table if exists question;
-- drop table if exists exam_point;
-- drop table if exists knowledge_point;
-- drop table if exists chapter;
-- drop table if exists textbook;
-- drop table if exists subject;
-- drop table if exists meta;
