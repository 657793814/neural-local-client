/**
 * 指针特效引擎：一个全局 pointermove 监听，驱动页面上的几类微交互。
 *
 * 之所以不每个卡片各自挂监听，是因为题库列表动辄几十张卡、每张卡里还有公式和图片，
 * 逐个绑定会让事件对象成倍增加。这里统一在 document 上做事件委托 + rAF 合并，
 * 只在指针真正落到目标上时写 CSS 变量，由样式层完成动画。
 *
 * 约定：
 *   .liquid           液态玻璃（--gx/--gy 高光位置，点击产生涟漪）
 *   .btn.primary      磁吸主按钮（--mag-x/--mag-y）
 *   [data-tilt]       3D 倾斜卡（--tilt-x/--tilt-y）
 *   .gate             登录页光斑视差（--par-x/--par-y）
 */

const LIQUID = '.liquid';
const MAGNET = '.btn.primary';
const TILT = '[data-tilt]';

const clamp = (value, min, max) => Math.min(max, Math.max(min, value));

function isElement(value) {
    return typeof Element !== 'undefined' && value instanceof Element;
}

/** 用户要求减少动态效果，或设备没有精确指针（触屏）时，整套特效都不挂载。 */
export function effectsDisabled() {
    if (typeof window === 'undefined') return true;
    const reduce = window.matchMedia?.('(prefers-reduced-motion: reduce)');
    const coarse = window.matchMedia?.('(hover: none)');
    return Boolean(reduce?.matches || coarse?.matches);
}

export function attachPointerEffects() {
    if (effectsDisabled()) return () => {};

    let frame = 0;
    let pending = null;
    let magnetEl = null;
    let tiltEl = null;

    const clearMagnet = () => {
        if (!magnetEl) return;
        magnetEl.style.removeProperty('--mag-x');
        magnetEl.style.removeProperty('--mag-y');
        magnetEl = null;
    };

    const clearTilt = () => {
        if (!tiltEl) return;
        tiltEl.style.removeProperty('--tilt-x');
        tiltEl.style.removeProperty('--tilt-y');
        tiltEl = null;
    };

    const apply = () => {
        frame = 0;
        const event = pending;
        if (!event) return;
        const target = isElement(event.target) ? event.target : null;
        // 液态玻璃高光。
        const liquid = target?.closest(LIQUID);
        if (liquid) {
            const rect = liquid.getBoundingClientRect();
            liquid.style.setProperty(
                '--gx',
                `${((event.clientX - rect.left) / Math.max(1, rect.width)) * 100}%`,
            );
            liquid.style.setProperty(
                '--gy',
                `${((event.clientY - rect.top) / Math.max(1, rect.height)) * 100}%`,
            );
        }

        // 磁吸主按钮：指针在按钮内时给一个很小的位移，离开后由 CSS 弹回。
        const nextMagnet = target?.closest(MAGNET) || null;
        if (nextMagnet !== magnetEl) {
            clearMagnet();
            magnetEl = nextMagnet;
        }
        if (magnetEl) {
            const rect = magnetEl.getBoundingClientRect();
            const dx = clamp(event.clientX - (rect.left + rect.width / 2), -80, 80);
            const dy = clamp(event.clientY - (rect.top + rect.height / 2), -60, 60);
            magnetEl.style.setProperty('--mag-x', `${(dx * 0.16).toFixed(1)}px`);
            magnetEl.style.setProperty('--mag-y', `${(dy * 0.16).toFixed(1)}px`);
        }

        // 统计卡 3D 倾斜：最大 4°，避免数字被拉变形。
        const nextTilt = target?.closest(TILT) || null;
        if (nextTilt !== tiltEl) {
            clearTilt();
            tiltEl = nextTilt;
        }
        if (tiltEl) {
            const rect = tiltEl.getBoundingClientRect();
            const px = (event.clientX - rect.left) / Math.max(1, rect.width);
            const py = (event.clientY - rect.top) / Math.max(1, rect.height);
            tiltEl.style.setProperty('--tilt-x', `${((0.5 - py) * 4).toFixed(2)}deg`);
            tiltEl.style.setProperty('--tilt-y', `${((px - 0.5) * 4).toFixed(2)}deg`);
        }

        // 登录页光斑随指针轻微视差。
        const gate = target?.closest('.gate');
        if (gate) {
            gate.style.setProperty('--par-x', `${((event.clientX / window.innerWidth) - 0.5) * 2}`);
            gate.style.setProperty('--par-y', `${((event.clientY / window.innerHeight) - 0.5) * 2}`);
        }
    };

    const onMove = (event) => {
        pending = event;
        if (!frame) frame = requestAnimationFrame(apply);
    };

    const onOut = (event) => {
        const target = isElement(event.target) ? event.target : null;
        const to = isElement(event.relatedTarget) ? event.relatedTarget : null;
        if (magnetEl && target?.closest(MAGNET) === magnetEl && !magnetEl.contains(to)) clearMagnet();
        if (tiltEl && target?.closest(TILT) === tiltEl && !tiltEl.contains(to)) clearTilt();
    };

    // 点击液态玻璃时从落点扩散一圈涟漪；节点播完动画自己移除。
    // 点在按钮 / 输入框上时不扩散，避免和控件自身的反馈打架。
    const onClick = (event) => {
        const target = isElement(event.target) ? event.target : null;
        const liquid = target?.closest(LIQUID);
        if (!liquid) return;
        if (target.closest('button, a, input, textarea, select, label, [role="button"]')) return;
        const rect = liquid.getBoundingClientRect();
        const ripple = document.createElement('span');
        ripple.className = 'liquid-ripple';
        ripple.setAttribute('aria-hidden', 'true');
        ripple.style.left = `${event.clientX - rect.left}px`;
        ripple.style.top = `${event.clientY - rect.top}px`;
        liquid.appendChild(ripple);
        ripple.addEventListener('animationend', () => ripple.remove(), {once: true});
    };

    document.addEventListener('pointermove', onMove, {passive: true});
    document.addEventListener('pointerout', onOut, {passive: true});
    document.addEventListener('click', onClick);

    return () => {
        document.removeEventListener('pointermove', onMove);
        document.removeEventListener('pointerout', onOut);
        document.removeEventListener('click', onClick);
        if (frame) cancelAnimationFrame(frame);
        clearMagnet();
        clearTilt();
    };
}
