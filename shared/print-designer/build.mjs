// 构建通用包产物：dist/print-designer.js（IIFE，全局 PrintDesk）+ dist/print-designer.esm.js
// 依赖 esbuild，优先用仓库根的 node_modules，其次本地。
import {build} from 'esbuild'
import {copyFile, mkdir, readFile, writeFile} from 'node:fs/promises'
import {dirname, resolve} from 'node:path'
import {fileURLToPath} from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const dist = resolve(here, 'dist')
const entry = resolve(here, 'src/index.js')

await mkdir(dist, {recursive: true})

const common = {
    entryPoints: [entry],
    bundle: true,
    target: 'es2020',
    legalComments: 'none',
    logLevel: 'warning',
}

await build({
    ...common,
    format: 'iife',
    globalName: 'PrintDesk',
    outfile: resolve(dist, 'print-designer.js'),
})

await build({
    ...common,
    format: 'esm',
    outfile: resolve(dist, 'print-designer.esm.js'),
})

// vendor 目录随包分发：dist/vendor/* 由 sync-vendor.mjs 准备，这里只做存在性提示
try {
    await readFile(resolve(dist, 'vendor/common.js'))
} catch {
    console.warn('[build] dist/vendor/common.js 缺失，先跑 npm run sync:vendor')
}

const pkg = JSON.parse(await readFile(resolve(here, 'package.json'), 'utf8'))
await writeFile(
    resolve(dist, 'VERSION'),
    `${pkg.name}@${pkg.version}\n内置设计器运行时：@worm-vue3-print/common@${pkg.wormPrintVersion}\n构建时间：${new Date().toISOString()}\n`,
)

const stat = async (file) => (await readFile(resolve(dist, file))).length
console.log(`[build] print-designer.js      ${(await stat('print-designer.js')).toLocaleString()} B`)
console.log(`[build] print-designer.esm.js  ${(await stat('print-designer.esm.js')).toLocaleString()} B`)

// IIFE 产物里 dist/package.json 写成 commonjs 只是为了让 node 别把 .js 当 ESM——
// 浏览器直挂不看 package.json，这里不复制源码样式（样式在运行时注入）。
await copyFile(resolve(here, 'README.md'), resolve(dist, 'README.md')).catch(() => {})
