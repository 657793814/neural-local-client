-- 听写台（P0）：只存当前批次听写内容与拼音，不存判分 / 会话 / 学情。
-- 照片放 assets/dictation/<setId>/，表里只存相对 assets/ 的路径。

create table if not exists local_dictation_set (
    id                text primary key,
    title             text not null,
    dictation_date    text not null default '',
    source_image_path text not null default '',
    lang_mode         text not null default 'MIXED',
    settings_json     text not null default '{}',
    created_at        text not null,
    updated_at        text not null,
    deleted_at        text
);
create index if not exists ix_local_dictation_set_date on local_dictation_set (dictation_date desc, deleted_at);

create table if not exists local_dictation_item (
    id                       text primary key,
    set_id                   text not null references local_dictation_set (id) on delete cascade,
    ord                      integer not null default 0,
    item_text                text not null,
    lang                     text not null default 'ZH',
    pinyin                   text not null default '',
    pinyin_candidates_json   text not null default '[]',
    selected_pinyin          text not null default '',
    speak_as                 text not null default '',
    meaning                  text not null default '',
    bbox_json                text not null default '',
    confidence               real not null default 0,
    needs_review             integer not null default 0,
    created_at               text not null
);
create index if not exists ix_local_dictation_item_set on local_dictation_item (set_id, ord);

insert or ignore into app_setting (key, value_json) values ('dictation.intervalMs', '5000');
insert or ignore into app_setting (key, value_json) values ('dictation.pinyinStyle', '"tone"');
insert or ignore into app_setting (key, value_json) values ('dictation.rate', '0.9');

-- 回滚：
-- drop table if exists local_dictation_item;
-- drop table if exists local_dictation_set;
-- delete from app_setting where key like 'dictation.%';
