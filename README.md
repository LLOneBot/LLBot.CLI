# LLBot CLI

LLBot 命令行启动器

## 功能

- 两种启动模式，默认 **headless**（与 Desktop 默认一致）：
  - **headless（默认）**：不启动 PMHQ，直接拉起 LLBot，由 LLBot 直连 QQ 协议
  - **PMHQ 模式**（加 `--pmhq` / `--no-headless`）：启动 PMHQ attach QQ，LLBot 经 PMHQ 通信
- 登录二维码由 LLBot 自身打印到终端
- 登录成功后通过命名管道 (IPC) 从 LLBot 获取 QQ 号 / 昵称，并设置窗口标题（仅 Windows）
- 进程生命周期管理（Ctrl+C 自动清理）

## 目录结构

```
llbot(.exe)
bin/
  llbot/                 # 必需 (headless / PMHQ 模式都用)
    node(.exe)
    llbot.js
  pmhq/                  # 仅 PMHQ 模式 (--pmhq) 需要; headless 不下载/不使用
    pmhq(.exe)
    pmhq_config.json
```

> 组件 (PMHQ / Node.js / LLBot) 缺失时会在启动时自动下载；headless 模式不会下载 PMHQ。

## 命令行参数

所有参数都是可选的。CLI 自身识别 `--pmhq` / `--no-headless` 切换模式；其余参数：headless 模式下仅 `--qq=` 透传给 LLBot，PMHQ 模式下透传给 PMHQ。

| 参数 | 说明 |
|------|------|
| `--pmhq`, `--no-headless` | 切换为 PMHQ 模式（默认 headless 直连） |
| `--qq=<number>` | 快速登录 QQ 号（两种模式均生效） |
| `--update` | 检查并执行更新 |
| `--help, -h` | 显示帮助信息 |
| `--version, -v` | 显示版本信息 |

以下参数仅在 **PMHQ 模式** 下生效（透传给 PMHQ）：

| 参数 | 说明 |
|------|------|
| `--qq-path=<path>` | QQ 可执行文件路径 |
| `--qq-console` | 启用 QQ 控制台日志 |
| `--debug` | 调试模式 |
| `--debug-pb[=true/false]` | 显示 send/recv Protobuf 日志 |
| `--work-dir=<path>` | 工作目录 |
| `--no-exit-with-qq` | 禁用 QQ 退出时自动退出（默认启用） |
| `--qq-exit-delay=<秒>` | QQ 退出后的缓冲时间（默认 15 秒，0 为立即退出） |

## 使用示例

```bash
# 直接启动（headless 直连，默认）
./llbot

# 快速登录
./llbot --qq=123456789

# PMHQ 模式（启动 PMHQ + attach QQ）
./llbot --pmhq

# PMHQ 模式 + 指定 QQ 路径
./llbot --pmhq --qq-path="/opt/QQ/qq"

# 检查更新
./llbot --update
```

## 支持平台

- Windows x64
- Linux x64
- Linux arm64

> 登录信息 (uin / 昵称) 的命名管道 IPC 仅 Windows 生效，用于登录后回填并设置窗口标题；其它平台依赖 LLBot 自身的终端输出（二维码、登录状态）。
