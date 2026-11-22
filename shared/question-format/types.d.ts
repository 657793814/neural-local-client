/**
 * Neural Question Pack v1 —— 前后端 / 第三方智能体共用的类型。
 * 与 question-pack.schema.json、local-client/src-tauri/src/import.rs 的校验口径一致。
 */

export type QuestionType = 'SINGLE' | 'MULTI' | 'FILL' | 'SOLVED';

export type AssetKind = 'FIGURE' | 'PAGE' | 'CAPTURE';

export interface PackOption {
    key: string;
    text: string;
}

export interface PackImage {
    /** ZIP 内相对路径，例如 images/q1-1.png */
    file: string;
    kind?: AssetKind;
    alt?: string;
}

export interface PackQuestion {
    clientId?: string;
    type: QuestionType | string;
    stem: string;
    /** 选择题给数组；也可以给 "A|4;B|5" */
    options?: PackOption[] | string;
    answer: string;
    analysis?: string;
    /** 1–5，缺省 3 */
    difficulty?: number;
    /** 1–6 个考察点 */
    examPoints: string[] | string;
    tags?: string[];
    /** 教材 / 章节 / 知识点 */
    knowledgePath?: string[];
    source?: string;
    year?: number;
    images?: PackImage[];
    missingImages?: string[];
    review?: boolean;
    reviewNote?: string;
}

export interface QuestionPack {
    format?: 'neural-question-pack';
    version?: 1;
    generatedAt?: string;
    generator?: string;
    subject?: string;
    questions: PackQuestion[];
}

/** 导入预览里单题的状态。 */
export type PreviewStatus = 'READY' | 'DUPLICATE' | 'SIMILAR' | 'INVALID';

/** 校验问题码，出现在 import_error.code 与界面的红字提示里。 */
export type PackIssueCode =
    | 'MISSING_STEM'
    | 'UNKNOWN_TYPE'
    | 'OPTIONS_TOO_FEW'
    | 'OPTIONS_TOO_MANY'
    | 'MISSING_ANSWER'
    | 'MISSING_EXAM_POINTS'
    | 'UNSAFE_IMAGE_PATH'
    | 'UNSUPPORTED_IMAGE'
    | 'PACK_VERSION_TOO_NEW'
    | 'NOT_A_PACK';
