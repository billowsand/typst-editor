# 第三方组件声明

字笺 · Typst（GPUI 版）在发布包中包含或静态链接以下组件。完整许可证文本随发布包放在 `licenses/` 目录，由打包脚本从各组件的源码包中复制。

## 字体（随程序内嵌，用于排版）

由 `typst-assets`（Apache-2.0）提供，经 `typst-kit` 的 `embed-fonts` 功能内嵌：

| 字体 | 许可证 |
|---|---|
| Libertinus Serif | SIL Open Font License 1.1 |
| New Computer Modern / New Computer Modern Math | GUST Font License |
| DejaVu Sans Mono | Bitstream Vera / DejaVu 许可证 |

详见 `licenses/typst-assets-NOTICE.txt`。界面字体（如 Noto Serif SC、LXGW Bright Code）不随程序分发，仅在本机已安装时使用。

## 主要库

| 组件 | 许可证 | 用途 |
|---|---|---|
| Typst（typst、typst-syntax、typst-ide、typst-render、typst-pdf、typst-svg、typst-kit） | Apache-2.0 | 编译、渲染、IDE 功能、包管理 |
| GPUI（gpui-pre 系列） | Apache-2.0 | 界面框架 |
| gpui-kit / gpui-component / gpui-base | Apache-2.0 | 界面组件、编辑器 |
| gpui-kit-assets（Lucide 图标） | ISC | 菜单与组件中的图标 |
| serde、serde_json、chrono、image、smallvec、trash 等 Rust crate | MIT 或 Apache-2.0 | 基础功能 |

Apache-2.0 全文见 `licenses/gpui-pre-LICENSE-APACHE.txt`（Typst 与 gpui-kit 的源码包未单独附带许可证文件，二者同为 Apache-2.0）；Lucide 的 ISC 许可证见 `licenses/gpui-kit-assets-LICENSE-LUCIDE.txt`。

完整依赖清单可用 `cargo tree` 查看；正式公开发布前建议用 `cargo about` 生成全部依赖的许可证汇总。

## 本项目原创素材

木纹、纸纹纹理和拟物 SVG 图标（`assets/`）由本仓库的 `assets/gen/` 脚本生成，属于本项目。
