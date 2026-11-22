-- 试卷管理（v12）：结构化编排 + 题目快照 + 固定版式模板 + 打印记录。
-- 设计见 docs/product/local-paper-management-plan.md
-- 约定：试卷只存「编排」，题目内容进 snapshot_json；题库改题不影响历史卷。
-- 回滚 SQL 见文件末尾注释。

-- 试卷主表
create table if not exists local_paper (
    id             text primary key,
    title          text not null,
    subject_id     text references subject (id) on delete set null,
    grade          text not null default '',
    exam_type      text not null default 'EXAM',   -- EXAM 考试卷 / PRACTICE 练习卷 / QUIZ 小测
    status         text not null default 'DRAFT',  -- DRAFT / FINAL / ARCHIVED
    -- 卷面总分：创建时按标准卷方案定死，之后只能在「重新设置总分」里改，
    -- 改完必须重新配平；定稿要求 sum(题分) 与它完全相等。
    total_score    real not null default 0,
    score_scheme   text not null default '',
    duration_min   integer not null default 0,
    question_count integer not null default 0,
    layout_json    text not null default '{}',
    blueprint_json text not null default '{}',
    slots_json     text not null default '{}',
    note           text not null default '',
    created_at     text not null,
    updated_at     text not null,
    finalized_at   text,
    deleted_at     text
);
create index if not exists ix_local_paper_list
    on local_paper (status, updated_at desc, deleted_at);
create index if not exists ix_local_paper_subject
    on local_paper (subject_id);

-- 大题：一、选择题 / 二、填空题 …
create table if not exists local_paper_section (
    id         text primary key,
    paper_id   text not null references local_paper (id) on delete cascade,
    ord        integer not null default 0,
    title      text not null default '',
    note       text not null default '',        -- 留空时按分值自动生成「每小题 X 分，共 Y 分」
    kind       text not null default '',        -- 题型约束，可空表示混合
    score_mode text not null default 'UNIFORM', -- UNIFORM 统一分值 / PER_ITEM 逐题自定
    unit_score real not null default 0,
    layout_json text not null default '{}'
);
create index if not exists ix_local_paper_section
    on local_paper_section (paper_id, ord);

-- 试卷题目：快照 + 分值 + 顺序
create table if not exists local_paper_question (
    paper_id      text not null references local_paper (id) on delete cascade,
    section_id    text not null references local_paper_section (id) on delete cascade,
    ord           integer not null default 0,
    question_id   text references question (id) on delete set null,
    score         real not null default 0,
    snapshot_json text not null,
    snapshot_at   text not null,
    flags_json    text not null default '{}',
    primary key (paper_id, section_id, ord)
);
create index if not exists ix_local_paper_question_q
    on local_paper_question (question_id);

-- 固定版式模板（封面 / 卷头 / 答题卡 / 答案页）
create table if not exists local_paper_template (
    id            text primary key,
    name          text not null,
    kind          text not null default 'COVER',
    template_json text not null,
    thumb         text,
    builtin       integer not null default 0,
    created_at    text not null,
    updated_at    text not null
);

-- 打印记录
create table if not exists local_paper_print (
    id         text primary key,
    paper_id   text not null references local_paper (id) on delete cascade,
    variant    text not null default 'STUDENT',
    copies     integer not null default 1,
    pages      integer not null default 0,
    channel    text not null default 'BROWSER',
    printer    text not null default '',
    created_at text not null
);
create index if not exists ix_local_paper_print
    on local_paper_print (paper_id, created_at desc);

-- 回滚（按依赖顺序）：
-- drop index if exists ix_local_paper_print;
-- drop table if exists local_paper_print;
-- drop table if exists local_paper_template;
-- drop index if exists ix_local_paper_question_q;
-- drop table if exists local_paper_question;
-- drop index if exists ix_local_paper_section;
-- drop table if exists local_paper_section;
-- drop index if exists ix_local_paper_subject;
-- drop index if exists ix_local_paper_list;
-- drop table if exists local_paper;
