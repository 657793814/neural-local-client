// 把 @worm-vue3-print/common 的 UMD 产物同步进 vendor/（离线可用、可审计版本）。
// 用法：node sync-vendor.mjs [版本号]     默认取 package.json 的 wormPrintVersion
import {execFileSync} from 'node:child_process'
import {copyFile, mkdir, mkdtemp, readFile, rm, writeFile} from 'node:fs/promises'
import {tmpdir} from 'node:os'
import {dirname, join, resolve} from 'node:path'
import {fileURLToPath} from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const pkg = JSON.parse(await readFile(resolve(here, 'package.json'), 'utf8'))
const version = process.argv[2] || pkg.wormPrintVersion || '1.4.8'
const name = '@worm-vue3-print/common'

const work = await mkdtemp(join(tmpdir(), 'printdesk-vendor-'))
try {
    console.log(`[vendor] npm pack ${name}@${version}`)
    const output = execFileSync('npm', ['pack', `${name}@${version}`, '--silent'], {cwd: work, encoding: 'utf8'})
    const tarball = output.trim().split('\n').pop()
    execFileSync('tar', ['-xzf', join(work, tarball)], {cwd: work})

    const vendor = resolve(here, 'vendor')
    await mkdir(vendor, {recursive: true})
    for (const file of ['common.js', 'common.css']) {
        await copyFile(join(work, 'package/dist', file), join(vendor, file))
        console.log(`[vendor] ${file}`)
    }
    await copyFile(join(work, 'package/LICENSE'), join(vendor, 'LICENSE'))
    await writeFile(
        join(vendor, 'VERSION'),
        `${name}@${version}\n同步时间：${new Date().toISOString()}\n来源：npm pack ${name}@${version}\n`,
    )
    console.log(`[vendor] 完成：${vendor}`)
} finally {
    await rm(work, {recursive: true, force: true})
}
