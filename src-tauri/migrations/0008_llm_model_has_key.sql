-- 设置页列模型时不再去系统钥匙串读密码：每次读都会触发 macOS 钥匙串授权弹窗。
-- 改成在表里记一个 has_key 标记位，保存 / 删除 Key 时同步更新；列表只读这一列。
alter table llm_model add column has_key integer not null default 0;

-- 回滚：alter table llm_model drop column has_key;
