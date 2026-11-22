//! 听写台的本地拼音：离线字典 + 词组消歧 + 多音字候选。
//!
//! `pinyin` crate 只有单字表，没有词组表；直接逐字取第一个音会把「银行」读成 yín xíng。
//! 这里先用一张小词组表按位置覆盖，其余字再退回单字默认音，并把多音字的全部读音
//! 交给校对页让人点选——自动结果永远只是建议，不会被当成定论。

use pinyin::{Pinyin, ToPinyin, ToPinyinMulti};
use serde_json::{json, Value};

/// 常见多音字词组：词组 → 整词拼音（音节数与字数一致）。
/// 只收听写里高频的词；没覆盖到的交给单字默认音 + 人工候选。
const PHRASES: &[(&str, &str)] = &[
    ("银行", "yín háng"),
    ("行走", "xíng zǒu"),
    ("行动", "xíng dòng"),
    ("行业", "háng yè"),
    ("行列", "háng liè"),
    ("长大", "zhǎng dà"),
    ("成长", "chéng zhǎng"),
    ("长度", "cháng dù"),
    ("长短", "cháng duǎn"),
    ("长江", "cháng jiāng"),
    ("音乐", "yīn yuè"),
    ("快乐", "kuài lè"),
    ("乐器", "yuè qì"),
    ("重要", "zhòng yào"),
    ("重复", "chóng fù"),
    ("重量", "zhòng liàng"),
    ("重新", "chóng xīn"),
    ("发现", "fā xiàn"),
    ("头发", "tóu fa"),
    ("朝阳", "zhāo yáng"),
    ("朝向", "cháo xiàng"),
    ("差别", "chā bié"),
    ("差不多", "chà bu duō"),
    ("参差", "cēn cī"),
    ("参加", "cān jiā"),
    ("人参", "rén shēn"),
    ("模型", "mó xíng"),
    ("模样", "mú yàng"),
    ("干净", "gān jìng"),
    ("干活", "gàn huó"),
    ("首都", "shǒu dū"),
    ("都是", "dōu shì"),
    ("爱好", "ài hào"),
    ("好奇", "hào qí"),
    ("教书", "jiāo shū"),
    ("教室", "jiào shì"),
    ("干净", "gān jìng"),
    ("朝鲜", "cháo xiǎn"),
    ("银行家", "yín háng jiā"),
    ("中学生", "zhōng xué shēng"),
    ("中间", "zhōng jiān"),
    ("中奖", "zhòng jiǎng"),
    ("空间", "kōng jiān"),
    ("空闲", "kòng xián"),
    ("多少", "duō shǎo"),
    ("少年", "shào nián"),
    ("好人", "hǎo rén"),
    ("好学", "hào xué"),
    ("还是", "hái shì"),
    ("归还", "guī huán"),
    ("关系", "guān xì"),
    ("系鞋带", "jì xié dài"),
    ("弯曲", "wān qū"),
    ("歌曲", "gē qǔ"),
    ("舍弃", "shě qì"),
    ("宿舍", "sù shè"),
    ("着急", "zháo jí"),
    ("着凉", "zháo liáng"),
    ("沉着", "chén zhuó"),
    ("看着", "kàn zhe"),
];

/// 选中某个读音后给 TTS 的提示字：AVSpeechSynthesizer / Web Speech 对中文多音字
/// 未必按语境读，用同音常用字最稳。没有映射时留空，由前端手填。
const HOMOPHONES: &[(&str, &str)] = &[
    ("xíng", "行"), ("háng", "航"), ("zhòng", "众"), ("chóng", "重"),
    ("cháng", "长"), ("zhǎng", "掌"), ("yuè", "月"), ("lè", "乐"),
    ("xiāng", "相"), ("xiàng", "向"), ("fā", "发"), ("fà", "发"),
    ("kōng", "空"), ("kòng", "控"), ("shǎo", "少"), ("shào", "绍"),
    ("hǎo", "好"), ("hào", "号"), ("jiāo", "交"), ("jiào", "叫"),
    ("gān", "干"), ("gàn", "干"), ("dū", "督"), ("dōu", "都"),
    ("chā", "插"), ("chà", "差"), ("chāi", "拆"), ("cī", "疵"),
    ("mó", "模"), ("mú", "母"), ("jǐn", "紧"), ("jìn", "进"),
    ("huán", "还"), ("hái", "孩"), ("zhāo", "招"), ("cháo", "潮"),
    ("shě", "舍"), ("shè", "社"), ("qǔ", "曲"), ("qū", "区"),
    ("jì", "计"), ("xì", "细"), ("zhe", "着"), ("zháo", "招"),
    ("zhuó", "着"), ("zhǎng", "掌"), ("chén", "沉"), ("shēn", "深"),
];

fn is_han(ch: char) -> bool {
    ('\u{3400}'..='\u{9fff}').contains(&ch)
}

fn homophone(pinyin: &str) -> &'static str {
    HOMOPHONES
        .iter()
        .find(|(key, _)| *key == pinyin)
        .map(|(_, value)| *value)
        .unwrap_or("")
}

/// 给一个中文词条带出拼音建议与多音字候选。
/// 返回 `{pinyin, candidates, needsReview}`，`candidates` 只含真正多音的字。
pub fn annotate(text: &str) -> Value {
    let chars: Vec<char> = text.chars().filter(|ch| is_han(*ch)).collect();
    if chars.is_empty() {
        return json!({"pinyin": "", "candidates": [], "needsReview": false});
    }
    let compact: String = chars.iter().collect();
    let mut readings: Vec<Option<String>> = vec![None; chars.len()];

    // 词组优先：按字符位置覆盖，长词先走，避免「银行家」被「银行」截断后剩一个错音。
    let mut phrases: Vec<(&str, &str)> = PHRASES.to_vec();
    phrases.sort_by_key(|(word, _)| std::cmp::Reverse(word.chars().count()));
    for (word, syllable_text) in phrases {
        let syllables: Vec<&str> = syllable_text.split_whitespace().collect();
        let width = word.chars().count();
        if syllables.len() != width {
            continue;
        }
        let mut from = 0usize;
        while let Some(found) = compact[from..].find(word) {
            let byte_at = from + found;
            let char_at = compact[..byte_at].chars().count();
            if char_at + width <= readings.len() {
                for (offset, syllable) in syllables.iter().enumerate() {
                    readings[char_at + offset] = Some((*syllable).to_string());
                }
            }
            from = byte_at + word.len();
            if from >= compact.len() {
                break;
            }
        }
    }

    let mut pinyin_parts: Vec<String> = Vec::with_capacity(chars.len());
    let mut candidates: Vec<Value> = Vec::new();
    for (index, ch) in chars.iter().enumerate() {
        let default = readings[index]
            .clone()
            .or_else(|| ch.to_pinyin().map(Pinyin::with_tone).map(str::to_string))
            .unwrap_or_default();
        pinyin_parts.push(default);

        let options: Vec<String> = ch
            .to_pinyin_multi()
            .into_iter()
            .flat_map(|multi| multi.into_iter())
            .take(4)
            .map(Pinyin::with_tone)
            .map(str::to_string)
            .collect();
        if options.len() > 1 {
            let choices: Vec<Value> = options
                .iter()
                .map(|py| {
                    json!({
                        "pinyin": py,
                        "homophone": homophone(py),
                    })
                })
                .collect();
            candidates.push(json!({"char": ch.to_string(), "index": index, "options": choices}));
        }
    }

    json!({
        "pinyin": pinyin_parts.join(" "),
        "candidates": candidates,
        "needsReview": !candidates.is_empty(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phrase_beats_single_character_default() {
        let result = annotate("银行");
        assert_eq!(result["pinyin"], "yín háng");
        assert_eq!(annotate("长大")["pinyin"], "zhǎng dà");
        assert_eq!(annotate("行走")["pinyin"], "xíng zǒu");
    }

    #[test]
    fn polyphone_has_candidates() {
        let result = annotate("重");
        let options = result["candidates"][0]["options"].as_array().unwrap();
        let readings: Vec<&str> = options
            .iter()
            .map(|item| item["pinyin"].as_str().unwrap())
            .collect();
        assert!(readings.contains(&"zhòng"), "{readings:?}");
        assert!(readings.contains(&"chóng"), "{readings:?}");
        assert_eq!(result["needsReview"], true);
    }

    #[test]
    fn single_reading_word_is_clean() {
        let result = annotate("老师");
        assert_eq!(result["needsReview"], false, "单音词不该要人工定音：{result}");
        assert_eq!(result["pinyin"].as_str().unwrap().split_whitespace().count(), 2);
    }
}
