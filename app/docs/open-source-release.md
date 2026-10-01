# GitHub 开源发布准备

检查日期：2026-10-01（Asia/Shanghai）。本次准备对象是当前桌面版源码，目标为 [RenaKana/ba-treasure-overlay](https://github.com/RenaKana/ba-treasure-overlay) 私有仓库。正式公开和 Windows Release 分发仍需完成下方待办。

GitHub 普通公开仓库没有应用商店式的“上架审核”。必要步骤是确认发布权利和目标仓库、清理公开范围、验证源码、推送并检查远程结果；Windows EXE 可另作为 Release 附件发布。CI、贡献指南、签名和安装包并非创建公开仓库的统一硬性门槛，应按本项目实际分发方式处理。

## 已准备

| 项目 | 结果 |
| --- | --- |
| 项目首页 | 简体中文、繁体中文、日语和英文 README 说明用途、使用方法、支持范围和已知限制 |
| 桌面界面 | 四语切换、语言与缩放偏好保存、75%–200% 控制界面缩放；顶部名称为“寻宝助手” |
| 上游归属 | 根目录 LICENSE 与原 `app/LICENSE` 完全一致，保留 MIT 版权原文和上游链接 |
| 素材边界 | `THIRD_PARTY_NOTICES.md` 列明游戏截图、生产嵌入参考及图标的待核实项 |
| 构建依赖 | pnpm 固定为 10.11.0；冻结前端 lockfile，WASM 和桌面构建使用 Cargo `--locked` |
| CI | 根目录 `.github/workflows/check.yml`；只读权限，执行检查和构建，无部署或发布步骤 |
| 源码范围 | 生成不含 `.git`、本地证据、原始样例、依赖目录或旧 Azure 部署流程的源码候选 |

本地 `origin` 仍指向原作者仓库，不应向其推送本项目发布。现有工作区包含此前未提交的功能改动，本次保留这些文件和所有旧发布目录。

## 验证结果

| 检查 | 本次结果 |
| --- | --- |
| 前端逻辑及显示偏好 | 34 / 34 通过，包含 7 项本地化和 2 项缩放测试 |
| 活动预设 | 14 / 14 通过，13 组预设 / 菜单及 4 种语言一致性通过 |
| Rust 求解器 | 37 次测试执行通过 |
| Windows 原生 | 加入开局 45 格参考后，232 次测试执行通过；1 项性能基准默认忽略，未计为通过；共享模块用例存在重复 |
| 候选目录安装 | pnpm 10.11.0 `install --frozen-lockfile --ignore-scripts` 通过 |
| 候选目录构建 | 重新生成 WASM、TypeScript / Vite 和 Tauri release EXE 均通过 |
| WASM 接口 | 常规与 snapshot smoke checks 通过，覆盖非法输入拒绝和既有导出 |
| CI 文件 | YAML 和只读权限检查通过；首次云端运行在桌面构建时因缺少 `pnpm` 命令失败，已补上 Corepack 命令入口，待新一次云端运行验证 |

候选目录使用独立安装的前端依赖，复用了本机 Rust 工具链和 Cargo 编译缓存。这证明当前源码在已具备工具链的本机可以构建，不等于全新 Windows 机器安装验收。构建仍有 dead_code 警告；本轮未把既有 lint 问题作为已通过项目。

显示功能另经过本地 TypeScript / Vite 与 Tauri release 构建，原生测试程序仅覆盖应用标识以隔离偏好。Windows 窗口已验证四语切换、标题更新、快捷键缩放、200% 布局及重启保留设置。浏览器模拟连接覆盖四语 × 8 档缩放（640 逻辑像素宽）：主要控件无溢出、未新增计算、概率遮罩尺寸不变；150% 缩放下框选坐标符合预期。模拟连接不等于真实游戏验收；本轮没有新增游戏识别、游戏内遮罩与刷新窗口的原生联动验收，相应边界沿用 [桌面验收记录](desktop-acceptance.md)。CSS 检查仍有 53 项既有规则问题，与修改前数量和规则分布一致。

本地原始结果保存在仓库根目录 `.artifacts/open-source-prep-20261001/`，不随公开源码上传。

开局方格参考的后续回归记录位于 `.artifacts/initial-board-reference-20261001/`。覆盖本轮 45 格逐位置对照、微小露出、遮挡、缺失或错误计数、Finish、同轮缩放及换轮重建；已确认参考不会被后续画面覆盖。新活动样式使用合成图检查，未新增真实跨活动验收。

## 正式公开前的待办

1. 当前目标已确定为 `RenaKana/ba-treasure-overlay` 私有仓库，上传使用独立仓库且保留上游归属；正式公开前确认可见性。若以后希望保留完整原始 Git 历史或发布为 fork，应先单独处理历史中的本地证据与个人信息，不能直接推送原工作区分支。
2. 确认游戏截图、两张生产嵌入参考和 `icon.ico` 的公开分发依据。需要删改或替换素材时，先重新构建和测试，避免交付缺少 `include_bytes!` 依赖的源码。
3. 处理或明确记录下表的已知依赖公告。当前检查没有自动升级依赖或改写依赖 lockfile；源码发布可披露已知问题，推荐在首次面向用户的二进制发布前修复并验证。
4. 上传后确认目标仓库可公开访问、README / LICENSE 可见、CI 实际通过，并检查 GitHub Secret Scanning / Push Protection 提示。只有这些远程结果可以作为 GitHub 发布成功的证据。
5. 若同步发布 EXE：从最终 tag 构建，补齐实际分发依赖的许可证 / NOTICE、使用说明、版本和 SHA256；如没有签名，明确说明。现有本地 EXE 与旧验收文件不自动等同于某个未来 tag 的发布产物。

### 已知依赖公告

`pnpm audit --prod` 对当前 lockfile 返回 2 项 moderate，0 high，0 critical；这只是 npm 生产依赖的公告查询，不包含 RustSec 审计或漏洞可利用性结论。

| 依赖 | 当前版本 | 公告 | 公告给出的修复版本 |
| --- | --- | --- | --- |
| `i18next-http-backend` | 3.0.2 | [GHSA-q89c-q3h5-w34g](https://github.com/advisories/GHSA-q89c-q3h5-w34g)，lng/ns 路径及 URL 注入 | >= 3.0.5 |
| `yaml`（Emotion 工具链间接引入） | 1.10.2 | [GHSA-48c2-rrv3-qjmp](https://github.com/advisories/GHSA-48c2-rrv3-qjmp)，深层 YAML 栈溢出 | >= 1.10.3 |

### 源码候选与历史的区别

候选复制当前文件内容，保留本次和此前未提交的桌面改动，不包含 Git 历史。它排除 `evidence/`、根目录 `样例1.jpg`、`app/.github/` 中的上游 Azure 部署配置、过期的 `app/package-lock.json` 以及 Git 忽略的本地产物，保留构建和回归需要的 `app/src-tauri/tests/fixtures/`。安装以 `app/pnpm-lock.yaml` 为准；旧 npm lock 缺少桌面 CLI，不用于候选构建，但工作区原文件仍保留。

`.gitignore` 不会删除已跟踪文件或清理历史。不能将候选包的排除效果误当作原分支已经完成公开清理，也不要使用当前 `HEAD` 的 `git archive` 替代候选：未提交的功能和准备文件尚不在 `HEAD` 中。公开源码内部分历史验收文档会引用本地忽略的证据，读者无法从仓库下载这些原始产物。

## GitHub 官方参考

- [仓库许可证](https://docs.github.com/en/repositories/managing-your-repositorys-settings-and-features/customizing-your-repository/licensing-a-repository)
- [GitHub 大文件限制与 LFS](https://docs.github.com/en/repositories/working-with-files/managing-large-files/about-large-files-on-github)
- [Secret Scanning](https://docs.github.com/en/code-security/secret-scanning/introduction/about-secret-scanning)
