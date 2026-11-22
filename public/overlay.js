    const invoke = (cmd, args) => window.__TAURI_INTERNALS__.invoke(cmd, args);

    const image = document.getElementById('shot');
    const veil = document.getElementById('veil');
    const sel = document.getElementById('sel');
    const ink = document.getElementById('ink');
    const sizeLabel = document.getElementById('size');
    const bar = document.getElementById('bar');
    const hint = document.getElementById('hint');
    // 提示条的原文。提交失败会把它改成报错，下一轮得能还原回去。
    const HINT_HTML = hint.innerHTML;

    const MARK = '#ff4d4f';

    let scale = { w: 1, h: 1 };   // 屏幕像素 / CSS 像素，用来显示真实的截图尺寸
    let k = 1;                    // 标注画布像素 / CSS 像素
    let baseImage = null;         // 底图的 Image，马赛克要从它采样
    let rect = null;              // 选区（视口 CSS 坐标）
    let shapes = [];              // 已完成的标注
    let draft = null;             // 正在画的
    let tool = 'pointer';
    let mode = null;              // null | select | draw | move
    let anchor = null;            // 框选/绘制的起点（CSS 或画布坐标）
    let moving = null;
    let textInput = null;
    let settled = false;
    let watchdog = null;

    function imageRect() {
      const box = image.getBoundingClientRect();
      return { left: box.left, top: box.top, width: box.width || 1, height: box.height || 1 };
    }

    function clampPoint(x, y) {
      const box = imageRect();
      return {
        x: Math.max(box.left, Math.min(x, box.left + box.width)),
        y: Math.max(box.top, Math.min(y, box.top + box.height))
      };
    }

    // ---------------------------------------------------------------- 画布

    function inkRatio() {
      const long = Math.max(rect ? rect.w : 0, rect ? rect.h : 0, 1);
      const dpr = Math.min(2, window.devicePixelRatio || 1);
      // 上限 1600：再大对识别没帮助，导出的 PNG 却会成倍变重。
      return Math.max(0.4, Math.min(dpr, 1600 / long));
    }

    function toInk(cx, cy) {
      return { x: (cx - rect.x) * k, y: (cy - rect.y) * k };
    }

    function sizeCanvas() {
      const ratio = inkRatio();
      const w = Math.max(1, Math.round(rect.w * ratio));
      const h = Math.max(1, Math.round(rect.h * ratio));
      if (ink.width !== w || ink.height !== h) {
        ink.width = w;
        ink.height = h;
      }
      ink.style.width = rect.w + 'px';
      ink.style.height = rect.h + 'px';
      k = ratio;
      redraw();
    }

    function lineWidth() {
      return Math.max(2, 3 * k);
    }

    function drawShape(ctx, shape) {
      const lw = lineWidth();
      ctx.lineWidth = lw;
      ctx.strokeStyle = MARK;
      ctx.fillStyle = MARK;
      ctx.lineCap = 'round';
      ctx.lineJoin = 'round';

      if (shape.type === 'rect') {
        ctx.strokeRect(shape.x, shape.y, shape.w, shape.h);
      } else if (shape.type === 'ellipse') {
        ctx.beginPath();
        ctx.ellipse(
          shape.x + shape.w / 2, shape.y + shape.h / 2,
          Math.abs(shape.w) / 2, Math.abs(shape.h) / 2,
          0, 0, Math.PI * 2
        );
        ctx.stroke();
      } else if (shape.type === 'arrow') {
        const dx = shape.x2 - shape.x1;
        const dy = shape.y2 - shape.y1;
        const len = Math.hypot(dx, dy);
        if (len < 1) return;
        const ux = dx / len;
        const uy = dy / len;
        const head = Math.max(9 * k, lw * 3.2);
        const wing = head * 0.42;
        // 线收在箭头根部，免得粗线从三角尖里戳出来
        ctx.beginPath();
        ctx.moveTo(shape.x1, shape.y1);
        ctx.lineTo(shape.x2 - ux * head * 0.9, shape.y2 - uy * head * 0.9);
        ctx.stroke();
        ctx.beginPath();
        ctx.moveTo(shape.x2, shape.y2);
        ctx.lineTo(shape.x2 - ux * head + -uy * wing, shape.y2 - uy * head + ux * wing);
        ctx.lineTo(shape.x2 - ux * head - -uy * wing, shape.y2 - uy * head - ux * wing);
        ctx.closePath();
        ctx.fill();
      } else if (shape.type === 'pen') {
        const pts = shape.points;
        if (!pts || pts.length === 0) return;
        ctx.beginPath();
        ctx.moveTo(pts[0].x, pts[0].y);
        for (let i = 1; i < pts.length; i += 1) ctx.lineTo(pts[i].x, pts[i].y);
        if (pts.length === 1) {
          ctx.arc(pts[0].x, pts[0].y, lw / 2, 0, Math.PI * 2);
          ctx.fill();
        } else {
          ctx.stroke();
        }
      } else if (shape.type === 'mosaic') {
        drawMosaic(ctx, shape);
      } else if (shape.type === 'text') {
        const size = shape.size * k;
        ctx.font = '600 ' + size + 'px -apple-system, "PingFang SC", sans-serif';
        ctx.textBaseline = 'top';
        ctx.fillText(shape.text, shape.x, shape.y);
      }
    }

    // 马赛克：从底图对应区域取像素，缩成小格再放大。画到标注层上是
    // 不透明的，叠回原图时就是「这块被糊掉了」——和微信的观感一致。
    function drawMosaic(ctx, shape) {
      if (!baseImage || !baseImage.naturalWidth) {
        ctx.fillStyle = 'rgba(18, 20, 26, .88)';
        ctx.fillRect(shape.x, shape.y, shape.w, shape.h);
        return;
      }
      const block = Math.max(5, 9 * k);
      const sw = Math.max(1, Math.round(shape.w / block));
      const sh = Math.max(1, Math.round(shape.h / block));
      const tmp = document.createElement('canvas');
      tmp.width = sw;
      tmp.height = sh;
      const tctx = tmp.getContext('2d');
      const box = imageRect();
      const perInk = baseImage.naturalWidth / (box.width * k);
      const ox = (rect.x - box.left) * k;
      const oy = (rect.y - box.top) * k;
      tctx.drawImage(
        baseImage,
        (ox + shape.x) * perInk, (oy + shape.y) * perInk,
        shape.w * perInk, shape.h * perInk,
        0, 0, sw, sh
      );
      ctx.save();
      ctx.imageSmoothingEnabled = false;
      ctx.drawImage(tmp, 0, 0, sw, sh, shape.x, shape.y, shape.w, shape.h);
      ctx.restore();
    }

    function redraw() {
      const ctx = ink.getContext('2d');
      ctx.clearRect(0, 0, ink.width, ink.height);
      for (let i = 0; i < shapes.length; i += 1) drawShape(ctx, shapes[i]);
      if (draft) drawShape(ctx, draft);
    }

    // ---------------------------------------------------------------- 布局

    function paint() {
      if (!rect) {
        sel.hidden = true;
        veil.hidden = false;
        hint.hidden = false;
        bar.hidden = true;
        return;
      }
      veil.hidden = true;
      hint.hidden = true;
      sel.hidden = false;
      sel.style.left = rect.x + 'px';
      sel.style.top = rect.y + 'px';
      sel.style.width = rect.w + 'px';
      sel.style.height = rect.h + 'px';
      sizeCanvas();
      const px = Math.round(rect.w * scale.w);
      const py = Math.round(rect.h * scale.h);
      sizeLabel.textContent = px + ' × ' + py;
      // 框太靠上时把尺寸标签放进框内，免得被屏幕边切掉
      sizeLabel.style.top = rect.y < 28 ? '4px' : '-24px';
    }

    function placeBar() {
      if (!rect) return;
      // 窗口是常驻复用的，页面不会重载，disabled 也就不会自己消失。
      // 上一轮提交时 lock() 过的按钮必须在这里解锁，否则第二轮看到的是
      // 「工具条在、但一个都点不动」。
      if (!settled) unlock();
      bar.hidden = false;
      const box = bar.getBoundingClientRect();
      const gap = 8;
      let top = rect.y + rect.h + gap;
      if (top + box.height > window.innerHeight - 8) top = rect.y - box.height - gap;
      if (top < 8) {
        top = Math.min(window.innerHeight - box.height - 8, rect.y + rect.h - box.height - gap);
      }
      let left = rect.x + rect.w - box.width;
      left = Math.max(8, Math.min(left, window.innerWidth - box.width - 8));
      bar.style.top = top + 'px';
      bar.style.left = left + 'px';
    }

    // ---------------------------------------------------------------- 选区

    function normalize(ax, ay, bx, by) {
      return {
        x: Math.min(ax, bx),
        y: Math.min(ay, by),
        w: Math.abs(ax - bx),
        h: Math.abs(ay - by)
      };
    }

    function startSelect(point) {
      closeTextInput(true);
      rect = { x: point.x, y: point.y, w: 0, h: 0 };
      shapes = [];
      draft = null;
      mode = 'select';
      anchor = point;
      bar.hidden = true;
      paint();
    }

    function inRect(point) {
      return rect && point.x >= rect.x && point.x <= rect.x + rect.w
        && point.y >= rect.y && point.y <= rect.y + rect.h;
    }

    // ---------------------------------------------------------------- 文字

    function openTextInput(point) {
      closeTextInput(true);
      const input = document.createElement('input');
      input.id = 'text-input';
      input.type = 'text';
      input.spellcheck = false;
      input.style.left = (point.x - rect.x) + 'px';
      input.style.top = (point.y - rect.y - 13) + 'px';
      input.style.fontSize = '20px';
      sel.appendChild(input);
      textInput = { el: input, x: (point.x - rect.x) * k, y: (point.y - rect.y - 13) * k };

      input.addEventListener('keydown', function (event) {
        event.stopPropagation();
        if (event.key === 'Enter') {
          event.preventDefault();
          commitText();
        } else if (event.key === 'Escape') {
          event.preventDefault();
          closeTextInput(true);
        }
      });
      input.addEventListener('blur', function () { commitText(); });
      input.focus();
    }

    function commitText() {
      if (!textInput) return;
      const entry = textInput;
      textInput = null;
      const text = entry.el.value.trim();
      if (entry.el.parentNode) entry.el.parentNode.removeChild(entry.el);
      if (text) {
        shapes.push({ type: 'text', x: entry.x, y: entry.y, text: text, size: 20 });
        redraw();
      }
    }

    function closeTextInput(commit) {
      if (!textInput) return;
      if (commit) {
        commitText();
      } else {
        const entry = textInput;
        textInput = null;
        if (entry.el.parentNode) entry.el.parentNode.removeChild(entry.el);
      }
    }

    // ---------------------------------------------------------------- 交互

    function setTool(next) {
      tool = next;
      const all = bar.querySelectorAll('button[aria-label]');
      for (let i = 0; i < all.length; i += 1) all[i].classList.remove('active');
      const active = document.getElementById('tool-' + next);
      if (active) active.classList.add('active');
    }

    function onPointerDown(event) {
      if (settled) return;
      if (event.button !== 0) return;
      // 工具条、文字输入框上的按下都不是「开始框选」。监听器挂在 window 上，
      // 不在这里放行的话，按下按钮会先把工具条藏掉（bar.hidden），按钮跟着消失，
      // click 就永远等不到——表现成「点了没反应，蒙层也不消失」。
      if (event.target.closest && event.target.closest('#bar')) return;
      if (event.target.closest && event.target.closest('#text-input')) return;

      closeTextInput(true);
      const point = clampPoint(event.clientX, event.clientY);

      if (!rect || !inRect(point)) {
        startSelect(point);
        return;
      }
      if (tool === 'pointer') {
        mode = 'move';
        moving = { from: point, box: { x: rect.x, y: rect.y, w: rect.w, h: rect.h } };
        return;
      }
      if (tool === 'text') {
        openTextInput(point);
        return;
      }

      mode = 'draw';
      anchor = toInk(point.x, point.y);
      if (tool === 'pen') {
        draft = { type: 'pen', points: [anchor] };
      } else if (tool === 'arrow') {
        draft = { type: 'arrow', x1: anchor.x, y1: anchor.y, x2: anchor.x, y2: anchor.y };
      } else {
        draft = { type: tool, x: anchor.x, y: anchor.y, w: 0, h: 0 };
      }
      redraw();
    }

    function onPointerMove(event) {
      if (!mode) return;
      const point = clampPoint(event.clientX, event.clientY);

      if (mode === 'select') {
        rect = normalize(anchor.x, anchor.y, point.x, point.y);
        paint();
        return;
      }
      if (mode === 'move') {
        const box = imageRect();
        const dx = point.x - moving.from.x;
        const dy = point.y - moving.from.y;
        const w = moving.box.w;
        const h = moving.box.h;
        rect = {
          x: Math.max(box.left, Math.min(moving.box.x + dx, box.left + box.width - w)),
          y: Math.max(box.top, Math.min(moving.box.y + dy, box.top + box.height - h)),
          w: w, h: h
        };
        paint();
        return;
      }
      if (mode === 'draw' && draft) {
        const at = toInk(point.x, point.y);
        if (draft.type === 'pen') {
          draft.points.push(at);
        } else if (draft.type === 'arrow') {
          draft.x2 = at.x;
          draft.y2 = at.y;
        } else {
          const shape = normalize(anchor.x, anchor.y, at.x, at.y);
          draft.x = shape.x;
          draft.y = shape.y;
          draft.w = shape.w;
          draft.h = shape.h;
        }
        redraw();
      }
    }

    function onPointerUp(event) {
      if (!mode) return;
      const finished = mode;
      mode = null;

      if (finished === 'select') {
        const point = clampPoint(event.clientX, event.clientY);
        rect = normalize(anchor.x, anchor.y, point.x, point.y);
        if (rect.w < 8 || rect.h < 8) {
          rect = null;
          paint();
          return;
        }
        paint();
        placeBar();
        return;
      }
      if (finished === 'move') {
        moving = null;
        placeBar();
        return;
      }
      if (finished === 'draw' && draft) {
        const shape = draft;
        draft = null;
        const tiny = (shape.type === 'arrow' && Math.hypot(shape.x2 - shape.x1, shape.y2 - shape.y1) < 4)
          || (shape.type === 'pen' && shape.points.length < 2)
          || ((shape.type === 'rect' || shape.type === 'ellipse' || shape.type === 'mosaic')
            && (shape.w < 4 || shape.h < 4));
        if (!tiny) shapes.push(shape);
        redraw();
        placeBar();
      }
    }

    function undo() {
      if (!shapes.length) return;
      shapes.pop();
      redraw();
    }

    // ---------------------------------------------------------------- 提交

    function lock() {
      const buttons = bar.querySelectorAll('button');
      for (let i = 0; i < buttons.length; i += 1) buttons[i].disabled = true;
    }

    function unlock() {
      const buttons = bar.querySelectorAll('button');
      for (let i = 0; i < buttons.length; i += 1) buttons[i].disabled = false;
    }

    /**
     * 提交后的兜底。
     *
     * 点了「识别」就把按钮全锁上，等壳把图编码完、把窗口收走。这一步平时几十毫秒，
     * 但只要它没有回音（IPC 掉了、sender 已经空了、编码卡住），页面就会永远停在
     * 「工具条全灰、点了没反应、Esc 也不响应」——人只能强退壳。
     *
     * 与其指望调用方一定会 reset，不如让页面自己有个止损：12 秒没被收走就当作失败，
     * 解锁并把话说清楚，用户可以再点一次，或按 Esc 退出。
     */
    function armWatchdog() {
      clearWatchdog();
      watchdog = setTimeout(function () {
        watchdog = null;
        if (!settled) return;
        settled = false;
        unlock();
        hint.textContent = '提交没有回音：可以再点一次「识别」，或按 Esc 取消';
        hint.hidden = false;
        invoke('overlay_report', { note: '提交超时，工具条已解锁，可重试' });
      }, 12000);
    }

    function clearWatchdog() {
      if (!watchdog) return;
      clearTimeout(watchdog);
      watchdog = null;
    }

    // 提交失败必须让人看见。选区层铺满整块屏幕，一旦这一步没通，
    // 人面对的就是「点了没反应、蒙层也不走」，连退都退不出去。
    function reportFailure(error) {
      settled = false;
      clearWatchdog();
      unlock();
      hint.textContent = '提交失败：' + error;
      hint.hidden = false;
    }

    function selectionPayload() {
      const box = imageRect();
      return {
        x: (rect.x - box.left) / box.width,
        y: (rect.y - box.top) / box.height,
        width: rect.w / box.width,
        height: rect.h / box.height,
        full: false
      };
    }

    function markup() {
      if (!shapes.length || ink.width < 2) return null;
      let url = null;
      try {
        url = ink.toDataURL('image/png');
      } catch (error) {
        // canvas 被污染（底图没走 CORS）时这里会抛 SecurityError。
        invoke('overlay_report', { note: '标注导出失败：' + error });
        return null;
      }
      // 整块马赛克是 PNG 最坏的情况（满是噪点）。IPC 不是无底洞，
      // 超过 2MB 就降一档重导，宁可糊一点，也别让提交卡在那儿。
      let ratio = 0.7;
      while (url && url.length > 2000000 && ratio >= 0.35) {
        const small = document.createElement('canvas');
        small.width = Math.max(1, Math.round(ink.width * ratio));
        small.height = Math.max(1, Math.round(ink.height * ratio));
        small.getContext('2d').drawImage(ink, 0, 0, small.width, small.height);
        try {
          url = small.toDataURL('image/png');
        } catch (error) {
          break;
        }
        ratio -= 0.2;
      }
      return url;
    }

    function submit(copyOnly) {
      if (settled) return;
      closeTextInput(true);
      const only = copyOnly === true;
      // 没选区或框太小：当成「整屏」处理。整屏没有标注层可言。
      if (!rect || rect.w < 8 || rect.h < 8) {
        submitFull(only);
        return;
      }
      settled = true;
      lock();
      armWatchdog();
      const payload = selectionPayload();
      payload.markup = markup();
      payload.copyOnly = only;
      invoke('submit_selection', payload).catch(reportFailure);
    }

    function submitFull(copyOnly) {
      if (settled) return;
      closeTextInput(true);
      settled = true;
      lock();
      armWatchdog();
      invoke('submit_selection', {
        x: 0, y: 0, width: 1, height: 1,
        full: true, markup: null, copyOnly: copyOnly === true
      }).catch(reportFailure);
    }

    function cancel() {
      if (settled) return;
      settled = true;
      clearWatchdog();
      // 取消失败也不该卡住人：壳那边有 180 秒超时兜底。
      invoke('cancel_selection').catch(function () {});
    }

    // ---------------------------------------------------------------- 事件

    window.addEventListener('pointerdown', onPointerDown);
    window.addEventListener('pointermove', onPointerMove);
    window.addEventListener('pointerup', onPointerUp);
    // 微信里双击选区＝完成，这里保持同一个肌肉记忆。
    window.addEventListener('dblclick', function (event) {
      if (settled || !rect) return;
      if (event.target.closest && event.target.closest('#bar')) return;
      submit(false);
    });
    window.addEventListener('contextmenu', function (event) {
      event.preventDefault();
      cancel();
    });
    window.addEventListener('keydown', function (event) {
      if (document.activeElement && document.activeElement.id === 'text-input') return;
      const meta = event.metaKey || event.ctrlKey;
      if (event.key === 'Escape') {
        event.preventDefault();
        cancel();
      } else if (event.key === 'Enter') {
        event.preventDefault();
        if (event.shiftKey) submitFull(false); else submit(false);
      } else if (meta && event.key.toLowerCase() === 'z') {
        event.preventDefault();
        undo();
      } else if (meta && event.key.toLowerCase() === 'c') {
        event.preventDefault();
        submit(true);
      } else if (event.key === 'Backspace' || event.key === 'Delete') {
        event.preventDefault();
        undo();
      } else if (event.key === 'v') { setTool('pointer'); }
      else if (event.key === 'r') { setTool('rect'); }
      else if (event.key === 'o') { setTool('ellipse'); }
      else if (event.key === 'a') { setTool('arrow'); }
      else if (event.key === 'p') { setTool('pen'); }
      else if (event.key === 'm') { setTool('mosaic'); }
      else if (event.key === 't') { setTool('text'); }
    });
    window.addEventListener('resize', function () {
      if (mode === 'move') return;
      paint();
      if (rect) placeBar();
    });

    document.getElementById('tool-pointer').addEventListener('click', function () { setTool('pointer'); });
    document.getElementById('tool-rect').addEventListener('click', function () { setTool('rect'); });
    document.getElementById('tool-ellipse').addEventListener('click', function () { setTool('ellipse'); });
    document.getElementById('tool-arrow').addEventListener('click', function () { setTool('arrow'); });
    document.getElementById('tool-pen').addEventListener('click', function () { setTool('pen'); });
    document.getElementById('tool-mosaic').addEventListener('click', function () { setTool('mosaic'); });
    document.getElementById('tool-text').addEventListener('click', function () { setTool('text'); });
    document.getElementById('undo').addEventListener('click', undo);
    document.getElementById('ok').addEventListener('click', function () { submit(false); });
    document.getElementById('copy').addEventListener('click', function () { submit(true); });
    document.getElementById('full').addEventListener('click', function () { submitFull(false); });
    document.getElementById('cancel').addEventListener('click', cancel);

    // ---------------------------------------------------------------- 生命周期

    // 壳常驻这个窗口，一晚上要开合几十次。这两个函数就是开合时拨动的开关：
    // reset 回到「屏幕刚暗下来」的样子，shoot 把底图贴上去。
    // 走全局函数而不是自定义事件，是因为壳那边只做一次 eval，链路短、失败也看得见。
    function attach(path, width, height) {
      const internals = window.__TAURI_INTERNALS__ || {};
      const url = typeof internals.convertFileSrc === 'function'
        ? internals.convertFileSrc(path)
        : path;
      // 底图走 asset 协议：几百 KB 的图放进 data URL 时这条 webview 不绘制，
      // 元素尺寸全对也白搭——落盘再加载才画得出来。
      image.style.backgroundImage = 'url("' + url + '")';
      image.dataset.ready = '1';

      // 马赛克要从底图采样，单独留一份 Image：背景图没法直接读像素。
      // 必须带 crossOrigin：asset:// 是另一个源，不带的话 drawImage 过去就把
      // canvas 污染了，之后 toDataURL 直接抛 SecurityError——整层标注会静默消失。
      const probe = new Image();
      probe.crossOrigin = 'anonymous';
      probe.addEventListener('load', function () {
        baseImage = probe;
        try {
          const t = document.createElement('canvas');
          t.width = 1;
          t.height = 1;
          const tctx = t.getContext('2d');
          tctx.drawImage(probe, 0, 0, 1, 1);
          tctx.getImageData(0, 0, 1, 1);
          invoke('overlay_report', { note: '底图可读像素，马赛克可用' });
        } catch (error) {
          invoke('overlay_report', { note: '底图污染了 canvas：' + error });
        }
      }, { once: true });
      probe.addEventListener('error', function () {
        baseImage = null;
        invoke('overlay_report', { note: '底图采样失败，马赛克退化成分块色块' });
      }, { once: true });
      probe.src = url;

      const box = imageRect();
      scale = { w: width / box.width, h: height / box.height };
      invoke('overlay_report', {
        note: '底图已挂上 · 视口 ' + window.innerWidth + 'x' + window.innerHeight
          + ' · 画布 ' + Math.round(box.width) + 'x' + Math.round(box.height)
      });
      paint();
    }

    window.__NB_OVERLAY__ = {
      reset() {
        // 顺序有讲究：先把「能不能操作」恢复，再收拾上一轮的残局。
        // 万一下面某一步抛异常（文字输入框已经被移除、底图还没加载完之类），
        // 工具条也已经解锁了——反过来写的话，异常会把 12 个按钮永远钉在灰色。
        settled = false;
        clearWatchdog();
        unlock();
        mode = null;
        moving = null;
        anchor = null;
        rect = null;
        shapes = [];
        draft = null;
        closeTextInput(false);
        setTool('pointer');
        hint.innerHTML = HINT_HTML;
        // 先把底图撤掉再重绘：否则上一题的选区框会叠在新画面上闪一帧。
        image.dataset.ready = '';
        image.style.backgroundImage = '';
        bar.hidden = true;
        paint();
        // 回执：日志里一轮一条。第二轮按钮若还灰，看这行有没有、
        // 有没有在「选区层上屏」之前，就能分清是没复位还是复位晚了。
        invoke('overlay_report', { note: '选区层已复位 · 工具条可点' });
      },
      shoot(path, width, height) {
        attach(path, width, height);
      }
    };

    (function boot() {
      // 窗口是壳在启动时就建好的，页面加载完只做一件事：报到，并问一句有没有已经在等的图。
      invoke('overlay_ready').then(function (shot) {
        if (shot && shot.shot_path) {
          attach(shot.shot_path, shot.width, shot.height);
        } else {
          window.__NB_OVERLAY__.reset();
        }
      }).catch(function (error) {
        invoke('overlay_report', { note: 'ready 失败：' + error });
      });
    })();
