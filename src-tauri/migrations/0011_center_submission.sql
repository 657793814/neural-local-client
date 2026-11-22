-- 中心作答照片（阶段 B）：中心收学生的拍照提交，本地拉进批改台。
--
-- 只增列与索引，不改历史表；回滚在文件尾部。
--
--   * local_assignment.center_assignment_id —— 本地作业对应到中心哪份作业。
--     教师手动绑定（两边各自建的作业没有天然对应关系），绑定后「拉取照片」才知道拉哪份。
--   * local_assignment.center_assignment_title —— 绑定时的中心作业标题，只用于界面显示：
--     断网时也能看出「这份绑的是哪份中心作业」，不用为此再请求一次中心。
--   * local_attempt_asset.center_asset_id —— 这张整页照片来自中心的哪个附件，用来去重：
--     同一张图重复拉取只落一份，网络抖动不会让照片墙翻倍。
--   * local_attempt_asset.origin —— LOCAL（本机拍的 / 手机扫码传的）还是 CENTER（中心拉下来的）。
--     本地拍的原件与中心原件在批改上没有区别，只是出问题时能一眼看出图从哪来。

alter table local_assignment add column center_assignment_id text;
alter table local_assignment add column center_assignment_title text;
alter table local_attempt_asset add column center_asset_id text;
alter table local_attempt_asset add column origin text not null default 'LOCAL';

create index if not exists ix_local_assignment_center on local_assignment (center_assignment_id);
-- 只对中心来的照片去重：本地照片 center_asset_id 为 null，不受唯一约束影响。
create unique index if not exists ux_local_attempt_asset_center
    on local_attempt_asset (center_asset_id)
    where center_asset_id is not null;

-- 回滚：
-- drop index if exists ux_local_attempt_asset_center;
-- drop index if exists ix_local_assignment_center;
-- alter table local_attempt_asset drop column origin;
-- alter table local_attempt_asset drop column center_asset_id;
-- alter table local_assignment drop column center_assignment_id;
-- alter table local_assignment drop column center_assignment_title;
