# LuaTools v4 架构设计文档

> 本文档沉淀自项目设计与重构协作过程，供本地开发者快速了解 v4 方案、对比 v2/v3 关键变化与精简点。
>
> 最后更新：2026-04-10

---

## 目录

1. [项目目标与背景](#1-项目目标与背景)
2. [V4 核心精简：断舍离决策](#2-v4-核心精简断舍离决策)
3. [技术栈选择](#3-技术栈选择)
4. [目录结构](#4-目录结构)
5. [前端架构设计](#5-前端架构设计)
6. [后端刷写引擎架构](#6-后端刷写引擎架构)
7. [核心外部依赖：yuzhan-tech/luatos-tools](#7-核心外部依赖yuzhan-techluatos-tools)
8. [杂项架构决策](#8-杂项架构决策)
9. [分阶段重构计划](#9-分阶段重构计划)
10. [PR 协作建议](#10-pr-协作建议)
11. [FAQ](#11-faq)

---

## 1. 项目目标与背景

### 1.1 项目定位

LuaTools v4 是合宙（Air) LuatOS 生态的**下一代桌面端开发与量产工具**，目标是：

- **极速**：本地全程，刷写和打包操作无需联网，毫秒级响应。
- **极简**：只聚焦真正需要的核心功能，砍掉历史包袱。
- **跨平台**：Windows 10/11 (64位)、macOS、Linux 三端原生桌面应用，单可执行文件分发。
- **量产友好**：工厂产线场景一等公民，多口并发刷写、大色块成功/失败指示。

### 1.2 与 V2/V3 的关键对比

| 维度 | V2/V3 (历史版本) | V4 (本版本) |
|------|-----------------|-------------|
| UI 框架 | wxPython（多窗口弹窗模式） | Vue 3 + Tailwind CSS（单页应用 SPA） |
| 后端语言 | Python 3 | Rust + Tauri 2 |
| 分发方式 | Python 解释器 + 大量依赖包，安装包数十 MB | Tauri 单体可执行文件，体积极小 |
| 支持芯片 | EC618/EC718、ESP32、MH1903(Air105)、W800(Air101/103)、RDA8910(Air724UG) 等大量平台 | **仅** EC618/EC718（Air780E/EP/700E 系列）和 Beken (BK) |
| Windows 最低版本 | Windows 7 (含补丁包) | Windows 10/11 64位（丢弃 Win7） |
| WebView | 内嵌独立 WebView2 运行时（几十 MB） | 系统自带 WebView2，零额外体积 |
| 云端耦合 | 设备绑定、云平台账号强耦合 | 仅保留可选的"拉取官方最新固件列表"薄层 |
| 串口配置 | 手动选择串口号/波特率 | 全自动热插拔识别，零配置即插即用 |

---

## 2. V4 核心精简：断舍离决策

### 2.1 彻底废弃的芯片平台

V4 **只支持**以下两个平台，其余一律移除：

| 平台 | 对应芯片 | 代表模组 |
|------|---------|---------|
| **EC618 / EC718** | 移芯（Eigencomm）EC618、EC718 | Air780E、Air780EP、Air700E 等 |
| **Beken (BK)** | BK7231N、BK7231T 等 | 合宙 BK 系列 Wi-Fi 模组 |

废弃的平台包括（不再编写/维护任何协议代码）：

- RDA8910（Air724UG 等旧 4G Cat.1 模组）—— 原厂支持弱，协议复杂
- RDA8955 / MTK 早期 2G 模块（Air202/208）
- MH1903 / 芯海（Air105）
- W600 / W800（联盛德，Air101/Air103）
- ESP32 系列（AirC3 等）—— ESP-IDF 生态已足够完善，无需集成

### 2.2 废弃的功能模块

- **老旧 .cpio 文件系统打包**（`cpiogen.py`）—— 替换为现代 `luadb` 格式
- **多窗口/弹窗式 UI**（wxPython 遗留）—— 替换为 SPA 侧边栏导航
- **32 位 Windows 补丁包**（`Windows6.1-KB2999226-x86.msu` 等）
- **手动串口配置**（波特率/奇偶校验下拉框）
- **设备云绑定逻辑**（`iotbind_downloader.py` 等重耦合云端模块）
- **复杂的管脚映射 SVG 生成**等边缘功能

---

## 3. 技术栈选择

### 3.1 总体分层

```
┌────────────────────────────────────────────────────────────┐
│                     前端 (Renderer Process)                 │
│         Vue 3 + TypeScript + Tailwind CSS + Vite            │
│   FlashView / LogView / FactoryView / SettingsView (SPA)    │
└─────────────────────┬──────────────────────────────────────┘
                      │  Tauri IPC (invoke / emit)
┌─────────────────────▼──────────────────────────────────────┐
│                    后端 (Main Process)                       │
│                Tauri 2 + Rust (src-tauri/)                  │
│  Tauri Commands │ serial:: │ flash:: │ luadb::              │
└─────────────────────────────────────────────────────────────┘
                      │  依赖
┌─────────────────────▼──────────────────────────────────────┐
│              底层 Rust 库 / 系统 API                         │
│  serialport crate │ yuzhan-tech/luatos-tools (参考/集成)    │
│  tokio (异步) │ anyhow (错误处理) │ serde (序列化)           │
└─────────────────────────────────────────────────────────────┘
```

### 3.2 各层职责

| 层 | 技术 | 职责 |
|----|------|------|
| **前端 UI** | Vue 3 + Tailwind | 用户交互、进度展示、日志渲染、文件拖拽 |
| **Tauri 胶水层** | Tauri 2 + Rust | IPC 命令注册、进度事件 emit、线程管理 |
| **serial 模块** | Rust + `serialport` crate | 端口枚举、热插拔监听、字节流读写缓冲 |
| **flash 模块** | Rust | 芯片 ISP 握手、擦写、校验、复位的 trait 抽象与实现 |
| **luadb 模块** | Rust（+ Lua 5.3 FFI） | Lua 脚本交叉编译（.lua → .luac）、luadb 文件系统打包 |

### 3.3 选型理由

- **Tauri 2**：无需打包 Node/Python/WebView 运行时，Windows 上利用系统 WebView2，macOS 用 WKWebView，体积极小（< 5 MB）。
- **Vue 3 + TypeScript**：响应式 + 组合式 API，适合状态频繁变化的工具类应用（串口日志实时刷新、多端口并发量产）。
- **Tailwind CSS**：原子化样式，无需维护额外 CSS 文件，深色主题开箱即用。
- **Rust**：内存安全、零成本抽象，串口通信和文件系统打包的性能上限高；与 Tauri 原生集成，无 FFI 开销。

---

## 4. 目录结构

```
luatools_v4/
├── index.html                  # Vite 入口 HTML
├── package.json                # npm 脚本 & 前端依赖
├── vite.config.ts              # Vite 构建配置
├── tailwind.config.js          # Tailwind CSS 主题配置
├── tsconfig.json
├── src/                        # Vue 3 前端源码
│   ├── main.ts                 # 应用入口，挂载 Vue 实例
│   ├── App.vue                 # 根组件：侧边栏 + 路由出口 + 底部状态栏
│   ├── style.css               # 全局样式（Tailwind 指令）
│   └── views/
│       ├── FlashView.vue       # 下载/刷机界面
│       ├── LogView.vue         # 实时日志查看器
│       ├── FactoryView.vue     # 量产模式大色块 Dashboard
│       └── SettingsView.vue    # 设置页
├── src-tauri/                  # Tauri + Rust 后端
│   ├── Cargo.toml              # Rust 依赖声明
│   ├── build.rs                # Tauri 构建脚本
│   ├── tauri.conf.json         # Tauri 应用配置（权限、窗口等）
│   └── src/
│       ├── main.rs             # 程序入口，注册 Tauri Commands
│       ├── serial/
│       │   └── mod.rs          # 端口枚举（list_ports）、SerialBuffer
│       ├── flash/
│       │   ├── mod.rs          # Flasher trait + FlashProgress 定义
│       │   ├── ec618.rs        # EC618/EC718 ISP 实现（FDL 协议）
│       │   └── bk.rs           # Beken ISP 实现
│       └── luadb/
│           └── mod.rs          # luadb 文件系统打包器
└── docs/
    └── ARCHITECTURE.zh-CN.md  # 本文档
```

---

## 5. 前端架构设计

### 5.1 布局设计

采用类 VS Code 的三区布局：

```
┌──────────────────────────────────────────────────────────┐
│  LuaTools v4         [最小化] [最大化] [关闭]             │
├─────────┬────────────────────────────────────────────────┤
│         │                                                 │
│ 侧边栏  │              主工作区                           │
│         │   （由 Vue Router 切换各 View）                  │
│ 🛠 刷机 │                                                 │
│ 📝 日志 │                                                 │
│ 🏭 量产 │                                                 │
│ ⚙ 设置  │                                                 │
│         │                                                 │
├─────────┴────────────────────────────────────────────────┤
│ 底部状态栏：● COM3 — Air780E [1a86:7523]  v4.0.0          │
└──────────────────────────────────────────────────────────┘
```

### 5.2 高性能实时日志

实时日志（Log Viewer）是工具最高频的使用场景，设计要点：

- **虚拟列表渲染**：日志行数可达数万行，必须使用虚拟滚动（如 `vue-virtual-scroller` 或 Canvas 自绘），避免 DOM 节点膨胀导致卡顿。
- **环形缓冲区**：Rust 端 `SerialBuffer` 按 4 KiB 块积累原始字节，通过 Tauri `emit` 事件批量推送到前端，避免频繁 IPC。
- **ANSI 颜色支持**：解析常见 ANSI 转义序列，以彩色区分 INFO / WARN / ERROR 等级别。
- **关键词过滤**：前端提供实时正则过滤输入框，不影响底层数据流。
- **自动跟随 / 手动锁定**：滚动到底部时自动跟随新行；用户向上滚动时自动锁定，不再强制跳底。

### 5.3 大文件拖拽体验

刷机界面（Flash View）支持两种文件拖入方式：

1. **固件文件**（`.bin` / `.soc` / `.pac`）：拖入"Core Firmware"区域，自动填写路径并校验文件头魔数。
2. **脚本文件夹**：拖入"Script / File System"区域，前端读取目录树，调用后端 `luadb::pack_directory` 打包，完成后自动关联到刷机流程。

设计细节：
- 拖入时区域高亮（青色边框发光效果），提供明确的视觉反馈。
- 对超大固件（> 10 MB）文件，进度显示分为"读取 → 传输 → 写入"三阶段，避免界面假死感。
- 支持多文件同时拖入，工具自动按扩展名判断是固件还是脚本资源。

### 5.4 量产模式大色块 Dashboard

量产模式（Factory View）专为工厂产线设计：

- 每个串口对应一个**大色块卡片**（占 1/4 或 1/8 屏幕），显示端口号和设备名。
- 状态颜色：**青色**（就绪）→ **蓝色**（刷写中，显示百分比）→ **绿色**（成功）/ **红色**（失败，附错误原因）。
- 支持 1 拖 4 / 1 拖 8 USB Hub 的多口并发刷写，进度互相独立。
- 适合工厂大妈远距离查看，字体超大，颜色高对比。

---

## 6. 后端刷写引擎架构

### 6.1 Flasher Trait 分层

```rust
// flash/mod.rs — 通用接口，所有芯片实现此 trait
pub trait Flasher {
    fn connect(&mut self) -> Result<()>;   // 打开串口，握手进入 ISP 模式
    fn erase(&mut self)   -> Result<()>;   // 擦除目标 Flash 区域
    fn write(&mut self, data: &[u8], on_progress: &ProgressCallback) -> Result<()>;
    fn verify(&mut self, data: &[u8]) -> Result<()>;  // 校验写入内容
    fn reset(&mut self)   -> Result<()>;   // 复位，退出 ISP，启动新固件
}
```

调用方（Tauri Command）只持有 `Box<dyn Flasher>`，完全不感知芯片差异：

```rust
// 示例：Tauri 刷机命令（未来实现）
#[tauri::command]
async fn start_flash(port: String, firmware: Vec<u8>) -> Result<(), String> {
    let flasher: Box<dyn Flasher> = detect_chip_and_create_flasher(&port)?;
    flasher.connect()?;
    flasher.erase()?;
    flasher.write(&firmware, &|p| emit_progress(p))?;
    flasher.verify(&firmware)?;
    flasher.reset()?;
    Ok(())
}
```

### 6.2 EC618/EC718 刷写流程（FDL 协议）

EC618/EC718（移芯平台）使用 UART FDL（Firmware Download）协议，流程如下：

```
主机                                    芯片 ROM Boot
  │                                          │
  │─── 拉低 DTR/RTS，触发复位 ──────────────►│
  │◄── 等待 0xAD 同步字节 ──────────────────│
  │─── 发送握手包（魔数 + 版本） ───────────►│
  │◄── ACK ─────────────────────────────────│
  │─── 协商波特率（切换至 921600） ─────────►│
  │◄── ACK ─────────────────────────────────│
  │─── 发送擦除命令（地址 + 长度） ─────────►│
  │◄── ACK ─────────────────────────────────│
  │─── 分片发送数据包（512B 每片） ──────────►│
  │◄── 逐片 ACK ────────────────────────────│
  │─── 发送 CRC 校验命令 ───────────────────►│
  │◄── 校验结果 ────────────────────────────│
  │─── 发送复位命令 ───────────────────────►│
```

> 详细字节序和魔数参见 `yuzhan-tech/luatos-tools` 中 `src/flash/` 目录的实现。

### 6.3 Beken (BK) 刷写流程

Beken 芯片使用私有 UART ISP 协议，流程类似但握手字节不同：

- 固定波特率 921600，发送 `0x01` 同步字节等待 `0xFE` 响应。
- 擦除、写入命令格式参见 BK7231 datasheet 或 `yuzhan-tech/luatos-tools` 对应实现。

### 6.4 luadb 文件系统打包

`luadb` 是 LuatOS 使用的脚本文件系统格式（基于 LittleFS 变种），打包流程：

1. 扫描输入目录树，收集所有 `.lua` 和资源文件。
2. 对每个 `.lua` 文件调用 Lua 5.3 编译器（通过 Rust FFI 调用 `csrc/` 中的 C 库），生成 `.luac` 字节码。
3. 按 luadb 头部格式（文件数量、各文件元数据、数据区）序列化为二进制 blob。
4. 输出 `.bin` 文件，供刷写流程使用。

> 参考实现：`yuzhan-tech/luatos-tools` 中 `src/luadb/` 目录。

---

## 7. 核心外部依赖：yuzhan-tech/luatos-tools

[yuzhan-tech/luatos-tools](https://github.com/yuzhan-tech/luatos-tools) 是一个用 Rust 实现的 LuatOS 工具库，包含：

| 子模块 | 功能 | 在 v4 中的用途 |
|--------|------|---------------|
| `src/flash/` | EC618/EC718 FDL 刷写协议 | 直接参考/移植至 `src-tauri/src/flash/ec618.rs` |
| `src/luadb/` | luadb 文件系统打包 | 直接参考/移植至 `src-tauri/src/luadb/mod.rs` |
| `src/serial/` | 串口通信封装 | 参考，v4 基于 `serialport` crate 自行实现 |
| `agentboot/` | 模组引导代理（辅助进入 ISP 模式） | 按需集成 |
| `csrc/` + `lua-5.3.6/` | Lua 5.3 C 源码（用于 Rust FFI） | 集成进 luadb 打包模块 |

### 引入方式（Cargo.toml）

```toml
# 方式 1：若已发布到 crates.io
luatos-tools = "0.x"

# 方式 2：直接引用 Git 仓库（推荐）
luatos-tools = { git = "https://github.com/yuzhan-tech/luatos-tools" }
```

> 注意：在正式引入之前，需确认该库的 License 与本项目兼容，并检查是否存在已知安全漏洞。

---

## 8. 杂项架构决策

### 8.1 放弃 Windows 7 兼容性

- Tauri 2 依赖系统 WebView2，最低要求 Windows 10 1803（2018年4月更新）。
- 老版工具包含的 `Windows6.1-KB2999226-x86.msu` 等补丁文件在 v4 中**彻底删除**。
- 不再提供 32 位安装包，统一 64 位分发。

### 8.2 云端功能薄化

V4 初期定位为**纯本地工具**，云端功能极简化：

| 功能 | V3 | V4 |
|------|----|----|
| 设备绑定账号 | 强耦合 | **移除** |
| 在线拉取固件列表 | 有 | 保留（薄层，仅 HTTP GET JSON） |
| 自动更新脚本 | 有 | **移除** |
| 量产报表上传 | 有 | **移除**（产线离线化） |

### 8.3 串口自动识别策略

无需用户手动选择串口，工具按以下优先级自动匹配：

1. 检查 USB VID/PID 是否在合宙已知设备列表（EC618: VID=`1782`, BK: VID 待确认）。
2. 过滤名称含 `CH340`、`CP210x`、`FTDI` 等常见 USB 转串口芯片关键词。
3. 多个匹配时，下拉框列出所有候选，用户选择或保持"Auto detect"。

### 8.4 进度事件推送机制

Tauri `emit` 替代轮询，将进度状态从 Rust 后端实时推送到 Vue 前端：

```rust
// Rust 端
app_handle.emit("flash_progress", FlashProgress { stage, written, total }).unwrap();
```

```typescript
// Vue 端
import { listen } from '@tauri-apps/api/event'
await listen<FlashProgress>('flash_progress', (event) => {
  progress.value = event.payload
})
```

### 8.5 错误处理原则

- Rust 端统一用 `anyhow::Result`，错误链完整保留。
- 通过 Tauri `Result<T, String>` 类型将错误序列化后传给前端。
- 前端所有 `invoke` 调用均包裹 `try/catch`，错误信息显示在对应面板的黄色提示条中，而非弹窗（SPA 原则）。

---

## 9. 分阶段重构计划

### Phase 1 — 骨架搭建（已完成）

- [x] Vue 3 + Tailwind CSS 前端骨架（深色主题、四视图 SPA）
- [x] Tauri 2 后端初始化，注册 `get_serial_ports` 命令
- [x] `flash/` 模块 Trait 定义 + EC618/BK stub 实现
- [x] `luadb/` 模块骨架
- [x] `serial/` 模块端口枚举与 `SerialBuffer`
- [x] README 串口调试指南

### Phase 2 — 核心功能实现（进行中）

- [ ] 移植/集成 `yuzhan-tech/luatos-tools` 的 EC618 FDL 刷写协议
- [ ] 移植/集成 BK ISP 刷写协议
- [ ] 实现 `luadb::pack_directory`（含 Lua 5.3 FFI 编译）
- [ ] 实现固件文件拖拽（前端）+ 大文件分块读取（Rust）
- [ ] 实现刷写进度 Tauri emit 事件链路
- [ ] 实现串口日志实时推流（虚拟列表 Log View）

### Phase 3 — 量产模式与体验打磨

- [ ] 量产模式大色块 Dashboard（多口并发，绿/红指示）
- [ ] USB 热插拔监听（设备插入自动刷新端口列表）
- [ ] ANSI 颜色日志支持
- [ ] 可选：在线拉取官方最新固件列表（薄层云端）
- [ ] 打包签名与自动更新（Tauri updater）

### Phase 4 — 稳定化与发布

- [ ] Windows / macOS / Linux 三平台冒烟测试
- [ ] 量产场景压力测试（1拖8 并发 100 台）
- [ ] 完善错误信息本地化（中文）
- [ ] 发布 v4.0.0 正式版

---

## 10. PR 协作建议

### 分支命名约定

```
feature/<简短描述>      # 新功能
fix/<简短描述>          # Bug 修复
refactor/<模块名>       # 重构
docs/<文档描述>         # 文档更新
```

### 提交信息规范（Conventional Commits）

```
feat(flash): implement EC618 FDL connect handshake
fix(serial): handle empty port list on macOS Ventura
refactor(luadb): extract Lua compiler to separate module
docs: add architecture document
```

### PR 检查清单

- [ ] Rust 代码通过 `cargo clippy` 无 warning
- [ ] 前端代码通过 `npm run build`（vue-tsc 类型检查）
- [ ] 新增功能包含对应的 stub/TODO 或实际测试
- [ ] 涉及 UI 变更时附截图
- [ ] 引入新的 Rust crate 前先查询 [GitHub Advisory Database](https://github.com/advisories) 确认无已知漏洞

### 本地开发环境启动

```bash
# 安装前端依赖
npm install

# 启动开发模式（Vite dev server + Tauri 窗口热重载）
npm run tauri:dev

# 仅构建前端（用于检查 TypeScript 错误）
npm run build

# 生产构建
npm run tauri:build
```

---

## 11. FAQ

**Q: 为什么不用 Electron 而用 Tauri？**

Electron 会打包一个完整的 Chromium + Node.js，安装包通常 > 100 MB。Tauri 利用系统自带 WebView（Windows WebView2、macOS WKWebView），可执行文件体积 < 5 MB，内存占用也更低，非常适合嵌入式开发工具的分发场景。

**Q: 为什么不继续用 Python？**

V3 的 Python 方案存在两个核心痛点：①分发困难（需要打包解释器或依赖 PyInstaller，体积大、启动慢）；②串口通信和底层协议实现在 Python 中难以达到高并发量产所需的可靠性。Rust 从根本上解决了这两个问题。

**Q: EC618 和 EC718 有什么区别？是否需要两套协议？**

EC618 和 EC718 使用相同的 FDL 下载协议，硬件接口完全兼容，仅在功能上（Cat.1 vs Cat.1 bis）和内部外设有差异。V4 中两者共用 `ec618.rs` 的同一套实现，无需分开。

**Q: 为什么不支持 ESP32？**

ESP32 有完善的官方工具链（`esptool.py`、ESP-IDF）和大量社区工具，已经不需要 LuaTools 来填补这个空缺。专注 EC618/EC718 + BK 可以把有限的精力用在刀刃上。

**Q: 如何在 Windows 上调试串口枚举问题？**

参见 [README - Debugging Serial-Port Enumeration](../README.md) 章节，提供了完整的逐步排查指南，包括启用 Rust 日志、检查 DevTools、验证 OS 驱动等步骤。

**Q: luadb 格式和 spiffs/fatfs 有什么区别？**

luadb 是合宙为 LuatOS 定制的轻量文件系统镜像格式，基于 LittleFS 的变种，专门针对小 Flash（256KB - 2MB）优化，支持 Lua 字节码直接映射执行。V4 只支持 luadb，不再支持旧的 cpio/spiffs 格式。

---

> 文档维护者：LuaTools v4 团队  
> 欢迎通过 PR 补充或修正本文档。
