-- 拍照批改（P3 后半）：在既有 attempt / submission 上挂图片附件，不建平行批改系统。
-- 只增表与增列，不改历史表；回滚在文件尾部。

-- 作答照片的溯源信息：
--   page_asset_id —— 这张题级图是从哪张整页照片切出来的（整页图自己是 null）；
--   bbox_json     —— 这块在整页里的位置（原图像素），方便重新切或回看原图。
-- 老库升级时这张表已存在，用 alter 补列；新库直接建全。
alter table local_attempt_asset add column page_asset_id text;
alter table local_attempt_asset add column bbox_json text;

create index if not exists ix_local_aaa_page on local_attempt_asset (page_asset_id);

-- 一道题挂几张图、顺序如何。题级图直接用 local_attempt_asset.attempt_id 关联，
-- 这张表只记顺序与备注（老师可以写「第二张是重拍的」）。
create table if not exists local_attempt_asset_note (
    attempt_id text not null references local_attempt (id) on delete cascade,
    asset_id   text not null references local_attempt_asset (id) on delete cascade,
    ord        integer not null default 0,
    note       text not null default '',
    primary key (attempt_id, asset_id)
);

-- 给已有模型补一条「批改抄录」路由：抄录要视觉模型，按视觉能力排。
-- 没有视觉模型时 llm::candidates 会退回全部启用模型并如实报错，不会静默瞎猜。
insert or ignore into llm_route (purpose, priority, model_id)
select 'grading',
       (select count(*) from llm_model m2
         where m2.enabled = 1 and m2.supports_vision = 1 and m2.id < m.id),
       m.id
  from llm_model m
 where m.enabled = 1 and m.supports_vision = 1;

-- 回滚：
-- delete from llm_route where purpose = 'grading';
-- drop table if exists local_attempt_asset_note;
-- drop index if exists ix_local_aaa_page;
-- alter table local_attempt_asset drop column bbox_json;
-- alter table local_attempt_asset drop column page_asset_id;
