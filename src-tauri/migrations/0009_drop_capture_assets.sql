-- 截图只用于识别，属于 tmp 里的临时文件，不再作为题目附件。
-- 历史库里 question_asset 的 CAPTURE 行一律清掉；对应图片文件由「设置 → 存储清理」
-- 按孤儿图回收。题干 / 选项里真正用到的图（FIGURE）不受影响。
delete from question_asset where kind = 'CAPTURE';

-- 回滚：无法恢复。旧版截图本就不该跟题绑定，重新识别即可补出需要的图。
