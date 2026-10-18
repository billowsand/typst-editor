# 字笺 · 拟物主题（锤子便签风格）

参照锤子便签的三张截图，把桌面编辑器做成“一张书桌”：
胡桃木顶栏压着桦木桌面，桌上放着便签卡片、一本横线稿纸和排好的纸页。

![运行截图](implemented.png)

![欢迎页](welcome.png)

`smartisan-concept.png` 是上一版（扁平暖色）的生成设计稿，仅作历史参考。

## 配色（从截图取样）

| 元素 | 颜色 | 出处 |
|---|---|---|
| 胡桃木顶栏 | `#736252`，上沿高光、下沿 `#4d3e31` | 截图 1 顶栏 |
| 桦木桌面 | `#f0ddbc`，纹理 `#f6e5c8`→`#e3c697` | 截图 1 背景 |
| 便签卡片 | 头部 `#fdfcf7`，正文 `#fbf8f1`，装订条 `#f6f2e9` | 截图 1 卡片 |
| 稿纸 | `#fbf7ec`，横线 `#e6dfd2`，页边 `#ede7db` | 截图 3 |
| 墨色 | 正文 `#574033`，时间戳 `#b3aba1` | 截图 1、3 |
| 橙色标签 | 边框/文字 `#b98c57` | 截图 3 “RTF” |

全部集中在 [`src/theme.rs`](../src/theme.rs)。

## 拟物细节对照

| 锤子便签 | 字笺 |
|---|---|
| 木纹顶栏、浮雕白字、带阴影的白色图标 | `walnut_bar` + `embossed` 文字 + `bar_glyph` 双层图标；悬停时按钮“凹进”木头 |
| 标题旁的凹陷小三角 | 标题按钮旁的凹陷三角，点开切换文稿 |
| 顶栏投在桌面上的阴影 | `bar_cast_shadow`，状态栏同样向上投影 |
| 桦木桌面（教堂纹） | `textures/birch.jpg`：按“年轮圆柱 + 倾斜锯切面”物理模型生成 |
| 凹陷的胶囊搜索框 | `inset_pill`：内阴影 + 下沿白色高光；可搜索文稿与标题 |
| 便签卡片：左侧装订条、回形针、时间戳、图钉、星标、分隔线 | 每个 `.typ` 文件一张卡片：图钉 = 设为预览主文件，星标 = 置顶；当前文稿挂红色丝带书签，未保存显示橙色墨点 |
| 编辑页：文件夹凹槽、“日期 \| 字数”、星标、RTF 标签 | 稿纸页眉：项目凹槽、修改时间 \| 字数、星标、`TYP` 标签 |
| 横线稿纸 + 左侧页边 | 编辑器背景透明，下面用 canvas 画页边条、双线和横线，随编辑器滚动对齐每一行 |
| — | 打开的文件是插在稿纸顶边的索引页签；稿纸下面露出两张纸，像一本便笺本 |
| — | 预览页躺在桌面上，首页别着竖向回形针，页码刻在木头上 |
| — | 胡桃木状态栏：编译指示灯（绿/琥珀/红）+ 浮雕文字 |
| — | 提示是一张别着回形针的黄色便条，3.5 秒后收起 |

字体：界面与卡片用 Noto Serif SC（接近锤子便签的宋体气质），稿纸用 LXGW Bright Code（中英等宽）。

## 素材

- 纹理：[`assets/textures`](../assets/textures)，由 [`assets/gen/textures.py`](../assets/gen/textures.py) 生成（numpy + Pillow）。
- 图标：[`assets/skin`](../assets/skin)，由 [`assets/gen/skin_svgs.py`](../assets/gen/skin_svgs.py) 生成。
  彩色 SVG（回形针、图钉、星标、丝带、指示灯、笔记本图标）按 3 倍尺寸绘制，
  因为 GPUI 按 SVG 自身尺寸栅格化彩色图片；`g_*.svg` 为单色字形，由 GPUI 着色。
- 运行时由 [`src/assets.rs`](../src/assets.rs) 内嵌，并叠加在 gpui-kit 自带图标之上。

重新生成：

```powershell
cd gpui-app/assets/gen
python textures.py ../textures
python skin_svgs.py ../skin
```

## 已知限制

- GPUI 没有文字阴影，浮雕/压印效果用“同一段文字错开 1px 叠两层”实现。
- 稿纸横线依赖编辑器顶部内边距常量 `EDITOR_TOP_PAD`；若 gpui-kit 调整编辑器内边距需同步。
- 软换行开启时不显示行号（锤子便签也没有），行列号在状态栏。

## 运行

```powershell
cd gpui-app
cargo run -- examples
```
