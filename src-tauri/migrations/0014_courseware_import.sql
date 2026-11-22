-- 已有课件批量导入：每个文件一条独立任务，原稿、抽取文字与校对草稿都可恢复。
-- 只增表；managed_path 始终是相对 assets/ 的受管路径。

create table if not exists local_courseware_import (
    id             text primary key,
    batch_id       text not null,
    source_path    text not null,
    managed_path   text not null default '',
    sha256         text not null default '',
    title          text not null default '',
    status         text not null default 'QUEUED'
                   check (status in ('QUEUED', 'EXTRACTING', 'REVIEW', 'FAILED', 'DUPLICATE', 'COMMITTED')),
    mode           text not null default 'FORMAT_ONLY'
                   check (mode in ('FORMAT_ONLY', 'OPTIMIZE')),
    use_ai         integer not null default 0,
    subject_id     text references subject (id) on delete set null,
    subject        text not null default '',
    grade          text not null default '',
    extracted_text text not null default '',
    draft_json     text,
    raw_ai_json    text,
    warnings_json  text not null default '[]',
    locations_json text not null default '[]',
    error          text not null default '',
    courseware_id  text references local_courseware (id) on delete set null,
    attempts       integer not null default 0,
    created_at     text not null,
    updated_at     text not null
);

create index if not exists ix_local_courseware_import_batch
    on local_courseware_import (batch_id, created_at);
create index if not exists ix_local_courseware_import_status
    on local_courseware_import (status);
create index if not exists ix_local_courseware_import_sha256
    on local_courseware_import (sha256);

-- 回滚：
-- drop table if exists local_courseware_import;
