-- 学号 + 成绩（考试） + AI 学情报告（v13）。
-- 只增列 / 增表，不改历史表；回滚 SQL 见文件末尾。
-- 设计：docs/product/local-first-teaching-plan.md §15.13

-- 学号：学校里的固定编号，用来在名单对不齐时定位人、也用来对成绩单。
-- 允许为空（老数据、临时学生），非空时全局唯一，避免把两个人的成绩并成一份。
alter table local_student add column student_no text not null default '';
create unique index if not exists ux_local_student_no
    on local_student (student_no)
    where deleted_at is null and student_no <> '';

-- 考试：一次考试一份记录，分数按人一行。
-- 与「作业」分开：作业是按题判分自动汇总，考试常常是整卷总分，直接录。
create table if not exists local_exam (
    id         text primary key,
    class_id   text not null references local_class (id) on delete cascade,
    title      text not null,
    kind       text not null default 'EXAM',   -- EXAM 考试 / QUIZ 小测 / UNIT 单元测
    full_score real not null default 100,
    -- 及格线 / 优秀线：留空表示按满分的 60% / 85% 算，改了满分不会把判定带歪。
    pass_line  real,
    good_line  real,
    taken_at   text not null,
    note       text not null default '',
    created_at text not null,
    updated_at text not null,
    deleted_at text
);
create index if not exists ix_local_exam_class on local_exam (class_id, taken_at, deleted_at);

-- 分数：score 为 null 表示缺考或还没录，和 0 分区分开。
create table if not exists local_exam_score (
    exam_id    text not null references local_exam (id) on delete cascade,
    student_id text not null references local_student (id) on delete cascade,
    score      real,
    note       text not null default '',
    updated_at text not null,
    primary key (exam_id, student_id)
);
create index if not exists ix_local_exam_score_student on local_exam_score (student_id);

-- AI 学情报告：一个学生一份，按数据指纹判断是否过期。
-- 只存生成结果，不存对话；模型不可用时用规则版，source 记 'RULE'。
create table if not exists local_insight_report (
    student_id  text primary key references local_student (id) on delete cascade,
    fingerprint text not null default '',
    source      text not null default 'RULE',   -- AI / RULE
    model       text not null default '',
    payload     text not null default '{}',
    created_at  text not null
);

-- 回滚：
-- drop table if exists local_insight_report;
-- drop table if exists local_exam_score;
-- drop table if exists local_exam;
-- drop index if exists ux_local_student_no;
-- alter table local_student drop column student_no;
