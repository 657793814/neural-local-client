-- 教学闭环：学生 / 班级 / 作业 / 作答（P2）。
-- 只增表，不改历史表；回滚在文件尾部。
-- 约定：一律软删除（deleted_at），查询带 deleted_at is null。

-- 学生档案：本地主体，不要求中心账号；center_user_id 只有绑定后才写。
create table if not exists local_student (
    id             text primary key,
    name           text not null,
    grade          text not null default '',
    subject_ids    text not null default '[]',
    note           text not null default '',
    center_user_id text,
    status         text not null default 'ACTIVE',
    created_at     text not null,
    updated_at     text not null,
    deleted_at     text
);
create index if not exists ix_local_student_name on local_student (name, deleted_at);
create index if not exists ix_local_student_grade on local_student (grade, deleted_at);

create table if not exists local_class (
    id         text primary key,
    name       text not null,
    subject_id text references subject (id) on delete set null,
    term       text not null default '',
    join_code  text not null,
    status     text not null default 'ACTIVE',
    created_at text not null,
    updated_at text not null,
    deleted_at text
);
-- 邀请码只在未删除的班级里唯一；字符集与中心一致（去掉易混的 I/O/0/1）
create unique index if not exists ux_local_class_code on local_class (join_code) where deleted_at is null;
create index if not exists ix_local_class_status on local_class (status, deleted_at);

create table if not exists local_class_member (
    class_id   text not null references local_class (id) on delete cascade,
    student_id text not null references local_student (id) on delete cascade,
    joined_at  text not null,
    primary key (class_id, student_id)
);
create index if not exists ix_local_class_member_student on local_class_member (student_id);

create table if not exists local_assignment (
    id          text primary key,
    class_id    text not null references local_class (id) on delete cascade,
    title       text not null,
    type        text not null default 'PRACTICE',
    description text not null default '',
    due_at      text,
    late_hours  integer not null default 0,
    status      text not null default 'DRAFT',
    created_at  text not null,
    updated_at  text not null,
    deleted_at  text
);
create index if not exists ix_local_assignment_class on local_assignment (class_id, status, deleted_at);
create index if not exists ix_local_assignment_due on local_assignment (due_at);

create table if not exists local_assignment_question (
    assignment_id text not null references local_assignment (id) on delete cascade,
    question_id   text not null references question (id) on delete cascade,
    ord           integer not null default 0,
    score         real not null default 1,
    primary key (assignment_id, question_id)
);
create index if not exists ix_local_asq_question on local_assignment_question (question_id);

-- 作答：一次提交 = 一个 submission_id，按题一行。
-- correct 为 null 表示待批（主观题 / 填空），客观题提交时自动判分。
-- 重新作答不删旧行，用 superseded_at 标记，便于追溯。
create table if not exists local_attempt (
    id            text primary key,
    assignment_id text not null references local_assignment (id) on delete cascade,
    question_id   text not null references question (id) on delete cascade,
    student_id    text not null references local_student (id) on delete cascade,
    submission_id text not null,
    answer        text not null default '',
    duration_ms   integer,
    submitted_at  text not null,
    correct       integer,
    score         real,
    max_score     real not null default 1,
    remark        text not null default '',
    graded_by     text not null default '',
    graded_at     text,
    late          integer not null default 0,
    superseded_at text
);
create index if not exists ix_local_attempt_assignment on local_attempt (assignment_id, student_id, superseded_at);
create index if not exists ix_local_attempt_student on local_attempt (student_id, submitted_at);
create index if not exists ix_local_attempt_question on local_attempt (question_id);

-- 回滚：
-- drop table if exists local_attempt;
-- drop table if exists local_assignment_question;
-- drop table if exists local_assignment;
-- drop table if exists local_class_member;
-- drop table if exists local_class;
-- drop table if exists local_student;
