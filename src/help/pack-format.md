# 题目包格式（v1）

一份 JSON，人看得懂，客户端校验得住。

## 骨架

```json
{
  "format": "neural-question-pack",
  "version": 1,
  "subject": "数学",
  "questions": [ { "type": "SINGLE", "stem": "…" } ]
}
```

## 字段

- `clientId`：可选 UUID，没有就自动生成。
- `type`：`SINGLE` 单选 / `MULTI` 多选 / `FILL` 填空 / `SOLVED` 解答。也认「单选题」这类中文写法。
- `stem`：题干，必填。公式必须用 `$...$`。
- `options`：选择题必填，2–7 项。可以是 `[{"key":"A","text":"4"}]`，也可以是字符串 `A|4;B|5`。
- `answer`：必填。选择题填字母，多选写 `AC`。
- `analysis`：解析，可空。公式同样用 `$...$`。
- `difficulty`：1–5，缺省 3。
- `knowledgePaths`：必填，至少 1 条教材 / 章节 / 知识点路径（数组的数组）。旧包里的 `examPoints` 仍兼容，会当作知识点名归并进来。
- `tags`：可选，最多 12 个。
- `knowledgePath`：可选，教材 / 章节 / 知识点路径；写第一条，`knowledgePaths` 可挂多个。
- `images`：可选，`[{"file":"images/q1-1.png","kind":"FIGURE","alt":"图1"}]`。
- `missingImages`：无法导出的图片名字，让客户端提示补图。
- `review` / `reviewNote`：不确定的题标一下，导入后进「草稿箱」。

## 客户端自己导出的包

「题库 → 导出」产出的包和导入是同一种格式，另外多两个字段：

- `generator` / `generatedAt` / `count` / `withImages`：说明是谁、什么时候、带没带图。
- `warnings`：导出时就发现「这题没挂知识点」的提醒，换台机器导入可能被判不合格。
- `clientId` 会写成原题 id，所以**把导出的包再导回来是覆盖同一道题**，不会多出一份。

带图 ZIP 的结构就是 `questions.json` + `images/…`，题干里用 `![](images/1-1.png)` 引用；`images[]` 里记 `file` / `kind` / `alt`。

## 公式与反斜杠

模型最容易在这里翻车：

- 公式写 `$\frac{1}{2}$`，不要写裸的 `\frac{1}{2}`；
- JSON 字符串里的反斜杠要写两个：`$\\frac{1}{2}$`。

客户端会在解析前按已知命令名修一次，但**能被修**不等于**应该这么写**。

## ZIP 结构

```text
my-pack.zip
├─ questions.json
├─ images/q1-1.png
└─ README.txt
```

## 版本

`version` 大于 1 时客户端会拒绝并提示升级，不会猜着解析。
