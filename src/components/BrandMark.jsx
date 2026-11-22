/// 界面左上角的品牌标记：与应用图标同一形状（深靛蓝圆角底 + 翻开的两页书）。
/// 30px 下只留剪影与书脊缝，去掉渐变、书口金线与投影，免得糊成一团。
const PAGE_L = 'M503 368C434 340 328 324 254 326C220 328 204 346 204 374V626C204 654 220 670 254 672C328 688 434 698 503 700Z';
const PAGE_R = 'M521 368C590 340 696 324 770 326C804 328 820 346 820 374V626C820 654 804 670 770 672C696 688 590 698 521 700Z';

export default function BrandMark({size = 30}) {
    return (
        <svg
            className="brand-mark"
            width={size}
            height={size}
            viewBox="0 0 1024 1024"
            role="img"
            aria-label="NeuralKB"
        >
            <rect x="100" y="100" width="824" height="824" rx="190" fill="#232b45"/>
            <g transform="translate(512 512) scale(0.945 1.055) translate(-512 -513)" fill="#ffffff">
                <path d={PAGE_L}/>
                <path d={PAGE_R}/>
            </g>
        </svg>
    );
}
