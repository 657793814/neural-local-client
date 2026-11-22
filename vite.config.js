import {defineConfig} from 'vite';
import react from '@vitejs/plugin-react';

// Tauri 需要固定端口，并在 Rust 侧 devUrl 指向它。
export default defineConfig({
    plugins: [react()],
    clearScreen: false,
    server: {
        port: 5183,
        strictPort: true,
        host: '127.0.0.1',
        watch: {ignored: ['**/src-tauri/**']},
    },
    build: {
        target: 'safari15',
        outDir: 'dist',
        emptyOutDir: true,
        // KaTeX 字体全部内联：安装包只多几百 KB，换来 dist 里只有 js + css，
        // 打包与清理都简单（也避开批量删除 60 多个字体文件）。
        assetsInlineLimit: 200000,
        // 桌面应用前端随包内置、断网加载，没有网络首屏，拆 chunk 不带收益；
        // CSS 内联字体后约 1.5MB 属于预期体积。调高阈值消掉「chunk 过大」提示，
        // 保留单 chunk（dist 里就一对 js + css），与 assetsInlineLimit 的策略一致。
        chunkSizeWarningLimit: 2000,
    },
});
