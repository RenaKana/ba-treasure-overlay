# 第三方代码与素材

## 上游代码

本项目基于 [terry-u16/schale-inventory-management](https://github.com/terry-u16/schale-inventory-management)，桌面版改造基线为 `dea3ccbabff83060e2afd5f3a9d469c3ae983d04`。

上游许可证为 MIT，版权声明为 `Copyright (c) 2024 terry_u16`。完整许可原文保留在根目录 [LICENSE](LICENSE) 与 [app/LICENSE](app/LICENSE)。请在源码及包含该代码的分发包中继续保留这些声明。原始网页项目说明保留在 [app/README.md](app/README.md)。

## 游戏画面、识别参考与图标

`app/src-tauri/tests/fixtures/` 含用户提供的 MuMu、Steam 国际服窗口截图、裁剪图和由它们生成的回归样本。截图中游戏图像、角色、UI 和商标的权利属于相应权利人；项目的 MIT 许可证不对这些第三方内容作重新授权。

生产识别代码还直接嵌入以下两张结构参考，不能将它们仅视为测试附件：

- `app/src-tauri/tests/fixtures/vision-hidden-tiles.png`
- `app/src-tauri/tests/fixtures/vision-selected-cover.png`

来源和用途见 [vision-fixtures.md](app/src-tauri/tests/fixtures/vision-fixtures.md)、[PC 截图来源](app/src-tauri/tests/fixtures/pc-clients-provenance.md)、[4:3 截图来源](app/src-tauri/tests/fixtures/pc-global-four-three-provenance.md) 及同目录其他 provenance 文件。这些文件证明样本来源和测试用途，不等于权利人的再分发许可。

`app/src-tauri/icons/icon.ico` 未附独立来源或许可记录。当前 Tauri 构建默认将它编译进 Windows 资源，即使关闭安装包生成也会使用，不能直接删除。公开源码和二进制前，应核实截图、嵌入参考与图标的可再分发依据；需要替换时，应重新运行相应构建与识别回归。本项目不声称得到游戏开发商或发行商的认可。

## 外部依赖

前端依赖由 `app/package.json` 与 `app/pnpm-lock.yaml` 记录；桌面端和求解器依赖分别由 `app/src-tauri/Cargo.lock`、`app/wasm/Cargo.lock` 锁定。兼容性探针的依赖见 `compatibility_probe/requirements.txt`。

源码候选包不包含 `node_modules`、Rust registry、Python 环境或工具链。安装这些依赖时，各依赖原有许可证继续适用。React、MUI、Tauri、Windows bindings、windows-capture、image、serde、wasm-bindgen 等第三方代码不能仅凭本仓库的根许可证替代其自身的版权声明。

发布包含依赖代码的便携 EXE、WASM 或前端构建产物时，应按实际目标平台与依赖树收集完整许可证和 NOTICE 文本。仅列出包名或许可证简称不等于完成二进制分发义务。当前源码准备检查发现 Rust lockfile 内存在 MPL-2.0 依赖，部分其他平台 crate 的本地许可资料不可用，因此不宣称已完成全部平台的二进制许可证清单。
