-- 采集增强：来源记忆 + 连拍队列（P0 取题快车道）。
-- 只增表，不改历史表；回滚在文件尾部。

-- 来源记忆：一次浏览/一个网址记一条；网址做唯一键，重复采集累加次数。
create table if not exists capture_source (
    id            text primary key,
    url           text not null,
    label         text not null default '',
    starred       integer not null default 0,
    capture_count integer not null default 0,
    last_at       text not null,
    created_at    text not null
);
create unique index if not exists ux_capture_source_url on capture_source(url);

-- 连拍队列：一张图一项；状态 PENDING / RUNNING / DONE / FAILED。
-- job_id 指向批量识别任务；失败项可单独重试（重置为 PENDING）。
create table if not exists capture_queue (
    id          text primary key,
    image_path  text not null,
    source_id   text,
    source_url  text not null default '',
    status      text not null default 'PENDING',
    job_id      text,
    error       text not null default '',
    item_count  integer not null default 0,
    created_at  text not null,
    updated_at  text not null,
    foreign key (source_id) references capture_source(id) on delete set null
);
create index if not exists ix_capture_queue_status on capture_queue(status, created_at);

-- 回滚（在应用之外手工执行；迁移本身只增）：
-- drop table if exists capture_queue;
-- drop table if exists capture_source;
