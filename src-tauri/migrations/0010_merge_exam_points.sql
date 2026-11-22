-- 考察点并入知识点：考察点不再作为独立概念存在。
-- 每个 (题目所属学科, 考察点名) 先复用同学科同名知识点，复用不到才新建；
-- 然后把原本挂在 question_tag(EXAM_POINT) 上的关系搬到 question_knowledge。
--
-- 题目 subject_id 可能为 NULL，而 knowledge_point.subject_id 是 NOT NULL + 外键：
-- 这类题统一挂到排序最靠前的真实学科下（与同步落库策略一致），不直接丢标签。
-- 原有 question_knowledge 连线一律保留；合并后一题最多可达 6 个知识点，可接受。

drop table if exists _merge_exam_kp;
create temporary table _merge_exam_kp (
    subject_id         text not null,
    tag                text not null,
    knowledge_point_id text not null,
    primary key (subject_id, tag)
);

-- 1) 同名复用：同学科已有同名知识点时直接用它的 id。
insert or ignore into _merge_exam_kp (subject_id, tag, knowledge_point_id)
select coalesce(q.subject_id, (select id from subject order by sort, id limit 1)),
       t.tag,
       kp.id
from question_tag t
join question q on q.id = t.question_id
join knowledge_point kp
  on kp.subject_id = coalesce(q.subject_id, (select id from subject order by sort, id limit 1))
 and kp.name = t.tag
where t.kind = 'EXAM_POINT'
  and trim(t.tag) <> '';

-- 2) 没有同名知识点时新建一个（id 唯一且不与现有 id 冲突）。
insert or ignore into _merge_exam_kp (subject_id, tag, knowledge_point_id)
select distinct
       coalesce(q.subject_id, (select id from subject order by sort, id limit 1)),
       t.tag,
       'merged-' || lower(hex(randomblob(16)))
from question_tag t
join question q on q.id = t.question_id
where t.kind = 'EXAM_POINT'
  and trim(t.tag) <> ''
  and not exists (
      select 1 from _merge_exam_kp m
      where m.subject_id = coalesce(q.subject_id, (select id from subject order by sort, id limit 1))
        and m.tag = t.tag
  );

-- 3) 真正落库新建的知识点（code/summary/chapter_id/external_id 全空，sort=0）。
insert into knowledge_point (id, subject_id, chapter_id, code, name, summary, sort, external_id)
select m.knowledge_point_id, m.subject_id, null, null, m.tag, null, 0, null
from _merge_exam_kp m
where not exists (select 1 from knowledge_point kp where kp.id = m.knowledge_point_id);

-- 4) 把每条考察点标签转成 question_knowledge 连线（与已有连线取并集）。
insert or ignore into question_knowledge (question_id, knowledge_point_id)
select t.question_id, m.knowledge_point_id
from question_tag t
join question q on q.id = t.question_id
join _merge_exam_kp m
  on m.subject_id = coalesce(q.subject_id, (select id from subject order by sort, id limit 1))
 and m.tag = t.tag
where t.kind = 'EXAM_POINT'
  and trim(t.tag) <> '';

-- 5) 考察点标签本身不再保留。
delete from question_tag where kind = 'EXAM_POINT';

-- 6) 没用的 exam_point 表整张删掉。
drop table if exists exam_point;

drop table _merge_exam_kp;

-- 回滚：exam_point 表结构可以恢复，但已经被搬走的标签关系无法自动还原
-- （无法区分哪些 question_knowledge 是这次合并出来的），迁移前请备份。
-- create table if not exists exam_point (
--     id          text primary key,
--     subject_id  text not null references subject (id) on delete cascade,
--     name        text not null,
--     external_id text,
--     unique (subject_id, name)
-- );
