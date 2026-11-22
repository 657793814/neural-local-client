-- 学情与错题本（P3）。只增表，不改历史表；回滚在文件尾部。
-- 约定与 0004 一致：一律软删除（deleted_at）。

-- 错题本：与中心 wrong_book 同状态机 NEW / REVIEWING / MASTERED。
-- 同一学生同一题只留一条，重复做错只累加 wrong_count 并刷新 last_wrong_at。
create table if not exists local_wrong_book (
    id                text primary key,
    student_id        text not null references local_student (id) on delete cascade,
    question_id       text not null references question (id) on delete cascade,
    source_attempt_id text,
    state             text not null default 'NEW',
    wrong_count       integer not null default 1,
    last_wrong_at     text not null,
    next_review_at    text,
    review_count      integer not null default 0,
    note              text not null default '',
    created_at        text not null,
    updated_at        text not null,
    deleted_at        text
);
create unique index if not exists ux_local_wrong_book on local_wrong_book (student_id, question_id)
    where deleted_at is null;
create index if not exists ix_local_wrong_book_state on local_wrong_book (student_id, state, deleted_at);
create index if not exists ix_local_wrong_book_review on local_wrong_book (next_review_at);

-- 作答照片：整页交上来的照片（attempt_id 为空）与切给某一题的照片（attempt_id 有值）共用一张表。
-- 文件落在 assets/submissions/<学生 id>/ 下，属于题目图那一类，不会被「清理临时文件」碰到。
create table if not exists local_attempt_asset (
    id            text primary key,
    attempt_id    text references local_attempt (id) on delete set null,
    student_id    text not null references local_student (id) on delete cascade,
    assignment_id text references local_assignment (id) on delete cascade,
    kind          text not null default 'PAGE',
    relative_path text not null,
    content_type  text not null default 'image/png',
    sha256        text not null default '',
    width         integer,
    height        integer,
    ord           integer not null default 0,
    created_at    text not null,
    deleted_at    text
);
create index if not exists ix_local_attempt_asset_student on local_attempt_asset (student_id, created_at);
create index if not exists ix_local_attempt_asset_attempt on local_attempt_asset (attempt_id);

-- 回滚：
-- drop table if exists local_attempt_asset;
-- drop table if exists local_wrong_book;
