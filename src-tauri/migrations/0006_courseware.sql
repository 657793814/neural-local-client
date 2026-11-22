-- 自制课件（P4）：本地课件库 + 表单化 spec 产物。
-- 只增表：导入的单文件 HTML 与本地编译产物共用一张表，source 区分来源。
-- 文件本身落在 assets/courseware/，表里只存相对 assets/ 的路径。

create table if not exists local_courseware (
    id               text primary key,
    title            text not null,
    subject_id       text references subject (id) on delete set null,
    grade            text not null default '',
    description      text not null default '',
    -- 知识点用 JSON 数组存，展示与筛选都在本地做，不做关联表。
    knowledge        text not null default '[]',
    file_path        text not null,
    file_sha256      text not null default '',
    -- LOCAL_BUILD 才有：生成这份课件用的 spec，编辑时可回填
    lesson_spec_json text,
    source           text not null default 'IMPORT',
    version          integer not null default 1,
    step_count       integer not null default 0,
    quiz_count       integer not null default 0,
    size_bytes       integer not null default 0,
    -- 导入时的静态扫描结论（外链、体积等），JSON 数组
    warnings         text not null default '[]',
    created_at       text not null,
    updated_at       text not null,
    deleted_at       text
);
create index if not exists ix_local_courseware_subject on local_courseware (subject_id, deleted_at);
create index if not exists ix_local_courseware_updated on local_courseware (updated_at desc);
create index if not exists ix_local_courseware_sha on local_courseware (file_sha256);

create table if not exists local_class_courseware (
    class_id      text not null references local_class (id) on delete cascade,
    courseware_id text not null references local_courseware (id) on delete cascade,
    assigned_at   text not null,
    due_at        text,
    note          text not null default '',
    primary key (class_id, courseware_id)
);
create index if not exists ix_local_class_courseware_cw on local_class_courseware (courseware_id);

-- 回滚：
-- drop table if exists local_courseware_question;
-- drop table if exists local_class_courseware;
-- drop table if exists local_courseware;

-- 讲义里的题与题库题的挂接：例题 / 变式 / 拔高 / 作业都能回溯到原题。
create table if not exists local_courseware_question (
    courseware_id text not null references local_courseware (id) on delete cascade,
    question_id   text not null references question (id) on delete cascade,
    role          text not null default 'EXAMPLE',
    ord           integer not null default 0,
    primary key (courseware_id, question_id, role)
);
create index if not exists ix_local_cwq_question on local_courseware_question (question_id);
