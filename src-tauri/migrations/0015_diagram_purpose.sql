-- AI 制图用途路由：给已有数据库补一条 diagram 路由。
--
-- llm::candidates() 对「没排过用途」会兜底到所有启用模型，所以这条不是必须的；
-- 但补上之后，用户在「设置 → 模型」里能看到 diagram 用途并单独排模型，
-- 数学精确图形（geogebra 用途）与通用示意图（diagram 用途）可以各挑擅长的模型。
-- 只在新用途完全没有任何路由时补，指向当前 priority 最低（即主）模型，不覆盖用户已有配置。

insert into llm_route (purpose, priority, model_id)
select 'diagram', 0, model_id
from llm_route
where purpose = 'question' and priority = 0
  and not exists (select 1 from llm_route where purpose = 'diagram');

-- 回滚说明：删除新用途的路由即可，不影响任何模型与题目数据。
-- delete from llm_route where purpose = 'diagram';
